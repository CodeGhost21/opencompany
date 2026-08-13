//! The billing operations issue #788 scopes, expressed against Chargebee's
//! REST API v2 and returning the compact projections in [`super::types`].
//!
//! This layer knows Chargebee and nothing about agents. The toolbelt bridge in
//! [`crate::harness::chargebee`] wraps each function below as an agent-callable
//! tool; keeping the split means the API shapes can be tested without a harness,
//! and the tool descriptions can change without touching the wire format.
//!
//! Arguments are validated here, before any network call, whenever the check is
//! one Chargebee would also make. That is not redundancy: a local rejection can
//! name the valid set, whereas Chargebee's own error arrives after a round trip
//! and, for the agent, after a turn that looked like it was working.

use crate::error::{OpenCompanyError, Result};
use serde_json::Value;

use super::client::{ChargebeeClient, Form};
use super::types::{
    CreateCustomerArgs, CustomerSummary, GetInvoiceArgs, InvoiceSummary, ListInvoicesArgs,
    SendInvoiceArgs,
};

/// Builds the invalid-argument error used for every local validation failure.
pub(crate) fn invalid(message: impl Into<String>) -> OpenCompanyError {
    OpenCompanyError::Chargebee {
        status: 0,
        code: "invalid_arguments".to_string(),
        message: message.into(),
    }
}

/// Percent-encodes a path segment.
///
/// Customer and invoice ids reach the URL path and originate in agent input, so
/// a value containing `/` or `?` would otherwise re-target the request at a
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

/// Pulls a required object out of a Chargebee response.
///
/// Every write below reads one named object (`customer`, `invoice`) out of the
/// reply. Defaulting a missing one to `Null` and projecting it anyway yields a
/// record with an **empty id** that looks successful, and the next call spends
/// it — `customer_id=` on an invoice create, which Chargebee answers with a
/// confusing parameter error far from the real cause. So an absent object is an
/// error here, where it can name what was expected.
fn require<'a>(body: &'a Value, key: &str) -> Result<&'a Value> {
    body.get(key)
        .filter(|v| v.is_object())
        .ok_or_else(|| OpenCompanyError::Chargebee {
            status: 0,
            code: "unexpected_response".to_string(),
            message: format!(
                "Chargebee's reply carried no `{key}` object — got: {}",
                body.to_string().chars().take(200).collect::<String>()
            ),
        })
}

/// Projects Chargebee's invoice object onto [`InvoiceSummary`].
fn summarize_invoice(invoice: &Value, payment_url: Option<String>) -> InvoiceSummary {
    let num = |key: &str| invoice.get(key).and_then(Value::as_i64).unwrap_or(0);
    InvoiceSummary {
        id: invoice
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        customer_id: invoice
            .get("customer_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: invoice
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        currency_code: invoice
            .get("currency_code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        total_in_minor_units: num("total"),
        amount_due_in_minor_units: num("amount_due"),
        amount_paid_in_minor_units: num("amount_paid"),
        due_date: invoice.get("due_date").and_then(Value::as_i64),
        line_items: invoice
            .get("line_items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|li| li.get("description").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        payment_url,
    }
}

/// Projects Chargebee's customer object onto [`CustomerSummary`].
fn summarize_customer(customer: &Value) -> CustomerSummary {
    let text = |key: &str| {
        customer
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let name = match (text("first_name"), text("last_name")) {
        (Some(first), Some(last)) => Some(format!("{first} {last}")),
        (Some(one), None) | (None, Some(one)) => Some(one),
        (None, None) => None,
    };
    CustomerSummary {
        id: customer
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        email: text("email"),
        name,
        company: text("company"),
    }
}

/// Looks a customer up by email, returning `None` when no record matches.
pub async fn get_customer(
    client: &ChargebeeClient,
    email: &str,
) -> Result<Option<CustomerSummary>> {
    let email = email.trim();
    if email.is_empty() {
        return Err(invalid("`email` is required"));
    }
    let mut query = Form::new();
    // Chargebee filters take an operator suffix: a bare `email=` is IGNORED
    // rather than rejected, which would return an unrelated customer as if it
    // were a match — the worst possible failure for a tool that decides whether
    // to create one.
    query.push("email[is]", email);
    query.push("limit", "1");
    let body = client.get("/customers", &query).await?;
    Ok(body
        .get("list")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("customer"))
        .map(summarize_customer))
}

/// Creates a customer.
pub async fn create_customer(
    client: &ChargebeeClient,
    args: CreateCustomerArgs,
) -> Result<CustomerSummary> {
    let email = args.email.trim();
    if email.is_empty() {
        return Err(invalid("`email` is required"));
    }
    let mut form = Form::new();
    form.push("email", email);
    if let Some(name) = args
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Chargebee has no single `name` field. Splitting on the first space
        // keeps "Alan" whole and "Ada Byron" correct; a middle name lands in
        // `last_name`, which is wrong in a way nobody is harmed by.
        match name.split_once(' ') {
            Some((first, last)) => {
                form.push("first_name", first);
                form.push("last_name", last);
            }
            None => form.push("first_name", name),
        }
    }
    form.push_opt("company", args.company);
    let body = client.post_form("/customers", &form, None).await?;
    Ok(summarize_customer(require(&body, "customer")?))
}

/// Returns the customer for `email`, creating one when no record matches.
async fn resolve_or_create_customer(
    client: &ChargebeeClient,
    email: &str,
    name: Option<String>,
) -> Result<CustomerSummary> {
    if let Some(found) = get_customer(client, email).await? {
        return Ok(found);
    }
    create_customer(
        client,
        CreateCustomerArgs {
            email: email.to_string(),
            name,
            company: None,
        },
    )
    .await
}

/// Raises a hosted page where the customer can settle what they owe.
///
/// Best-effort by design: this is a second call after the invoice already
/// exists, and a site without a configured gateway (or without the hosted-page
/// feature) refuses it. Failing the whole tool at that point would report "no
/// invoice" for an invoice that was in fact created — the worst answer
/// available. So a failure logs and yields `None`, and the caller says the
/// invoice was raised without a link.
async fn payment_url(
    client: &ChargebeeClient,
    customer_id: &str,
    currency: &str,
) -> Option<String> {
    let mut form = Form::new();
    form.push("customer[id]", customer_id);
    form.push("currency_code", currency);
    match client
        .post_form("/hosted_pages/collect_now", &form, None)
        .await
    {
        Ok(body) => body
            .get("hosted_page")
            .and_then(|p| p.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Err(e) => {
            tracing::warn!(%customer_id, error = %e, "[chargebee] could not raise a payment link");
            None
        }
    }
}

/// Creates an invoice for `customer_email`, creating the customer if needed,
/// and returns it with a payment link when one could be raised.
pub async fn send_invoice(
    client: &ChargebeeClient,
    args: SendInvoiceArgs,
) -> Result<InvoiceSummary> {
    if args.line_items.is_empty() {
        return Err(invalid("`line_items` must contain at least one entry"));
    }
    let currency = args.currency_code.trim().to_uppercase();
    if currency.is_empty() {
        return Err(invalid("`currency_code` is required, e.g. USD"));
    }
    for (i, line) in args.line_items.iter().enumerate() {
        if line.amount_in_minor_units < 1 {
            return Err(invalid(format!(
                "line_items[{i}].amount_in_minor_units must be at least 1 (amounts are in minor \
                 units — $100.00 is 10000)"
            )));
        }
    }

    let customer =
        resolve_or_create_customer(client, args.customer_email.trim(), args.customer_name).await?;

    let mut form = Form::new();
    form.push("customer_id", &customer.id);
    form.push("currency_code", &currency);
    // Unasked-for, and load-bearing. Chargebee's default follows the customer
    // record and charges a stored card the moment the invoice exists — verified
    // against a live site, which answered `payment_method_not_present`. Two
    // reasons to override it: "send an invoice" is not "take a payment", and an
    // auto-collected invoice is already paid, which would make the "has Alan
    // paid?" flow (#788) answer itself.
    form.push("auto_collection", "off");
    form.push_opt("net_term_days", args.due_days);
    form.push_opt("invoice_note", args.invoice_note);
    for (i, line) in args.line_items.iter().enumerate() {
        form.push_indexed("charges", "description", i, &line.description);
        form.push_indexed("charges", "amount", i, line.amount_in_minor_units);
    }

    let body = client
        .post_form(
            "/invoices/create_for_charge_items_and_charges",
            &form,
            args.idempotency_key.as_deref(),
        )
        .await?;
    let invoice = require(&body, "invoice")?.clone();
    let url = payment_url(client, &customer.id, &currency).await;
    Ok(summarize_invoice(&invoice, url))
}

/// Fetches one invoice by id.
pub async fn get_invoice(client: &ChargebeeClient, args: GetInvoiceArgs) -> Result<InvoiceSummary> {
    let id = args.invoice_id.trim();
    if id.is_empty() {
        return Err(invalid("`invoice_id` is required"));
    }
    let body = client
        .get(&format!("/invoices/{}", urlencode(id)), &Form::new())
        .await?;
    Ok(summarize_invoice(require(&body, "invoice")?, None))
}

/// Lists invoices, optionally narrowed to one customer and/or status.
pub async fn list_invoices(
    client: &ChargebeeClient,
    args: ListInvoicesArgs,
) -> Result<Vec<InvoiceSummary>> {
    let mut query = Form::new();
    if let Some(email) = args
        .customer_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // An email that matches nobody must return "no invoices", not "every
        // invoice on the site" — which is what dropping an unresolvable filter
        // would do.
        let Some(customer) = get_customer(client, email).await? else {
            return Ok(Vec::new());
        };
        query.push("customer_id[is]", customer.id);
    }
    query.push_opt("status[is]", args.status);
    query.push_opt("limit", args.limit);

    let body = client.get("/invoices", &query).await?;
    Ok(body
        .get("list")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("invoice"))
                .map(|invoice| summarize_invoice(invoice, None))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_invoice_summary_keeps_the_facts_an_operator_asked_about() {
        let raw = json!({
            "id": "inv_1", "customer_id": "cus_1", "status": "payment_due",
            "currency_code": "USD", "total": 10000, "amount_due": 10000,
            "amount_paid": 0, "due_date": 1786603009,
            "line_items": [{"description": "Consulting", "amount": 10000}],
            // Noise the projection must drop rather than hand to a model.
            "linked_payments": [], "site_details_at_creation": {"timezone": "UTC"}
        });
        let s = summarize_invoice(&raw, Some("https://pay.example".to_string()));
        assert_eq!(s.id, "inv_1");
        assert_eq!(s.status, "payment_due");
        assert_eq!(s.total_in_minor_units, 10_000);
        assert_eq!(s.amount_due_in_minor_units, 10_000);
        assert_eq!(s.line_items, vec!["Consulting".to_string()]);
        assert_eq!(s.payment_url.as_deref(), Some("https://pay.example"));
    }

    #[test]
    fn a_missing_field_does_not_panic_the_projection() {
        // Chargebee omits `due_date` on some invoice shapes, and a summary that
        // panicked on one would take the whole turn with it.
        let s = summarize_invoice(&json!({"id": "inv_2"}), None);
        assert_eq!(s.id, "inv_2");
        assert_eq!(s.status, "unknown");
        assert_eq!(s.due_date, None);
        assert!(s.line_items.is_empty());
    }

    #[test]
    fn a_full_name_splits_into_chargebee_first_and_last() {
        let one = summarize_customer(&json!({"id": "c1", "first_name": "Alan"}));
        assert_eq!(one.name.as_deref(), Some("Alan"));
        let two = summarize_customer(
            &json!({"id": "c2", "first_name": "Ada", "last_name": "Byron", "email": "a@b.test"}),
        );
        assert_eq!(two.name.as_deref(), Some("Ada Byron"));
        assert_eq!(two.email.as_deref(), Some("a@b.test"));
    }

    #[test]
    fn ids_are_percent_encoded_so_they_cannot_retarget_the_path() {
        assert_eq!(urlencode("inv_123"), "inv_123");
        assert_eq!(urlencode("../customers/x"), "..%2Fcustomers%2Fx");
    }

    // ---- Wire-level tests -------------------------------------------------
    //
    // These drive a stub over a real socket rather than stopping at argument
    // validation. The wire format is where this module is most likely to be
    // wrong — form-encoded rather than JSON, `charges[amount][0]` nesting,
    // `email[is]` rather than `email` — and none of it is exercised by a test
    // that never builds a request. Every shape asserted below was also checked
    // against a live Chargebee site.

    use crate::chargebee::types::{ChargeLine, ChargebeeConfig};
    use std::sync::{Arc, Mutex};

    /// A canned reply: `"<METHOD> <path fragment>"`, status, JSON body.
    type Route = (&'static str, u16, &'static str);
    /// The stub's shared state: what it has seen, and what to answer with.
    type StubState = (Arc<Mutex<Vec<Seen>>>, Arc<Vec<Route>>);

    /// One request the stub saw.
    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        path: String,
        query: String,
        body: String,
    }

    /// Serves canned responses by path prefix and records every request.
    ///
    /// Keyed on `"<METHOD> <path fragment>"` because `send_invoice` is three
    /// calls in a row (customer lookup, invoice create, payment link) and two of
    /// them share the `/customers` path — a route table keyed on path alone
    /// answers the create with the lookup's body, which is exactly how the
    /// fabricated-empty-id bug surfaced.
    async fn stub<F, Fut, T>(routes: Vec<Route>, call: F) -> (Result<T>, Vec<Seen>)
    where
        F: FnOnce(ChargebeeClient) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        use axum::extract::State;
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let routes = Arc::new(routes);

        let handler = move |State((seen, routes)): State<StubState>,
                            method: axum::http::Method,
                            uri: axum::http::Uri,
                            body: String| async move {
            let path = format!("{method} {}", uri.path());
            seen.lock().expect("lock").push(Seen {
                method: method.to_string(),
                path: uri.path().to_string(),
                query: uri.query().unwrap_or_default().to_string(),
                body,
            });
            let (status, payload) = routes
                .iter()
                .find(|(prefix, _, _)| path.contains(prefix))
                .map(|(_, s, b)| (*s, *b))
                .unwrap_or((404, "{}"));
            (
                axum::http::StatusCode::from_u16(status).expect("status"),
                [("content-type", "application/json")],
                payload,
            )
        };

        let app = axum::Router::new()
            .fallback(axum::routing::any(handler))
            .with_state((seen.clone(), routes));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
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
        let out = call(client).await;
        server.abort();
        let requests = seen.lock().expect("lock").clone();
        (out, requests)
    }

    fn line(desc: &str, minor: i64) -> ChargeLine {
        ChargeLine {
            description: desc.to_string(),
            amount_in_minor_units: minor,
        }
    }

    const NO_CUSTOMER: &str = r#"{"list":[]}"#;
    const ONE_CUSTOMER: &str =
        r#"{"list":[{"customer":{"id":"cus_1","email":"alan@tinyhumans.ai"}}]}"#;
    const CREATED_CUSTOMER: &str = r#"{"customer":{"id":"cus_new","email":"alan@tinyhumans.ai"}}"#;
    const CREATED_INVOICE: &str = r#"{"invoice":{"id":"inv_1","customer_id":"cus_new","status":"payment_due","currency_code":"USD","total":10000,"amount_due":10000,"amount_paid":0,"line_items":[{"description":"Consulting"}]}}"#;
    const HOSTED_PAGE: &str =
        r#"{"hosted_page":{"url":"https://acme.chargebee.com/pages/v3/abc"}}"#;

    #[tokio::test]
    async fn an_unknown_customer_is_created_before_invoicing() {
        // TC-05: the operator names an email, not an internal id.
        let (result, seen) = stub(
            vec![
                ("GET /customers", 200, NO_CUSTOMER),
                ("POST /customers", 200, CREATED_CUSTOMER),
                (
                    "POST /invoices/create_for_charge_items_and_charges",
                    200,
                    CREATED_INVOICE,
                ),
                ("POST /hosted_pages/collect_now", 200, HOSTED_PAGE),
            ],
            |client| async move {
                send_invoice(
                    &client,
                    SendInvoiceArgs {
                        customer_email: "alan@tinyhumans.ai".to_string(),
                        customer_name: Some("Alan Turing".to_string()),
                        currency_code: "usd".to_string(),
                        line_items: vec![line("Consulting", 10_000)],
                        due_days: Some(7),
                        invoice_note: None,
                        idempotency_key: None,
                    },
                )
                .await
            },
        )
        .await;

        let invoice = result.expect("invoice is created");
        assert_eq!(invoice.status, "payment_due");
        assert_eq!(invoice.total_in_minor_units, 10_000);
        assert_eq!(
            invoice.payment_url.as_deref(),
            Some("https://acme.chargebee.com/pages/v3/abc")
        );

        // The exact sequence matters: look up, then create, then invoice the id
        // the create returned. Asserting it here is what catches a reordering
        // that would invoice against an id nothing had produced yet.
        let calls: Vec<(&str, &str)> = seen
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            calls,
            vec![
                ("GET", "/customers"),
                ("POST", "/customers"),
                ("POST", "/invoices/create_for_charge_items_and_charges"),
                ("POST", "/hosted_pages/collect_now"),
            ]
        );

        // The lookup is filtered with the operator suffix — a bare `email=` is
        // ignored by Chargebee and would match the wrong customer.
        assert!(seen[0].query.contains("email%5Bis%5D="), "{:?}", seen[0]);
        // Then the create, splitting the display name across Chargebee's fields.
        assert!(seen[1].body.contains("first_name=Alan"), "{:?}", seen[1]);
        assert!(seen[1].body.contains("last_name=Turing"), "{:?}", seen[1]);
        // Then the invoice, against the id the create returned.
        let inv = &seen[2];
        assert!(inv.body.contains("customer_id=cus_new"), "{inv:?}");
        assert!(inv.body.contains("currency_code=USD"), "{inv:?}");
        assert!(inv.body.contains("net_term_days=7"), "{inv:?}");
        assert!(
            inv.body.contains("charges%5Bamount%5D%5B0%5D=10000"),
            "{inv:?}"
        );
        // The guard that a live site taught us: without this Chargebee charges
        // a stored card on creation and the invoice is born paid.
        assert!(inv.body.contains("auto_collection=off"), "{inv:?}");
    }

    #[tokio::test]
    async fn an_existing_customer_is_reused_and_never_renamed() {
        let (result, seen) = stub(
            vec![
                ("GET /customers", 200, ONE_CUSTOMER),
                (
                    "POST /invoices/create_for_charge_items_and_charges",
                    200,
                    CREATED_INVOICE,
                ),
                ("POST /hosted_pages/collect_now", 200, HOSTED_PAGE),
            ],
            |client| async move {
                send_invoice(
                    &client,
                    SendInvoiceArgs {
                        customer_email: "alan@tinyhumans.ai".to_string(),
                        // Supplied, and must be ignored: invoicing someone is
                        // not a licence to rewrite their name.
                        customer_name: Some("Wrong Name".to_string()),
                        currency_code: "USD".to_string(),
                        line_items: vec![line("Consulting", 10_000)],
                        due_days: None,
                        invoice_note: None,
                        idempotency_key: None,
                    },
                )
                .await
            },
        )
        .await;

        result.expect("invoice is created");
        assert!(
            !seen.iter().any(|r| r.body.contains("Wrong")),
            "an existing customer must not be renamed: {seen:?}"
        );
        assert!(
            seen.iter().any(|r| r.body.contains("customer_id=cus_1")),
            "the existing id must be used: {seen:?}"
        );
    }

    #[tokio::test]
    async fn an_unresolvable_email_returns_no_invoices_rather_than_all_of_them() {
        // Dropping an unresolvable filter would list the whole site's invoices
        // in answer to "has this stranger paid?".
        let (result, seen) = stub(
            vec![("GET /customers", 200, NO_CUSTOMER)],
            |client| async move {
                list_invoices(
                    &client,
                    ListInvoicesArgs {
                        customer_email: Some("nobody@nowhere.test".to_string()),
                        ..Default::default()
                    },
                )
                .await
            },
        )
        .await;

        assert!(result.expect("lookup succeeds").is_empty());
        assert_eq!(
            seen.len(),
            1,
            "the invoice list must not be reached: {seen:?}"
        );
    }

    #[tokio::test]
    async fn an_invoice_survives_a_payment_link_that_cannot_be_raised() {
        // A site with no gateway configured refuses the hosted page. Reporting
        // "no invoice" for an invoice that exists is the worst answer available.
        let (result, _) = stub(
            vec![
                ("GET /customers", 200, ONE_CUSTOMER),
                (
                    "POST /invoices/create_for_charge_items_and_charges",
                    200,
                    CREATED_INVOICE,
                ),
                (
                    "POST /hosted_pages/collect_now",
                    400,
                    r#"{"message":"no gateway","api_error_code":"invalid_request"}"#,
                ),
            ],
            |client| async move {
                send_invoice(
                    &client,
                    SendInvoiceArgs {
                        customer_email: "alan@tinyhumans.ai".to_string(),
                        customer_name: None,
                        currency_code: "USD".to_string(),
                        line_items: vec![line("Consulting", 10_000)],
                        due_days: None,
                        invoice_note: None,
                        idempotency_key: None,
                    },
                )
                .await
            },
        )
        .await;

        let invoice = result.expect("the invoice itself still succeeds");
        assert_eq!(invoice.id, "inv_1");
        assert_eq!(invoice.payment_url, None);
    }

    #[tokio::test]
    async fn dollars_written_as_cents_are_caught_at_the_floor_only() {
        // The naming convention carries the real weight; the floor is all a
        // guard can check without reading intent.
        let (result, seen) = stub(vec![], |client| async move {
            send_invoice(
                &client,
                SendInvoiceArgs {
                    customer_email: "alan@tinyhumans.ai".to_string(),
                    customer_name: None,
                    currency_code: "USD".to_string(),
                    line_items: vec![line("Consulting", 0)],
                    due_days: None,
                    invoice_note: None,
                    idempotency_key: None,
                },
            )
            .await
        })
        .await;

        let err = result.expect_err("a zero amount is rejected");
        assert!(err.to_string().contains("at least 1"), "got: {err}");
        assert!(seen.is_empty(), "rejected before any request: {seen:?}");
    }
}
