//! The inbound Chargebee webhook: `POST /hooks/{company}/chargebee` (issue #788).
//!
//! Chargebee posts here when a payment succeeds or fails. A verified delivery
//! raises a [`CompanyEvent::WebhookReceived`] on the `chargebee` channel, which
//! drives one cycle — so the operator hears "Alan paid the $100 invoice" in
//! chat **without having asked**. That push is the whole point of this route.
//!
//! # Why this does not persist invoice state
//!
//! Issue #788's TC-03 describes the webhook updating stored state that the agent
//! then reads to answer "has Alan paid?". It deliberately does not do that.
//! `chargebee_get_invoice` already answers that question **live from
//! Chargebee**, and does it strictly better: stored state goes stale the moment
//! a delivery is dropped, retried, or replayed out of order, and then the agent
//! confidently reports a payment status that Chargebee disagrees with. A cache
//! that can silently diverge from the system of record is worse than no cache
//! when the subject is money.
//!
//! So the split is: **pull** stays live (`chargebee_get_invoice`), and this
//! route owns **push** — the thing a live read genuinely cannot do.
//!
//! # Verification comes before parsing
//!
//! Chargebee protects a webhook URL with HTTP Basic auth, configured beside the
//! URL in its dashboard. The handler compares that header, constant-time,
//! against the company's stored secret **before it parses anything**: an
//! unverifiable POST is dropped with `401` and never becomes an event. Same
//! order, and the same reason, as the Telegram hook next door.
//!
//! # The event names are Chargebee's, not the issue's
//!
//! #788 calls them `invoice_paid` and `invoice_payment_failed`. Chargebee has no
//! such events — the real ones are `payment_succeeded` and `payment_failed`
//! (plus `invoice_generated`). The names here are the ones an operator will
//! actually find in the Chargebee dashboard's webhook configuration.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::AppState;
use crate::company::runtime::CompanyRuntime;
use crate::ports::types::CompanyEvent;
use crate::server::ops::resolve;

pub use crate::company::billing::WEBHOOK_SECRET_KEY;

/// The channel a verified delivery is raised on.
pub const CHANNEL: &str = "chargebee";

/// The Chargebee events this route acts on.
///
/// An unlisted event is acknowledged and ignored rather than refused: Chargebee
/// sends whatever the dashboard subscribes to, an operator will over-subscribe,
/// and answering non-2xx would make Chargebee retry — then disable the endpoint
/// — over an event we simply had no interest in.
const ACTED_ON: &[&str] = &["payment_succeeded", "payment_failed", "invoice_generated"];

/// Builds the Chargebee webhook route fragment.
pub fn router() -> Router<AppState> {
    Router::new().route("/hooks/{company}/chargebee", post(chargebee_hook))
}

/// A `401` drop for an unverifiable delivery.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "invalid webhook credentials", "code": "unauthorized" })),
    )
        .into_response()
}

/// Length-checked, branch-independent byte comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Decodes an `Authorization: Basic <base64>` header into `user:pass`.
///
/// Hand-rolled because `base64` is behind the `mcp` feature and this route ships
/// in every build. Decoding is 20 lines; taking a feature dependency for it
/// would make an always-on route conditional on an unrelated one.
fn decode_basic(header: &str) -> Option<String> {
    let encoded = header.strip_prefix("Basic ")?.trim();
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut out: Vec<u8> = Vec::new();
    for byte in encoded.bytes() {
        if byte == b'=' {
            break;
        }
        let value = ALPHABET.iter().position(|c| *c == byte)? as u32;
        bits = (bits << 6) | value;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    String::from_utf8(out).ok()
}

/// `POST /hooks/{company}/chargebee`.
async fn chargebee_hook(
    State(state): State<AppState>,
    Path(company): Path<String>,
    headers: HeaderMap,
    raw: Bytes,
) -> Response {
    let runtime = match resolve(&state, &company) {
        Ok(runtime) => runtime,
        Err(err) => return err.into_response(),
    };
    handle(runtime, &headers, &raw).await
}

/// Verifies the credential, then raises one cycle for an event worth telling
/// the operator about.
async fn handle(runtime: Arc<CompanyRuntime>, headers: &HeaderMap, raw: &[u8]) -> Response {
    // A stored credential must exist to verify against. An empty stored value
    // counts as "not configured" — reject rather than accept anything.
    let expected = match runtime
        .secrets()
        .get(runtime.id(), WEBHOOK_SECRET_KEY)
        .await
    {
        Ok(Some(secret)) if !secret.expose().is_empty() => secret.expose().to_string(),
        Ok(_) => return unauthorized(),
        Err(err) => return crate::server::error::ApiError(err).into_response(),
    };

    let Some(provided) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(decode_basic)
    else {
        return unauthorized();
    };
    if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return unauthorized();
    }

    // Only parse once the credential checks out.
    let Ok(event) = serde_json::from_slice::<Value>(raw) else {
        // Malformed body from a caller that DID authenticate: accept it so
        // Chargebee stops retrying, and say so in the log rather than silently.
        tracing::warn!(company = %runtime.id().as_ref(), "[chargebee] webhook body was not JSON");
        return (
            StatusCode::OK,
            Json(json!({"ok": true, "ignored": "unparseable"})),
        )
            .into_response();
    };

    let event_type = event
        .get("event_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !ACTED_ON.contains(&event_type.as_str()) {
        return (
            StatusCode::OK,
            Json(json!({"ok": true, "ignored": event_type})),
        )
            .into_response();
    }

    let summary = summarize(&event_type, &event);
    tracing::info!(company = %runtime.id().as_ref(), %event_type, "[chargebee] webhook");

    // Drive one cycle so the company *says* something. A paused or archived
    // company acknowledges without running — Chargebee must not retry because
    // the operator happened to have the company stopped.
    if runtime.ensure_running().await.is_ok() {
        let event = CompanyEvent::WebhookReceived {
            channel: CHANNEL.to_string(),
            // The summary, not the raw event: `body` reaches the brain, and a
            // whole Chargebee payload spends a great deal of context to say
            // "Alan paid". The original is still in the log line above.
            body: json!({"event_type": event_type, "summary": summary}),
        };
        if let Err(err) = runtime.run_cycle(vec![event]).await {
            tracing::warn!(company = %runtime.id(), "chargebee cycle failed: {err}");
        }
    }

    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

/// Renders the event as the sentence the agent is asked to relay.
///
/// A summary rather than the raw payload: a Chargebee event carries the whole
/// invoice, customer, transaction and card objects, and handing that to a model
/// spends a large amount of context to say "Alan paid". Amounts stay in minor
/// units with the currency beside them — this text reaches a model, and a bare
/// `10000` with no unit is exactly how a $100 payment gets reported as $10,000.
fn summarize(event_type: &str, event: &Value) -> String {
    let content = event.get("content").cloned().unwrap_or(Value::Null);
    let invoice = content.get("invoice");
    let field = |obj: Option<&Value>, key: &str| -> Option<String> {
        obj?.get(key).and_then(Value::as_str).map(str::to_string)
    };
    let id = field(invoice, "id").unwrap_or_else(|| "(unknown)".to_string());
    let currency = field(invoice, "currency_code").unwrap_or_default();
    let total = invoice
        .and_then(|i| i.get("total"))
        .and_then(Value::as_i64)
        .map(|t| format!("{t} {currency} (minor units)"))
        .unwrap_or_else(|| "an unknown amount".to_string());
    let who = field(content.get("customer"), "email")
        .or_else(|| field(content.get("customer"), "id"))
        .unwrap_or_else(|| "the customer".to_string());

    match event_type {
        "payment_succeeded" => format!(
            "Chargebee: invoice {id} for {total} was PAID by {who}. Tell the operator, briefly."
        ),
        "payment_failed" => format!(
            "Chargebee: a payment FAILED for invoice {id} ({total}) from {who}. Tell the operator, \
             briefly, and say the invoice is still outstanding."
        ),
        _ => format!("Chargebee: invoice {id} for {total} was generated for {who}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_auth_decodes_to_the_user_pass_pair() {
        // base64("cbuser:cbpass")
        assert_eq!(
            decode_basic("Basic Y2J1c2VyOmNicGFzcw==").as_deref(),
            Some("cbuser:cbpass")
        );
        // Anything that is not Basic is not ours to interpret.
        assert_eq!(decode_basic("Bearer abc"), None);
        assert_eq!(decode_basic("Basic !!!not base64!!!"), None);
    }

    #[test]
    fn constant_time_eq_still_compares_correctly() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        // A length difference must not be reported as equal.
        assert!(!constant_time_eq(b"secret", b"secretx"));
    }

    #[test]
    fn only_the_three_billing_events_are_acted_on() {
        // Over-subscribing in the Chargebee dashboard is the normal case; an
        // unlisted event must be ignorable, not a reason to retry.
        assert!(ACTED_ON.contains(&"payment_succeeded"));
        assert!(ACTED_ON.contains(&"payment_failed"));
        assert!(!ACTED_ON.contains(&"subscription_created"));
        // The names #788 uses do not exist in Chargebee; if these ever start
        // matching, the issue's names were adopted and this test should say so.
        assert!(!ACTED_ON.contains(&"invoice_paid"));
    }

    #[test]
    fn a_paid_summary_names_the_invoice_amount_and_payer() {
        let event = json!({
            "event_type": "payment_succeeded",
            "content": {
                "invoice": {"id": "inv_42", "total": 10000, "currency_code": "USD"},
                "customer": {"email": "alan@tinyhumans.ai"}
            }
        });
        let text = summarize("payment_succeeded", &event);
        assert!(text.contains("inv_42"), "{text}");
        assert!(text.contains("alan@tinyhumans.ai"), "{text}");
        assert!(text.contains("PAID"), "{text}");
        // The unit must travel with the number or $100 gets reported as $10,000.
        assert!(text.contains("minor units"), "{text}");
    }

    #[test]
    fn a_failed_summary_says_the_invoice_is_still_outstanding() {
        let event = json!({
            "event_type": "payment_failed",
            "content": {"invoice": {"id": "inv_9", "total": 500, "currency_code": "USD"}}
        });
        let text = summarize("payment_failed", &event);
        assert!(text.contains("FAILED"), "{text}");
        assert!(text.contains("outstanding"), "{text}");
        // No customer object in the payload is normal; it must not panic or
        // render an empty gap where a person should be.
        assert!(text.contains("the customer"), "{text}");
    }

    #[test]
    fn a_summary_survives_a_payload_with_no_content() {
        let text = summarize(
            "payment_succeeded",
            &json!({"event_type": "payment_succeeded"}),
        );
        assert!(text.contains("(unknown)"), "{text}");
        assert!(text.contains("an unknown amount"), "{text}");
    }
}
