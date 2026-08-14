//! A thin HTTP client for the Chargebee Billing API v2.
//!
//! Three things about Chargebee's wire format drive the shape of this module,
//! and all three are easy to get wrong from memory (each is checked against
//! `spec/chargebee_api_v2_pc_v2_spec.json` in the `chargebee/openapi` repo):
//!
//! 1. **Writes are `application/x-www-form-urlencoded`, not JSON.** Posting
//!    JSON to `/invoices/create_for_charge_items_and_charges` fails with a
//!    parameter error, not a content-type error, so the mistake reads as a bad
//!    request body.
//! 2. **Nested parameters use bracket-array notation.** A two-line invoice is
//!    `charges[description][0]=…&charges[amount][0]=…&charges[description][1]=…`
//!    — the index is per-field, not per-object. [`Form::push_indexed`] is the
//!    only place that encoding exists.
//! 3. **Auth is HTTP Basic with the API key as the username and an empty
//!    password.** Not a bearer token.

use crate::error::{OpenCompanyError, Result};
use serde_json::Value;

use super::types::ChargebeeConfig;

/// The header Chargebee reads for idempotent replay of a `POST`.
const IDEMPOTENCY_HEADER: &str = "chargebee-idempotency-key";

/// Builds the error for a reply whose body could not be used.
fn err_body(status: u16, code: &str, body: &str) -> OpenCompanyError {
    OpenCompanyError::Chargebee {
        status,
        code: code.to_string(),
        message: format!(
            "Chargebee returned {status} with a body that is not a JSON object — got: {}",
            body.chars().take(200).collect::<String>()
        ),
    }
}

/// A form body under construction.
///
/// Deliberately a `Vec` of pairs rather than a map: Chargebee's bracket-array
/// notation repeats a prefix across indices, so key order is meaningful for
/// readability of the encoded body and there are no duplicate keys to collapse.
#[derive(Debug, Default)]
pub struct Form(Vec<(String, String)>);

impl Form {
    /// An empty body.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `key=value`.
    pub fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.push((key.into(), value.into()));
    }

    /// Appends `key=value` when `value` is `Some`, and nothing otherwise.
    ///
    /// Chargebee treats an empty string as an instruction to clear a field, so
    /// an omitted optional must be absent from the body rather than blank.
    pub fn push_opt(&mut self, key: impl Into<String>, value: Option<impl ToString>) {
        if let Some(v) = value {
            self.0.push((key.into(), v.to_string()));
        }
    }

    /// Appends `prefix[field][index]=value` — Chargebee's nested-array form.
    pub fn push_indexed(&mut self, prefix: &str, field: &str, index: usize, value: impl ToString) {
        self.0
            .push((format!("{prefix}[{field}][{index}]"), value.to_string()));
    }

    /// The pairs, in insertion order.
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }
}

/// A Chargebee API client bound to one site.
#[derive(Clone, Debug)]
pub struct ChargebeeClient {
    http: reqwest::Client,
    api_key: String,
    /// Resolved once at construction. Holding the string rather than deriving
    /// it per request is what lets a test point the client at a local stub
    /// without the production path carrying a test-only branch.
    base_url: String,
}

impl ChargebeeClient {
    /// Builds a client for `config`, talking to that site's real API.
    pub fn new(config: ChargebeeConfig) -> Result<Self> {
        let base_url = config.base_url();
        Self::with_base_url(config, base_url)
    }

    /// Builds a client against an explicit base URL, with no trailing slash.
    pub fn with_base_url(config: ChargebeeConfig, base_url: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            // Every request carries HTTP Basic with the API key as the
            // username. reqwest follows redirects by default and re-sends the
            // Authorization header, so a 30x pointing at `http://` would put the
            // key on the wire in clear text. Chargebee's API does not redirect,
            // so refusing them outright costs nothing and removes the downgrade
            // entirely — a scheme check would still leave same-scheme
            // redirection to an unintended host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| OpenCompanyError::Chargebee {
                status: 0,
                code: "client_build_failed".to_string(),
                message: e.to_string(),
            })?;
        Ok(Self {
            http,
            api_key: config.api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn base(&self) -> &str {
        &self.base_url
    }

    /// `POST path` with a form body, returning the decoded JSON response.
    pub async fn post_form(
        &self,
        path: &str,
        form: &Form,
        idempotency_key: Option<&str>,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base(), path);
        let mut req = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .form(form.pairs());
        if let Some(key) = idempotency_key {
            req = req.header(IDEMPOTENCY_HEADER, key);
        }
        Self::decode(req.send().await).await
    }

    /// `GET path` with query parameters, returning the decoded JSON response.
    pub async fn get(&self, path: &str, query: &Form) -> Result<Value> {
        let url = format!("{}{}", self.base(), path);
        let req = self
            .http
            .get(&url)
            .basic_auth(&self.api_key, Some(""))
            .query(query.pairs());
        Self::decode(req.send().await).await
    }

    /// Turns a transport result into either the parsed body or a
    /// [`OpenCompanyError::Chargebee`] carrying Chargebee's own error fields.
    ///
    /// Chargebee reports business failures (`payment_method` not enabled, a
    /// customer that does not exist) as a 4xx with a JSON body naming the
    /// problem. That body is far more useful to the agent than the status code,
    /// so it is preserved rather than flattened into "request failed".
    async fn decode(sent: std::result::Result<reqwest::Response, reqwest::Error>) -> Result<Value> {
        let response = sent.map_err(|e| OpenCompanyError::Chargebee {
            status: e.status().map(|s| s.as_u16()).unwrap_or(0),
            code: "transport_error".to_string(),
            message: e.to_string(),
        })?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|e| OpenCompanyError::Chargebee {
                status,
                code: "unreadable_body".to_string(),
                message: e.to_string(),
            })?;

        let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);

        if (200..300).contains(&status) {
            // A success whose body is not a JSON object is not a success we can
            // use: `Value::Null` would flow on and every field read would yield
            // a default, so a proxy's HTML 200 became an invoice with an empty
            // id rather than a reported failure. The raw-body fallback below
            // stays for NON-2xx replies, where prose is all there is.
            if !parsed.is_object() {
                return Err(err_body(status, "unexpected_response", &body));
            }
            return Ok(parsed);
        }

        Err(OpenCompanyError::Chargebee {
            status,
            code: parsed
                .get("api_error_code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            message: parsed
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                // A non-JSON body (a proxy's HTML 502, say) still has to say
                // something; the raw text is truncated so it stays readable.
                .unwrap_or_else(|| body.chars().take(300).collect()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_opt_omits_none_rather_than_sending_blank() {
        let mut form = Form::new();
        form.push("customer_id", "acme");
        form.push_opt("net_term_days", None::<i64>);
        form.push_opt("invoice_note", Some("Q3 retainer"));

        let keys: Vec<&str> = form.pairs().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["customer_id", "invoice_note"]);
    }

    #[test]
    fn indexed_encoding_is_per_field_not_per_object() {
        let mut form = Form::new();
        for (i, (desc, amount)) in [("Pro plan", 50_000), ("Setup", 2_500)].iter().enumerate() {
            form.push_indexed("charges", "description", i, desc);
            form.push_indexed("charges", "amount", i, amount);
        }

        let pairs: Vec<(&str, &str)> = form
            .pairs()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("charges[description][0]", "Pro plan"),
                ("charges[amount][0]", "50000"),
                ("charges[description][1]", "Setup"),
                ("charges[amount][1]", "2500"),
            ]
        );
    }
}
