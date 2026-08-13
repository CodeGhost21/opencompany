//! The Chargebee billing configuration write-plane (issue #788, UI in #527):
//! store the API key, the site identifier and the webhook credential — all
//! **write-only** — and surface the webhook URL an operator pastes into
//! Chargebee.
//!
//! `GET …/billing/chargebee` returns only [`BillingStatus`], which carries
//! booleans and the site slug. The API key and the webhook credential are never
//! serialized into any response, by construction: they live in
//! [`SecretStore`](crate::ports::SecretStore) and this module reads them back
//! only to *use* them, never to echo them.
//!
//! # Why the site identifier is a secret too
//!
//! It is not confidential, and it *is* returned by `GET` — a settings form has
//! to show what it is configured against, and "Connected ✓" beside the wrong
//! site is exactly the confusion this avoids. It shares the secret store with
//! the key only because the pair is meaningless apart: the tools need both or
//! neither, so keeping them in one place makes "half configured" impossible to
//! express by accident.
//!
//! # Three things can each be missing, and they fail differently
//!
//! [`BillingStatus`] reports them separately rather than as one "connected"
//! flag, because the remedies differ: no key or site means the agent has no
//! billing tools at all; no webhook credential means the tools work but nobody
//! is told when a customer pays; and a missing `chargebee` grant means both are
//! configured and still nothing reaches an agent. A single boolean would send an
//! operator looking in the wrong place for two of those three.

use axum::extract::State;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::billing::{API_KEY_SECRET, SITE_SECRET, WEBHOOK_SECRET_KEY};
use crate::company::runtime::CompanyRuntime;
use crate::ports::types::{CompanyId, SecretValue};
use crate::server::error::ApiError;
use crate::server::ops::scope::{AdminScopedCompany, ScopedCompany, scoped};

/// The non-secret view of a company's Chargebee configuration.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BillingStatus {
    /// Whether an API key is stored and non-empty. Never the key itself.
    pub api_key_configured: bool,
    /// The Chargebee site slug, when set — shown so a settings form can say
    /// *which* site it is connected to rather than only that it is.
    pub site: Option<String>,
    /// Whether a webhook credential is stored, i.e. whether a delivery from
    /// Chargebee could be verified at all.
    pub webhook_configured: bool,
    /// The URL to paste into Chargebee's webhook settings. `None` on a host with
    /// no publicly reachable base URL — Chargebee cannot deliver to a loopback
    /// address, and showing one would send an operator to configure a webhook
    /// that silently never arrives (the shape of issue #203).
    pub webhook_url: Option<String>,
    /// Whether this company's manifest **explicitly** grants `chargebee`.
    /// Both credentials can be present and still wire no tools without it.
    pub granted: bool,
    /// Whether the `chargebee` feature is compiled into this build at all.
    pub in_build: bool,
}

/// Builds the billing configuration routes.
pub fn router() -> Router<AppState> {
    scoped("/billing/chargebee", get(get_billing).put(put_billing))
        .merge(scoped("/billing/chargebee/key", delete(delete_billing)))
}

/// The webhook URL for `company`, or `None` when this host has no publicly
/// reachable base URL.
///
/// Deliberately the same source as the telegram channel's — not the bind
/// address, which yields a `http://127.0.0.1:<port>/…` URL that is
/// syntactically fine and undeliverable in practice.
fn webhook_url(state: &AppState, company: &CompanyId) -> Option<String> {
    let base = state.config().public_webhook_base_url()?;
    Some(format!("{base}/hooks/{}/chargebee", company.as_ref()))
}

/// Reads a stored secret, treating empty as absent.
async fn read(runtime: &CompanyRuntime, key: &str) -> Result<Option<String>, ApiError> {
    Ok(runtime
        .secrets()
        .get(runtime.id(), key)
        .await?
        .map(|value| value.expose().to_string())
        .filter(|value| !value.trim().is_empty()))
}

/// Assembles the non-secret status.
async fn status_of(state: &AppState, runtime: &CompanyRuntime) -> Result<BillingStatus, ApiError> {
    // The grant lives in the stored manifest, not on the runtime handle. A
    // company that cannot be loaded reports `granted: false` rather than
    // failing the whole status: the operator still needs to see what IS
    // configured, and a settings page that 500s tells them nothing.
    let granted = runtime
        .store()
        .load(runtime.id())
        .await
        .ok()
        .flatten()
        .map(|record| crate::company::grants_chargebee_explicit(&record.manifest.tools.allow))
        .unwrap_or(false);
    Ok(BillingStatus {
        api_key_configured: read(runtime, API_KEY_SECRET).await?.is_some(),
        site: read(runtime, SITE_SECRET).await?,
        webhook_configured: read(runtime, WEBHOOK_SECRET_KEY).await?.is_some(),
        webhook_url: webhook_url(state, runtime.id()),
        granted,
        in_build: cfg!(feature = "chargebee"),
    })
}

/// `GET …/billing/chargebee` — non-secret status only.
async fn get_billing(
    company: ScopedCompany,
    State(state): State<AppState>,
) -> Result<Json<BillingStatus>, ApiError> {
    Ok(Json(status_of(&state, &company.runtime).await?))
}

/// The write-only config body. Every field is optional; only fields present and
/// non-empty are applied, so the site can be corrected without re-entering the
/// key.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BillingConfigBody {
    /// The Chargebee API key (write-only). Omit to leave it unchanged.
    #[serde(default)]
    api_key: Option<String>,
    /// The site identifier — the `acme-test` in `acme-test.chargebee.com`.
    #[serde(default)]
    site: Option<String>,
    /// The `username:password` pair Chargebee is configured to present on its
    /// webhook deliveries (write-only). Omit to leave it unchanged.
    #[serde(default)]
    webhook_secret: Option<String>,
}

/// Normalises a site identifier an operator may paste in several shapes.
///
/// `acme-test`, `acme-test.chargebee.com` and `https://acme-test.chargebee.com/`
/// all mean the same site, and all three are what somebody actually pastes out
/// of a browser address bar. Storing the second or third produces a base URL of
/// `https://acme-test.chargebee.com.chargebee.com/api/v2`, whose failure names
/// DNS rather than the typo.
fn normalize_site(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

/// `PUT …/billing/chargebee` — store any supplied credentials, return status.
///
/// Requires authority over the company: this key can raise invoices against
/// real customers in the company's name, so pointing it at a different
/// Chargebee site is not an ordinary member's edit.
async fn put_billing(
    company: AdminScopedCompany,
    State(state): State<AppState>,
    Json(body): Json<BillingConfigBody>,
) -> Result<Json<BillingStatus>, ApiError> {
    let runtime = &company.runtime;
    let write = async |key: &str, value: Option<&str>| -> Result<(), ApiError> {
        if let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) {
            runtime
                .secrets()
                .set(runtime.id(), key, SecretValue(value.to_string()))
                .await?;
        }
        Ok(())
    };
    write(API_KEY_SECRET, body.api_key.as_deref()).await?;
    write(WEBHOOK_SECRET_KEY, body.webhook_secret.as_deref()).await?;

    if let Some(site) = body
        .site
        .as_deref()
        .map(normalize_site)
        .filter(|s| !s.is_empty())
    {
        runtime
            .secrets()
            .set(runtime.id(), SITE_SECRET, SecretValue(site))
            .await?;
    }

    Ok(Json(status_of(&state, runtime).await?))
}

/// `DELETE …/billing/chargebee/key` — clear every stored credential.
///
/// The [`SecretStore`](crate::ports::SecretStore) port has no delete, so a
/// cleared credential is stored as the empty string; every read site treats an
/// empty value as unset (the tools fail closed, the webhook rejects).
async fn delete_billing(
    company: AdminScopedCompany,
    State(state): State<AppState>,
) -> Result<Json<BillingStatus>, ApiError> {
    let runtime = &company.runtime;
    for key in [API_KEY_SECRET, SITE_SECRET, WEBHOOK_SECRET_KEY] {
        runtime
            .secrets()
            .set(runtime.id(), key, SecretValue(String::new()))
            .await?;
    }
    Ok(Json(status_of(&state, runtime).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_is_normalized_from_every_shape_an_operator_pastes() {
        for raw in [
            "acme-test",
            " acme-test ",
            "acme-test.chargebee.com",
            "https://acme-test.chargebee.com",
            "https://acme-test.chargebee.com/",
            "http://acme-test.chargebee.com/",
        ] {
            assert_eq!(normalize_site(raw), "acme-test", "from {raw:?}");
        }
    }

    #[test]
    fn an_empty_site_stays_empty_rather_than_becoming_a_url_fragment() {
        assert_eq!(normalize_site(""), "");
        assert_eq!(normalize_site("   "), "");
        assert_eq!(normalize_site("https://"), "");
    }

    #[test]
    fn status_never_serializes_a_credential() {
        // The whole contract of this module: whatever else changes, no field
        // here may carry the key. Asserted on the serialized form, because that
        // is what actually reaches a browser.
        let status = BillingStatus {
            api_key_configured: true,
            site: Some("acme-test".to_string()),
            webhook_configured: true,
            webhook_url: Some("https://oc.example/hooks/acme/chargebee".to_string()),
            granted: true,
            in_build: true,
        };
        let json = serde_json::to_string(&status).expect("serializes");
        assert!(json.contains("apiKeyConfigured"));
        assert!(json.contains("acme-test"));
        // No field may be named in a way that could carry the secret itself.
        assert!(!json.contains("apiKey\""), "{json}");
        assert!(!json.contains("webhookSecret"), "{json}");
    }

    #[test]
    fn the_three_failure_modes_stay_distinguishable() {
        // Credentials present, grant missing: the operator's remedy is the
        // manifest, not the settings form. Collapsing these into one
        // "connected" boolean is what sends them to the wrong place.
        let configured_but_ungranted = BillingStatus {
            api_key_configured: true,
            site: Some("acme-test".to_string()),
            webhook_configured: false,
            webhook_url: None,
            granted: false,
            in_build: true,
        };
        assert!(configured_but_ungranted.api_key_configured);
        assert!(!configured_but_ungranted.granted);
        assert!(!configured_but_ungranted.webhook_configured);
    }
}
