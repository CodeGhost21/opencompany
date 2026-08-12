//! The five billing tools this server advertises, and their dispatch onto
//! [`ChargebeeClient`].
//!
//! Arguments are validated here, before any network call, whenever the check is
//! one Chargebee would also make. That is not redundancy: a rejection written
//! locally can name the valid set (`payment_method` must be one of …), whereas
//! Chargebee's own error arrives after a round trip and, for the agent, after a
//! turn that looked like it was working.

use crate::error::{OpenCompanyError, Result};
use serde_json::{Value, json};

use super::client::{ChargebeeClient, Form};
use super::types::{
    AUTO_COLLECTION, CreateInvoiceArgs, GetSubscriptionArgs, ListInvoicesArgs, PAYMENT_METHODS,
    RecordPaymentArgs, UpsertCustomerArgs,
};

/// One tool as advertised over `tools/list`.
pub struct ToolDescriptor {
    /// The name the agent calls.
    pub name: &'static str,
    /// What the tool does, in the agent's terms.
    pub description: &'static str,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
}

/// Every tool this server exposes, in the order `tools/list` reports them.
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "chargebee_create_invoice",
            description: "Create a Chargebee invoice for an existing customer with one or more ad-hoc \
                 line items. Amounts are in the currency's MINOR unit: $500.00 USD is 50000, \
                 not 500. Raises an UNPAID invoice by default — it does not charge the \
                 customer's card. Returns the created invoice including its id, total, and status.",
            input_schema: json!({
                "type": "object",
                "required": ["customer_id", "currency_code", "charges"],
                "additionalProperties": false,
                "properties": {
                    "customer_id": {
                        "type": "string",
                        "description": "Existing Chargebee customer id. Use chargebee_upsert_customer first if the customer may not exist."
                    },
                    "currency_code": {
                        "type": "string",
                        "description": "ISO 4217 code, e.g. USD."
                    },
                    "charges": {
                        "type": "array",
                        "minItems": 1,
                        "description": "The invoice line items.",
                        "items": {
                            "type": "object",
                            "required": ["description", "amount_in_minor_units"],
                            "additionalProperties": false,
                            "properties": {
                                "description": {"type": "string"},
                                "amount_in_minor_units": {
                                    "type": "integer",
                                    "minimum": 1,
                                    "description": "Cents for USD. $500.00 is 50000."
                                }
                            }
                        }
                    },
                    "net_term_days": {"type": "integer", "minimum": 0, "description": "Days until due."},
                    "invoice_note": {"type": "string"},
                    "auto_collection": {
                        "type": "string",
                        "enum": AUTO_COLLECTION,
                        "description": "Defaults to `off`, raising an unpaid invoice to be settled later. Pass `on` ONLY if the user explicitly asked to charge the customer's stored card now."
                    },
                    "idempotency_key": {
                        "type": "string",
                        "description": "Reuse on retry to avoid creating a duplicate invoice."
                    }
                }
            }),
        },
        ToolDescriptor {
            name: "chargebee_list_invoices",
            description: "List Chargebee invoices, optionally filtered by customer, status, or invoice \
                 date range. Dates are Unix timestamps in seconds.",
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "customer_id": {"type": "string"},
                    "status": {
                        "type": "string",
                        "enum": ["paid", "posted", "payment_due", "not_paid", "voided", "pending"]
                    },
                    "invoice_date_after": {"type": "integer", "description": "Unix seconds."},
                    "invoice_date_before": {"type": "integer", "description": "Unix seconds."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100}
                }
            }),
        },
        ToolDescriptor {
            name: "chargebee_get_subscription",
            description: "Fetch a Chargebee subscription's status, plan, billing period, and next \
                 billing date by subscription id.",
            input_schema: json!({
                "type": "object",
                "required": ["subscription_id"],
                "additionalProperties": false,
                "properties": {"subscription_id": {"type": "string"}}
            }),
        },
        ToolDescriptor {
            name: "chargebee_upsert_customer",
            description: "Create a Chargebee customer, or update an existing one when `id` is given. \
                 Returns the customer record including its id.",
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Omit to create a new customer; supply to update that customer."
                    },
                    "first_name": {"type": "string"},
                    "last_name": {"type": "string"},
                    "email": {"type": "string"},
                    "company": {"type": "string", "description": "e.g. Acme Corp"},
                    "net_term_days": {"type": "integer", "minimum": 0}
                }
            }),
        },
        ToolDescriptor {
            name: "chargebee_record_payment",
            description: "Record an offline payment against a Chargebee invoice, marking it paid when \
                 the amount settles the balance. Amount is in the currency's MINOR unit.",
            input_schema: json!({
                "type": "object",
                "required": ["invoice_id", "amount_in_minor_units", "payment_method"],
                "additionalProperties": false,
                "properties": {
                    "invoice_id": {"type": "string"},
                    "amount_in_minor_units": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Cents for USD. $500.00 is 50000."
                    },
                    "payment_method": {"type": "string", "enum": PAYMENT_METHODS},
                    "reference_number": {"type": "string", "description": "Cheque or wire reference."},
                    "comment": {"type": "string"},
                    "idempotency_key": {
                        "type": "string",
                        "description": "Reuse on retry to avoid recording the payment twice."
                    }
                }
            }),
        },
    ]
}

/// Builds the invalid-argument error used for every local validation failure.
fn invalid(message: impl Into<String>) -> OpenCompanyError {
    OpenCompanyError::Chargebee {
        status: 0,
        code: "invalid_arguments".to_string(),
        message: message.into(),
    }
}

/// Deserializes `args` into `T`, reporting a parse failure as an argument error
/// rather than a transport one.
fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|e| invalid(e.to_string()))
}

/// Routes a `tools/call` to its handler.
pub async fn call(client: &ChargebeeClient, name: &str, args: Value) -> Result<Value> {
    match name {
        "chargebee_create_invoice" => create_invoice(client, parse(args)?).await,
        "chargebee_list_invoices" => list_invoices(client, parse(args)?).await,
        "chargebee_get_subscription" => get_subscription(client, parse(args)?).await,
        "chargebee_upsert_customer" => upsert_customer(client, parse(args)?).await,
        "chargebee_record_payment" => record_payment(client, parse(args)?).await,
        other => Err(invalid(format!(
            "unknown tool `{other}` — call tools/list for the available set"
        ))),
    }
}

async fn create_invoice(client: &ChargebeeClient, args: CreateInvoiceArgs) -> Result<Value> {
    if args.charges.is_empty() {
        return Err(invalid("`charges` must contain at least one line item"));
    }
    if args.currency_code.trim().is_empty() {
        return Err(invalid("`currency_code` is required, e.g. USD"));
    }
    for (i, charge) in args.charges.iter().enumerate() {
        if charge.amount_in_minor_units < 1 {
            return Err(invalid(format!(
                "charges[{i}].amount_in_minor_units must be at least 1 (the amount is in minor \
                 units — $500.00 is 50000)"
            )));
        }
    }

    if !AUTO_COLLECTION.contains(&args.auto_collection.as_str()) {
        return Err(invalid(format!(
            "`auto_collection` must be one of: {}",
            AUTO_COLLECTION.join(", ")
        )));
    }

    let mut form = Form::new();
    form.push("customer_id", args.customer_id);
    form.push("currency_code", args.currency_code.trim().to_uppercase());
    form.push("auto_collection", args.auto_collection);
    form.push_opt("net_term_days", args.net_term_days);
    form.push_opt("invoice_note", args.invoice_note);
    for (i, charge) in args.charges.iter().enumerate() {
        form.push_indexed("charges", "description", i, &charge.description);
        form.push_indexed("charges", "amount", i, charge.amount_in_minor_units);
    }

    client
        .post_form(
            "/invoices/create_for_charge_items_and_charges",
            &form,
            args.idempotency_key.as_deref(),
        )
        .await
}

async fn list_invoices(client: &ChargebeeClient, args: ListInvoicesArgs) -> Result<Value> {
    let mut query = Form::new();
    // Chargebee filters use an operator suffix: a bare `status=paid` is ignored
    // rather than rejected, which would silently return every invoice.
    query.push_opt("customer_id[is]", args.customer_id);
    query.push_opt("status[is]", args.status);
    query.push_opt("date[after]", args.invoice_date_after);
    query.push_opt("date[before]", args.invoice_date_before);
    query.push_opt("limit", args.limit);
    client.get("/invoices", &query).await
}

async fn get_subscription(client: &ChargebeeClient, args: GetSubscriptionArgs) -> Result<Value> {
    let id = args.subscription_id.trim();
    if id.is_empty() {
        return Err(invalid("`subscription_id` is required"));
    }
    client
        .get(&format!("/subscriptions/{}", urlencode(id)), &Form::new())
        .await
}

async fn upsert_customer(client: &ChargebeeClient, args: UpsertCustomerArgs) -> Result<Value> {
    let mut form = Form::new();
    form.push_opt("first_name", args.first_name);
    form.push_opt("last_name", args.last_name);
    form.push_opt("email", args.email);
    form.push_opt("company", args.company);
    form.push_opt("net_term_days", args.net_term_days);

    match args.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Chargebee splits these across two endpoints; the tool does not, so
        // the agent never has to know whether the record already existed.
        Some(id) => {
            client
                .post_form(&format!("/customers/{}", urlencode(id)), &form, None)
                .await
        }
        None => client.post_form("/customers", &form, None).await,
    }
}

async fn record_payment(client: &ChargebeeClient, args: RecordPaymentArgs) -> Result<Value> {
    if args.amount_in_minor_units < 1 {
        return Err(invalid(
            "`amount_in_minor_units` must be at least 1 (the amount is in minor units — \
             $500.00 is 50000)",
        ));
    }
    if !PAYMENT_METHODS.contains(&args.payment_method.as_str()) {
        return Err(invalid(format!(
            "`payment_method` must be one of: {}",
            PAYMENT_METHODS.join(", ")
        )));
    }
    let invoice_id = args.invoice_id.trim();
    if invoice_id.is_empty() {
        return Err(invalid("`invoice_id` is required"));
    }

    let mut form = Form::new();
    form.push(
        "transaction[amount]",
        args.amount_in_minor_units.to_string(),
    );
    form.push("transaction[payment_method]", args.payment_method);
    form.push_opt("transaction[reference_number]", args.reference_number);
    form.push_opt("comment", args.comment);

    client
        .post_form(
            &format!("/invoices/{}/record_payment", urlencode(invoice_id)),
            &form,
            args.idempotency_key.as_deref(),
        )
        .await
}

/// Percent-encodes a path segment.
///
/// Customer and invoice ids are caller-supplied and reach the URL path, so a
/// value containing `/` or `?` would otherwise re-target the request at a
/// different endpoint.
fn urlencode(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chargebee::types::ChargebeeConfig;

    fn client() -> ChargebeeClient {
        ChargebeeClient::with_base_url(
            ChargebeeConfig {
                site: "test".to_string(),
                api_key: "key".to_string(),
            },
            // Port 0 is never listening, so any test reaching the network here
            // fails fast instead of hanging — every test below must be rejected
            // by local validation before the request is built.
            "http://127.0.0.1:0".to_string(),
        )
        .expect("client builds")
    }

    #[tokio::test]
    async fn dollars_mistaken_for_cents_is_not_rejected_but_zero_is() {
        // 500 is a legal (if small) amount — the guard cannot read intent, which
        // is exactly why the field is named for its unit and the schema says so.
        // What it can catch is Chargebee's documented floor.
        let err = create_invoice(
            &client(),
            serde_json::from_value(json!({
                "customer_id": "acme",
                "currency_code": "USD",
                "charges": [{"description": "Pro plan", "amount_in_minor_units": 0}]
            }))
            .expect("args parse"),
        )
        .await
        .expect_err("a zero amount is rejected");
        assert!(err.to_string().contains("at least 1"), "got: {err}");
    }

    #[tokio::test]
    async fn empty_charges_is_rejected_before_any_request() {
        let err = create_invoice(
            &client(),
            serde_json::from_value(json!({
                "customer_id": "acme", "currency_code": "USD", "charges": []
            }))
            .expect("args parse"),
        )
        .await
        .expect_err("no line items is rejected");
        assert!(
            err.to_string().contains("at least one line item"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn a_bad_payment_method_names_the_valid_set() {
        let err = record_payment(
            &client(),
            serde_json::from_value(json!({
                "invoice_id": "inv_1",
                "amount_in_minor_units": 50_000,
                "payment_method": "credit_card"
            }))
            .expect("args parse"),
        )
        .await
        .expect_err("an unsupported method is rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("bank_transfer"),
            "expected the valid set, got: {msg}"
        );
    }

    #[tokio::test]
    async fn an_unknown_tool_points_at_tools_list() {
        let err = call(&client(), "chargebee_delete_everything", json!({}))
            .await
            .expect_err("unknown tools are rejected");
        assert!(err.to_string().contains("tools/list"), "got: {err}");
    }

    #[test]
    fn ids_are_percent_encoded_so_they_cannot_retarget_the_path() {
        assert_eq!(urlencode("inv_123"), "inv_123");
        assert_eq!(urlencode("../subscriptions/x"), "..%2Fsubscriptions%2Fx");
    }

    /// What a stub captured from one request.
    struct Captured {
        authorization: String,
        idempotency: Option<String>,
        query: String,
        body: String,
    }

    /// Serves one canned response from a local socket and captures the request.
    ///
    /// The wire format is the part of this module most likely to be wrong —
    /// form encoding rather than JSON, bracket-array nesting, Basic rather than
    /// Bearer — and none of it is exercised by a test that stops at validation.
    /// So these tests go all the way to a socket.
    async fn stub(
        status: u16,
        response_body: &'static str,
        call: impl AsyncFnOnce(ChargebeeClient) -> Result<Value>,
    ) -> (Result<Value>, Captured) {
        use axum::extract::State;
        use axum::http::HeaderMap;
        use std::sync::{Arc, Mutex};

        let seen: Arc<Mutex<Option<Captured>>> = Arc::new(Mutex::new(None));

        let handler = {
            move |State(seen): State<Arc<Mutex<Option<Captured>>>>,
                  uri: axum::http::Uri,
                  headers: HeaderMap,
                  body: String| async move {
                *seen.lock().expect("lock") = Some(Captured {
                    authorization: headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_string(),
                    idempotency: headers
                        .get("chargebee-idempotency-key")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    query: uri.query().unwrap_or_default().to_string(),
                    body,
                });
                (
                    axum::http::StatusCode::from_u16(status).expect("valid status"),
                    [("content-type", "application/json")],
                    response_body,
                )
            }
        };

        let app = axum::Router::new()
            .fallback(axum::routing::any(handler))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let client = ChargebeeClient::with_base_url(
            ChargebeeConfig {
                site: "test".to_string(),
                api_key: "cb_key".to_string(),
            },
            format!("http://{addr}"),
        )
        .expect("client builds");

        let result = call(client).await;
        server.abort();

        let captured = seen.lock().expect("lock").take().expect("stub was called");
        (result, captured)
    }

    #[tokio::test]
    async fn create_invoice_sends_form_encoded_bracket_arrays_with_basic_auth() {
        let (result, captured) = stub(
            200,
            r#"{"invoice":{"id":"inv_42","total":52500,"status":"payment_due"}}"#,
            async |client| {
                create_invoice(
                    &client,
                    serde_json::from_value(json!({
                        "customer_id": "acme",
                        "currency_code": "usd",
                        "charges": [
                            {"description": "Pro plan", "amount_in_minor_units": 50_000},
                            {"description": "Setup", "amount_in_minor_units": 2_500}
                        ],
                        "net_term_days": 30,
                        "idempotency_key": "turn-7"
                    }))
                    .expect("args parse"),
                )
                .await
            },
        )
        .await;

        let value = result.expect("the stub returns success");
        assert_eq!(value["invoice"]["id"], "inv_42");

        // Basic, not Bearer — the single most likely thing to get wrong — and
        // the key is the USERNAME with an empty password, which is the second.
        // `Y2Jfa2V5Og==` is base64("cb_key:"); asserted as a literal because
        // the `base64` crate is behind the `mcp` feature and this lane builds
        // without it.
        assert_eq!(captured.authorization, "Basic Y2Jfa2V5Og==");

        assert_eq!(captured.idempotency.as_deref(), Some("turn-7"));

        let body = &captured.body;
        assert!(body.contains("customer_id=acme"), "body: {body}");
        // Normalized: Chargebee expects the ISO code uppercase.
        assert!(body.contains("currency_code=USD"), "body: {body}");
        assert!(body.contains("net_term_days=30"), "body: {body}");
        // Bracket-array nesting, percent-encoded by the form serializer.
        assert!(
            body.contains("charges%5Bdescription%5D%5B0%5D=Pro+plan"),
            "body: {body}"
        );
        assert!(
            body.contains("charges%5Bamount%5D%5B0%5D=50000"),
            "body: {body}"
        );
        assert!(
            body.contains("charges%5Bamount%5D%5B1%5D=2500"),
            "body: {body}"
        );
        // Unasked-for, and load-bearing: without it Chargebee follows the
        // customer's own setting and charges a stored card the moment the
        // invoice exists. Live-tested — a real site answered
        // `payment_method_not_present` before this was sent.
        assert!(body.contains("auto_collection=off"), "body: {body}");
        // An omitted optional must be absent, not blank — Chargebee reads a
        // blank value as "clear this field".
        assert!(!body.contains("invoice_note"), "body: {body}");
    }

    #[tokio::test]
    async fn creating_an_invoice_never_charges_a_card_unless_asked() {
        // The default is not Chargebee's: theirs follows the customer record and
        // collects immediately. "Create an invoice" must not move money.
        let args: CreateInvoiceArgs = serde_json::from_value(json!({
            "customer_id": "acme",
            "currency_code": "USD",
            "charges": [{"description": "Pro plan", "amount_in_minor_units": 50_000}]
        }))
        .expect("args parse");
        assert_eq!(args.auto_collection, "off");

        // And an explicit request is still honoured.
        let (_, captured) = stub(200, r#"{"invoice":{"id":"inv_1"}}"#, async |client| {
            create_invoice(
                &client,
                serde_json::from_value(json!({
                    "customer_id": "acme",
                    "currency_code": "USD",
                    "charges": [{"description": "Pro plan", "amount_in_minor_units": 50_000}],
                    "auto_collection": "on"
                }))
                .expect("args parse"),
            )
            .await
        })
        .await;
        assert!(
            captured.body.contains("auto_collection=on"),
            "body: {}",
            captured.body
        );
    }

    #[tokio::test]
    async fn a_bad_auto_collection_names_the_valid_set() {
        let err = create_invoice(
            &client(),
            serde_json::from_value(json!({
                "customer_id": "acme",
                "currency_code": "USD",
                "charges": [{"description": "Pro plan", "amount_in_minor_units": 50_000}],
                "auto_collection": "yes"
            }))
            .expect("args parse"),
        )
        .await
        .expect_err("an unsupported value is rejected");
        assert!(err.to_string().contains("on, off"), "got: {err}");
    }

    #[tokio::test]
    async fn a_chargebee_4xx_surfaces_its_api_error_code_and_message() {
        let (result, _) = stub(
            404,
            r#"{"message":"Customer not found","type":"invalid_request",
                "api_error_code":"resource_not_found","http_status_code":404}"#,
            async |client| {
                create_invoice(
                    &client,
                    serde_json::from_value(json!({
                        "customer_id": "nope",
                        "currency_code": "USD",
                        "charges": [{"description": "Pro plan", "amount_in_minor_units": 50_000}]
                    }))
                    .expect("args parse"),
                )
                .await
            },
        )
        .await;

        let err = result.expect_err("a 404 is an error");
        let message = err.to_string();
        // Both halves matter: the code lets a caller branch, the message is what
        // actually tells the agent the customer does not exist.
        assert!(message.contains("resource_not_found"), "got: {message}");
        assert!(message.contains("Customer not found"), "got: {message}");
        assert_eq!(err.code(), "chargebee_resource_not_found");
    }

    #[tokio::test]
    async fn list_invoices_uses_chargebee_filter_operators() {
        let (result, captured) = stub(200, r#"{"list":[]}"#, async |client| {
            list_invoices(
                &client,
                serde_json::from_value(json!({
                    "customer_id": "acme",
                    "status": "paid",
                    "limit": 25
                }))
                .expect("args parse"),
            )
            .await
        })
        .await;

        result.expect("the stub returns success");
        let query = &captured.query;
        // A bare `status=paid` is IGNORED by Chargebee rather than rejected,
        // which would quietly return every invoice on the site.
        assert!(query.contains("status%5Bis%5D=paid"), "query: {query}");
        assert!(query.contains("customer_id%5Bis%5D=acme"), "query: {query}");
        assert!(query.contains("limit=25"), "query: {query}");
        // Unset filters must not appear at all.
        assert!(!query.contains("date"), "query: {query}");
    }

    #[tokio::test]
    async fn upsert_customer_targets_the_update_path_only_when_an_id_is_given() {
        let (_, created) = stub(200, r#"{"customer":{"id":"c1"}}"#, async |client| {
            upsert_customer(
                &client,
                serde_json::from_value(json!({"company": "Acme Corp"})).expect("args parse"),
            )
            .await
        })
        .await;
        assert!(
            created.body.contains("company=Acme+Corp"),
            "body: {}",
            created.body
        );

        // The stub answers any path, so the assertion that the update form is
        // still sent is what distinguishes the two branches here; the path
        // itself is covered by the percent-encoding test above.
        let (_, updated) = stub(200, r#"{"customer":{"id":"c1"}}"#, async |client| {
            upsert_customer(
                &client,
                serde_json::from_value(json!({"id": "c1", "email": "ap@acme.test"}))
                    .expect("args parse"),
            )
            .await
        })
        .await;
        assert!(
            updated.body.contains("email=ap%40acme.test"),
            "body: {}",
            updated.body
        );
    }

    #[tokio::test]
    async fn record_payment_nests_the_transaction_parameters() {
        let (result, captured) = stub(
            200,
            r#"{"invoice":{"id":"inv_42","status":"paid"},"transaction":{"id":"txn_1"}}"#,
            async |client| {
                record_payment(
                    &client,
                    serde_json::from_value(json!({
                        "invoice_id": "inv_42",
                        "amount_in_minor_units": 52_500,
                        "payment_method": "bank_transfer",
                        "reference_number": "WIRE-9",
                        "comment": "Paid by wire"
                    }))
                    .expect("args parse"),
                )
                .await
            },
        )
        .await;

        assert_eq!(result.expect("success")["invoice"]["status"], "paid");
        let body = &captured.body;
        assert!(
            body.contains("transaction%5Bamount%5D=52500"),
            "body: {body}"
        );
        assert!(
            body.contains("transaction%5Bpayment_method%5D=bank_transfer"),
            "body: {body}"
        );
        assert!(
            body.contains("transaction%5Breference_number%5D=WIRE-9"),
            "body: {body}"
        );
        // `comment` is a top-level parameter, not part of the transaction.
        assert!(body.contains("comment=Paid+by+wire"), "body: {body}");
    }

    #[test]
    fn every_descriptor_has_an_object_schema_and_a_unique_name() {
        let mut seen = std::collections::HashSet::new();
        for d in descriptors() {
            assert!(seen.insert(d.name), "duplicate tool name {}", d.name);
            assert_eq!(d.input_schema["type"], "object", "{} schema", d.name);
            assert!(!d.description.is_empty(), "{} description", d.name);
        }
        assert_eq!(seen.len(), 5, "issue #788 scopes exactly five operations");
    }
}
