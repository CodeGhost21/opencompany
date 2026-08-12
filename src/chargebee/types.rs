//! Configuration and tool-input types for the Chargebee MCP server.
//!
//! Every money-carrying field in this module is named `*_in_minor_units` and
//! typed `i64` on purpose. Chargebee's API is minor-unit based (`amount = 50000`
//! is $500.00 for a two-decimal currency), and an agent filling a field called
//! plain `amount` from the prompt "invoice Acme for $500" will write `500` —
//! a $5.00 invoice that looks entirely successful. The unit lives in the field
//! name so the mistake has to be made deliberately.

use serde::{Deserialize, Serialize};

/// Connection settings for a Chargebee site.
///
/// The API key is read from the process environment by
/// [`ChargebeeConfig::from_env`] and never appears in a tool argument, a tool
/// result, or the MCP manifest — issue #788 requires it be injected at runtime
/// and never hardcoded.
#[derive(Clone, Debug)]
pub struct ChargebeeConfig {
    /// The Chargebee site slug, i.e. the `acme` in `acme.chargebee.com`.
    pub site: String,
    /// The site's API key, sent as the HTTP Basic username.
    pub api_key: String,
}

impl ChargebeeConfig {
    /// Reads `CHARGEBEE_SITE` and `CHARGEBEE_API_KEY` from the environment.
    ///
    /// Returns the names of the missing variables rather than a generic error
    /// so an operator sees which half of the pair they forgot.
    pub fn from_env() -> std::result::Result<Self, Vec<&'static str>> {
        let site = std::env::var("CHARGEBEE_SITE")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let api_key = std::env::var("CHARGEBEE_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let mut missing = Vec::new();
        if site.is_none() {
            missing.push("CHARGEBEE_SITE");
        }
        if api_key.is_none() {
            missing.push("CHARGEBEE_API_KEY");
        }
        if !missing.is_empty() {
            return Err(missing);
        }

        Ok(Self {
            // Both are Some by construction — the checks above returned otherwise.
            site: site.expect("site checked above").trim().to_string(),
            api_key: api_key.expect("api key checked above").trim().to_string(),
        })
    }

    /// The API v2 base URL for this site, without a trailing slash.
    pub fn base_url(&self) -> String {
        format!("https://{}.chargebee.com/api/v2", self.site)
    }
}

/// One ad-hoc line item on an invoice.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChargeLine {
    /// Line-item text shown on the invoice.
    pub description: String,
    /// The charge amount in the currency's minor unit (cents for USD).
    /// Chargebee rejects anything below 1.
    pub amount_in_minor_units: i64,
}

/// Arguments for `chargebee_create_invoice`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateInvoiceArgs {
    /// An existing Chargebee customer id. Create the customer first if unknown.
    pub customer_id: String,
    /// ISO 4217 code, e.g. `USD`. Required: Chargebee will not infer it for a
    /// site with more than one enabled currency, and inferring it here would
    /// mean guessing what money the user meant.
    pub currency_code: String,
    /// At least one line item.
    pub charges: Vec<ChargeLine>,
    /// Days until the invoice is due.
    #[serde(default)]
    pub net_term_days: Option<i64>,
    /// Free-text note stored on the invoice.
    #[serde(default)]
    pub invoice_note: Option<String>,
    /// Whether Chargebee should immediately charge the customer's stored card.
    /// `"off"` (the default here) raises an unpaid invoice to be settled later;
    /// `"on"` attempts collection at once. See [`default_auto_collection`].
    #[serde(default = "default_auto_collection")]
    pub auto_collection: String,
    /// Optional `chargebee-idempotency-key`. A retried agent turn that reuses
    /// this key gets the original invoice back instead of billing twice.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// `"off"` — invoices are raised unpaid unless the caller says otherwise.
///
/// This deliberately departs from Chargebee, whose default follows the
/// customer's `auto_collection` and therefore charges a stored card the moment
/// the invoice exists. Two reasons to override it here:
///
/// - **"Create an invoice" is not "take a payment."** An agent acting on a
///   natural-language billing instruction should not move money as a side
///   effect of a word the user did not say. Charging a card is the kind of
///   action that has to be asked for.
/// - **Otherwise `chargebee_record_payment` is unreachable.** Issue #788 scopes
///   both operations; an invoice that auto-collects on creation is already paid,
///   so the tool that records a payment against it could never apply.
///
/// A caller who does want immediate collection passes `auto_collection: "on"`.
fn default_auto_collection() -> String {
    "off".to_string()
}

/// The values Chargebee accepts for `auto_collection`.
pub const AUTO_COLLECTION: &[&str] = &["on", "off"];

/// Arguments for `chargebee_list_invoices`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ListInvoicesArgs {
    /// Restrict to one customer.
    #[serde(default)]
    pub customer_id: Option<String>,
    /// Chargebee invoice status: `paid`, `posted`, `payment_due`, `not_paid`,
    /// `voided`, or `pending`.
    #[serde(default)]
    pub status: Option<String>,
    /// Unix timestamp; returns invoices dated on or after it.
    #[serde(default)]
    pub invoice_date_after: Option<i64>,
    /// Unix timestamp; returns invoices dated on or before it.
    #[serde(default)]
    pub invoice_date_before: Option<i64>,
    /// Page size, 1-100. Chargebee's own default is 10.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Arguments for `chargebee_get_subscription`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GetSubscriptionArgs {
    /// The Chargebee subscription id.
    pub subscription_id: String,
}

/// Arguments for `chargebee_upsert_customer`.
///
/// One tool rather than separate create/update calls: the agent is working from
/// a name in a prompt and rarely knows whether the record already exists.
/// Passing `id` updates that customer; omitting it creates a new one.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UpsertCustomerArgs {
    /// Existing customer id to update. Omit to create.
    #[serde(default)]
    pub id: Option<String>,
    /// Given name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Family name.
    #[serde(default)]
    pub last_name: Option<String>,
    /// Billing email.
    #[serde(default)]
    pub email: Option<String>,
    /// Company name, e.g. `Acme Corp`.
    #[serde(default)]
    pub company: Option<String>,
    /// Default days until invoices for this customer are due.
    #[serde(default)]
    pub net_term_days: Option<i64>,
}

/// Arguments for `chargebee_record_payment`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecordPaymentArgs {
    /// The invoice being paid.
    pub invoice_id: String,
    /// Amount received, in the currency's minor unit.
    pub amount_in_minor_units: i64,
    /// One of Chargebee's offline payment methods. See [`PAYMENT_METHODS`].
    pub payment_method: String,
    /// Cheque number, wire reference, etc.
    #[serde(default)]
    pub reference_number: Option<String>,
    /// Free-text note stored against the payment.
    #[serde(default)]
    pub comment: Option<String>,
    /// Optional `chargebee-idempotency-key`, as on invoice creation.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// The offline payment methods `record_payment` accepts, taken from the
/// `transaction[payment_method]` enum in Chargebee's OpenAPI spec.
///
/// Recorded here so a bad value is rejected locally with the valid set in the
/// message, rather than costing a network round trip to learn the same thing.
pub const PAYMENT_METHODS: &[&str] = &[
    "cash",
    "check",
    "bank_transfer",
    "other",
    "custom",
    "dana",
    "touch_n_go",
    "tamara",
    "qpay",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_the_site_api_v2_root() {
        let cfg = ChargebeeConfig {
            site: "acme-test".to_string(),
            api_key: "cb_test_key".to_string(),
        };
        assert_eq!(cfg.base_url(), "https://acme-test.chargebee.com/api/v2");
    }

    #[test]
    fn charge_line_names_its_unit_in_the_field() {
        // The whole point of the naming convention: deserializing a bare
        // `amount` must not silently populate the minor-unit field.
        let err = serde_json::from_str::<ChargeLine>(r#"{"description":"Pro plan","amount":500}"#);
        assert!(err.is_err(), "a bare `amount` must not satisfy ChargeLine");

        let ok: ChargeLine =
            serde_json::from_str(r#"{"description":"Pro plan","amount_in_minor_units":50000}"#)
                .expect("explicit minor units parse");
        assert_eq!(ok.amount_in_minor_units, 50_000);
    }

    #[test]
    fn optional_invoice_fields_default_to_none() {
        let args: CreateInvoiceArgs = serde_json::from_str(
            r#"{"customer_id":"acme","currency_code":"USD",
                "charges":[{"description":"Pro plan","amount_in_minor_units":50000}]}"#,
        )
        .expect("minimal args parse");
        assert_eq!(args.net_term_days, None);
        assert_eq!(args.invoice_note, None);
        assert_eq!(args.idempotency_key, None);
        assert_eq!(args.charges.len(), 1);
    }
}
