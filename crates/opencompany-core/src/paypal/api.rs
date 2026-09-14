//! The two read operations issue #789 scopes: wallet balance and recent
//! transactions.
//!
//! Deliberately read-only. #789 lists `send_payment` as optional and requires a
//! scoping decision before implementation; moving money is not something to
//! ship on an "optional" line in an issue.

use serde::Serialize;
use serde_json::Value;

use super::client::PaypalClient;
use crate::error::{OpenCompanyError, Result};

/// One currency's balance in the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Balance {
    /// ISO 4217 code.
    pub currency_code: String,
    /// The amount as PayPal reports it — a decimal STRING, e.g. `"4320.50"`.
    ///
    /// Kept as text rather than parsed into a float: this value is rendered to
    /// an operator, and `4320.50` through an `f64` is how a balance acquires a
    /// trailing `0000001`. Nothing here does arithmetic on it.
    pub available: String,
    /// Funds not yet available, same format.
    pub withheld: String,
    /// Whether this is the account's primary currency.
    pub primary: bool,
}

/// One transaction, projected down from PayPal's very large record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Transaction {
    /// PayPal's transaction id.
    pub id: String,
    /// ISO 8601, as PayPal reports it.
    pub date: String,
    /// Signed decimal string — negative for money leaving the account.
    pub amount: String,
    /// ISO 4217 code.
    pub currency_code: String,
    /// `S` success, `P` pending, `V` reversed, `D` denied.
    pub status: String,
    /// The counterparty's name or email, when PayPal supplies one.
    pub counterparty: Option<String>,
    /// The payer-supplied note, when present.
    pub note: Option<String>,
}

/// Rewrites PayPal's opaque "Data for the given start date is not available"
/// into something the caller can act on.
///
/// PayPal serves transaction data on a lag — a completed payment takes up to
/// three hours to appear — and rejects any window whose start is inside that
/// gap. Its own message says only that data "is not available", which reads as
/// "there were no transactions" rather than "ask for an earlier window", so an
/// agent handed it starts guessing at timeframes instead of moving the start
/// date back. That is exactly what happened in testing: a start date of today
/// failed, and the agent asked the operator to pick a different period rather
/// than knowing what to do.
///
/// Only this one error is rewritten, and the original text is kept alongside the
/// remedy so nothing is hidden.
fn explain_unavailable_window(error: OpenCompanyError) -> OpenCompanyError {
    let OpenCompanyError::Paypal {
        status,
        code,
        message,
    } = &error
    else {
        return error;
    };
    // Matched on PayPal's specific sentence, not a bare "not available".
    // The looser test also caught unrelated failures — "The requested resource
    // is not available" is a 404, and answering it with advice about start
    // dates sends the agent adjusting timeframes for a problem that has nothing
    // to do with them.
    if !message
        .to_ascii_lowercase()
        .contains("data for the given start date is not available")
    {
        return error;
    }
    OpenCompanyError::Paypal {
        status: *status,
        code: code.clone(),
        message: format!(
            "{message} PayPal publishes transactions on a delay of up to 3 hours, so a window \
             that starts today may have no data yet — retry with a `start_date` at least a day \
             earlier. The window must also span no more than 31 days."
        ),
    }
}

fn text(value: Option<&Value>, key: &str) -> Option<String> {
    value?
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Reads a money field PayPal must have supplied, or fails.
///
/// The same rule the missing-array checks below already apply, one level down:
/// **never invent a number about money.** These fields used to default to
/// `"0.00"`, so a response whose shape drifted — a renamed field, a projection
/// PayPal serves to some accounts, an object that arrives empty — reported a
/// funded wallet as empty and a real payment as a zero-value transaction. That
/// is not a degraded answer, it is a confident wrong one: an agent told the
/// balance is `0.00` says the company has no money, and an operator acts on it.
///
/// An error says "PayPal's reply did not parse", which is true and which
/// somebody can fix. `field` names the JSON path so the report identifies which
/// part of the shape moved; the reply itself goes to the log, never into the
/// message — same rule as `unparsed_body_message`.
fn money(parent: Option<&Value>, key: &str, field: &str) -> Result<String> {
    text(parent, key).ok_or_else(|| {
        tracing::warn!(
            field,
            parent = %parent.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                .chars().take(200).collect::<String>(),
            "[paypal] reply carried no amount where one is required"
        );
        OpenCompanyError::Paypal {
            status: 0,
            code: "unexpected_response".to_string(),
            message: format!(
                "PayPal's reply carried no `{field}`, and this host does not substitute a zero \
                 for an amount PayPal did not report. The reply is in the host log."
            ),
        }
    })
}

/// Fetches the account's balances.
pub async fn get_wallet_balance(client: &PaypalClient) -> Result<Vec<Balance>> {
    let body = client.get("/v1/reporting/balances", &[]).await?;
    let balances = body
        .get("balances")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            // Logged, not relayed — same rule as the client's
            // `unparsed_body_message` and `chargebee::api::require`.
            tracing::warn!(
                body = %body.to_string().chars().take(200).collect::<String>(),
                "[paypal] reply carried no `balances` array"
            );
            OpenCompanyError::Paypal {
                status: 0,
                code: "unexpected_response".to_string(),
                message:
                    "PayPal's reply carried no `balances` array. The reply is in the host log."
                        .to_string(),
            }
        })?;

    balances
        .iter()
        .map(|entry| {
            Ok(Balance {
                currency_code: text(Some(entry), "currency")
                    .or_else(|| text(entry.get("total_balance"), "currency_code"))
                    .unwrap_or_default(),
                available: money(
                    entry.get("available_balance"),
                    "value",
                    "balances[].available_balance.value",
                )?,
                withheld: money(
                    entry.get("withheld_balance"),
                    "value",
                    "balances[].withheld_balance.value",
                )?,
                primary: entry
                    .get("primary")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Fetches transactions between two ISO 8601 instants.
///
/// PayPal caps the window at **31 days** and publishes on a lag of up to three
/// hours; both limits are enforced by PayPal, not here. Only the non-empty
/// check below is local — parsing ISO 8601 to pre-validate the span would mean
/// duplicating PayPal's calendar rules to save one round trip, and getting that
/// subtly wrong would reject windows PayPal would have accepted. What this does
/// instead is make PayPal's own refusal actionable: see
/// [`explain_unavailable_window`].
pub async fn list_transactions(
    client: &PaypalClient,
    start_date: &str,
    end_date: &str,
    page_size: Option<i64>,
) -> Result<Vec<Transaction>> {
    if start_date.trim().is_empty() || end_date.trim().is_empty() {
        return Err(OpenCompanyError::Paypal {
            status: 0,
            code: "invalid_arguments".to_string(),
            message: "`start_date` and `end_date` are both required, in ISO 8601. PayPal allows a \
                      window of at most 31 days and publishes on a delay of up to 3 hours."
                .to_string(),
        });
    }

    let query = vec![
        ("start_date".to_string(), start_date.trim().to_string()),
        ("end_date".to_string(), end_date.trim().to_string()),
        // Without this PayPal returns only ids and amounts — no counterparty,
        // no note — and the answer reads as a list of anonymous numbers.
        (
            "fields".to_string(),
            "transaction_info,payer_info".to_string(),
        ),
        (
            "page_size".to_string(),
            page_size.unwrap_or(20).clamp(1, 500).to_string(),
        ),
    ];

    let body = client
        .get("/v1/reporting/transactions", &query)
        .await
        .map_err(explain_unavailable_window)?;
    let rows = body
        .get("transaction_details")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            // A successful empty array means no transactions. A missing array
            // means the response shape changed, and reporting the latter as an
            // empty history would be a confident lie about money.
            tracing::warn!(
                body = %body.to_string().chars().take(200).collect::<String>(),
                "[paypal] reply carried no `transaction_details` array"
            );
            OpenCompanyError::Paypal {
                status: 0,
                code: "unexpected_response".to_string(),
                message: "PayPal's reply carried no `transaction_details` array. The reply is in the host log."
                    .to_string(),
            }
        })?;

    rows.iter()
        .map(|row| {
            let info = row.get("transaction_info");
            let payer = row.get("payer_info");
            Ok(Transaction {
                id: text(info, "transaction_id").unwrap_or_default(),
                date: text(info, "transaction_initiation_date").unwrap_or_default(),
                amount: money(
                    info.and_then(|i| i.get("transaction_amount")),
                    "value",
                    "transaction_details[].transaction_info.transaction_amount.value",
                )?,
                currency_code: text(
                    info.and_then(|i| i.get("transaction_amount")),
                    "currency_code",
                )
                .unwrap_or_default(),
                status: text(info, "transaction_status").unwrap_or_default(),
                counterparty: text(
                    payer.and_then(|p| p.get("payer_name")),
                    "alternate_full_name",
                )
                .or_else(|| text(payer, "email_address")),
                note: text(info, "transaction_note"),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::company::paypal::PaypalEnvironment;
    use crate::paypal::client::{PaypalClient, PaypalConfig};

    /// A client pointed at a stub serving `body` for every PayPal path.
    ///
    /// The token endpoint answers too, so an operation under test goes through
    /// the same auth path it does in production rather than a client with the
    /// credential step skipped. Abort the returned handle when done.
    async fn stub_client(body: &'static str) -> (PaypalClient, tokio::task::JoinHandle<()>) {
        let handler = move |uri: axum::http::Uri| async move {
            let payload = if uri.path().contains("oauth2/token") {
                r#"{"access_token":"tok","expires_in":32400}"#
            } else {
                body
            };
            (
                axum::http::StatusCode::OK,
                [("content-type", "application/json")],
                payload,
            )
        };
        let app = axum::Router::new().fallback(axum::routing::any(handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let client = PaypalClient::with_base_url(
            PaypalConfig {
                client_id: "AY_id".to_string(),
                client_secret: "EL_secret".to_string(),
                environment: PaypalEnvironment::Sandbox,
            },
            format!("http://{addr}"),
        )
        .expect("client builds");
        (client, server)
    }

    #[tokio::test]
    async fn a_balance_keeps_paypals_decimal_string_verbatim() {
        // Through an f64 this becomes 4320.500000000001 on some inputs. It is
        // rendered to an operator and never computed on, so it stays text.
        // Driven through `get_wallet_balance` itself, against a stub serving
        // PayPal's real response shape. An earlier version of this test rebuilt
        // the projection inline and asserted on its own copy — which passes
        // whatever the production projection does, including not existing.
        let (client, server) = stub_client(
            r#"{"balances":[{
                "currency":"USD","primary":true,
                "available_balance":{"currency_code":"USD","value":"4320.50"},
                "withheld_balance":{"currency_code":"USD","value":"12.30"}
            },{
                "currency":"EUR","primary":false,
                "available_balance":{"currency_code":"EUR","value":"0.00"},
                "withheld_balance":{"currency_code":"EUR","value":"0.00"}
            }]}"#,
        )
        .await;
        let balances = get_wallet_balance(&client).await.expect("balances");
        server.abort();

        assert_eq!(balances.len(), 2);
        // Through an f64 this becomes 4320.500000000001 on some inputs. It is
        // rendered to an operator and never computed on, so it stays text.
        assert_eq!(balances[0].available, "4320.50");
        assert_eq!(balances[0].withheld, "12.30");
        assert_eq!(balances[0].currency_code, "USD");
        assert!(balances[0].primary);
        assert_eq!(balances[1].currency_code, "EUR");
        assert!(!balances[1].primary);
    }

    #[tokio::test]
    async fn a_reply_without_a_balances_array_is_an_error_not_an_empty_wallet() {
        // An account always has balances, so a reply without them is a broken
        // integration — reporting "no funds" would be a confident lie about
        // money. The transaction query follows the same rule: an empty array
        // is a real empty history, but a missing array is not.
        let (client, server) = stub_client(r#"{"name":"INTERNAL","debug_id":"x"}"#).await;
        let err = get_wallet_balance(&client)
            .await
            .expect_err("a missing array is an error");
        server.abort();
        let rendered = err.to_string();
        assert!(rendered.contains("balances"), "{rendered}");
        // And the body itself is logged, not relayed into the transcript.
        assert!(!rendered.contains("debug_id"), "{rendered}");
    }

    #[tokio::test]
    async fn a_balance_with_no_amount_is_an_error_rather_than_a_fabricated_zero() {
        // These fields used to default to "0.00". A response whose shape drifted
        // therefore reported a funded wallet as EMPTY — not a degraded answer but
        // a confident wrong one, which an agent relays and an operator acts on.
        for (label, body) in [
            (
                "no available_balance at all",
                r#"{"balances":[{"currency":"USD","primary":true,
                    "withheld_balance":{"currency_code":"USD","value":"12.30"}}]}"#,
            ),
            (
                "an available_balance with no value",
                r#"{"balances":[{"currency":"USD","primary":true,
                    "available_balance":{"currency_code":"USD"},
                    "withheld_balance":{"currency_code":"USD","value":"12.30"}}]}"#,
            ),
            (
                "an empty-string value",
                r#"{"balances":[{"currency":"USD","primary":true,
                    "available_balance":{"currency_code":"USD","value":""},
                    "withheld_balance":{"currency_code":"USD","value":"12.30"}}]}"#,
            ),
            (
                "no withheld_balance",
                r#"{"balances":[{"currency":"USD","primary":true,
                    "available_balance":{"currency_code":"USD","value":"4320.50"}}]}"#,
            ),
        ] {
            let (client, server) = stub_client(body).await;
            let err = match get_wallet_balance(&client).await {
                Ok(balances) => {
                    panic!("{label}: a missing amount must not become a number: {balances:?}")
                }
                Err(err) => err,
            };
            server.abort();
            let rendered = err.to_string();
            // The report names WHICH part of the shape moved, so the fix is
            // findable rather than "PayPal broke".
            assert!(rendered.contains("balance.value"), "{label}: {rendered}");
            assert!(!rendered.contains("0.00"), "{label}: {rendered}");
        }
    }

    #[tokio::test]
    async fn a_transaction_with_no_amount_is_an_error_rather_than_a_zero_payment() {
        // Same rule for the transaction list: a payment reported as 0.00 reads
        // as a failed or free transaction, which is a lie about money rather
        // than a gap in the answer.
        let (client, server) = stub_client(
            r#"{"transaction_details":[{"transaction_info":{
                "transaction_id":"T1","transaction_status":"S",
                "transaction_initiation_date":"2026-08-01T00:00:00+0000"
            }}]}"#,
        )
        .await;
        let err = match list_transactions(
            &client,
            "2026-08-01T00:00:00Z",
            "2026-08-02T00:00:00Z",
            None,
        )
        .await
        {
            Ok(rows) => panic!("a missing amount must not become 0.00: {rows:?}"),
            Err(err) => err,
        };
        server.abort();
        let rendered = err.to_string();
        assert!(rendered.contains("transaction_amount.value"), "{rendered}");
        assert!(!rendered.contains("0.00"), "{rendered}");
    }

    #[tokio::test]
    async fn a_reply_without_transaction_details_is_an_error_not_an_empty_history() {
        let (client, server) = stub_client(r#"{"name":"INTERNAL","debug_id":"x"}"#).await;
        let err = list_transactions(
            &client,
            "2026-08-01T00:00:00Z",
            "2026-08-02T00:00:00Z",
            None,
        )
        .await
        .expect_err("a missing array is an error");
        server.abort();
        let rendered = err.to_string();
        assert!(rendered.contains("transaction_details"), "{rendered}");
        assert!(!rendered.contains("debug_id"), "{rendered}");
    }

    #[test]
    fn an_unavailable_window_is_explained_rather_than_relayed() {
        // PayPal's own words read as "there were no transactions", which sends
        // an agent guessing at timeframes instead of moving the start date back.
        let raw = OpenCompanyError::Paypal {
            status: 400,
            code: "INVALID_REQUEST".to_string(),
            message: "Data for the given start date is not available.".to_string(),
        };
        let explained = explain_unavailable_window(raw).to_string();
        assert!(
            explained.contains("Data for the given start date"),
            "{explained}"
        );
        assert!(explained.contains("3 hours"), "{explained}");
        assert!(explained.contains("start_date"), "{explained}");

        // Every other failure passes through untouched — this must not become a
        // catch-all that buries unrelated PayPal errors under a date hint.
        let other = OpenCompanyError::Paypal {
            status: 401,
            code: "NOT_AUTHORIZED".to_string(),
            message: "Authorization failed due to insufficient permissions.".to_string(),
        };
        let untouched = explain_unavailable_window(other).to_string();
        assert!(!untouched.contains("3 hours"), "{untouched}");

        // And neither does an unrelated failure that merely says "not
        // available" — a 404 answered with advice about start dates sends the
        // agent adjusting timeframes for a problem that has nothing to do with
        // them.
        let missing = OpenCompanyError::Paypal {
            status: 404,
            code: "RESOURCE_NOT_FOUND".to_string(),
            message: "The requested resource is not available.".to_string(),
        };
        let relayed = explain_unavailable_window(missing).to_string();
        assert!(!relayed.contains("3 hours"), "{relayed}");
        assert!(!relayed.contains("start_date"), "{relayed}");
    }

    #[tokio::test]
    async fn a_missing_date_is_rejected_before_any_request() {
        use crate::company::paypal::PaypalEnvironment;
        use crate::paypal::client::{PaypalClient, PaypalConfig};
        // Port 0 never listens, so anything that reached the network would fail
        // fast rather than hang — the rejection must happen before that.
        let client = PaypalClient::with_base_url(
            PaypalConfig {
                client_id: "id".into(),
                client_secret: "secret".into(),
                environment: PaypalEnvironment::Sandbox,
            },
            "http://127.0.0.1:0".into(),
        )
        .expect("builds");

        let err = list_transactions(&client, "", "2026-08-13T00:00:00Z", None)
            .await
            .expect_err("an empty start is rejected");
        assert!(err.to_string().contains("31 days"), "got: {err}");
    }
}
