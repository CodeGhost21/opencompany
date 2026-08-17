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
use crate::company::paypal::{
    CLIENT_ID_SECRET, CLIENT_SECRET_SECRET, ENVIRONMENT_SECRET, PaypalEnvironment,
};
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

/// The non-secret view of a company's PayPal connection (issue #789).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PaypalStatus {
    /// Whether a client id is stored. Never the id itself — it is half a
    /// credential, and there is no reason to render it back.
    pub client_id_configured: bool,
    /// Whether a client secret is stored.
    pub client_secret_configured: bool,
    /// `sandbox` or `live`. Shown, because "Connected" against the wrong world
    /// is the confusion this exists to avoid.
    pub environment: String,
    /// Whether this company's manifest explicitly grants `paypal`.
    pub granted: bool,
    /// Whether the `paypal` feature is compiled into this build.
    pub in_build: bool,
}

/// Builds the billing configuration routes.
pub fn router() -> Router<AppState> {
    scoped("/billing/chargebee", get(get_billing).put(put_billing))
        .merge(scoped("/billing/chargebee/key", delete(delete_billing)))
        .merge(scoped("/billing/paypal", get(get_paypal).put(put_paypal)))
        .merge(scoped("/billing/paypal/key", delete(delete_paypal)))
}

/// Assembles the non-secret PayPal status.
async fn paypal_status_of(runtime: &CompanyRuntime) -> Result<PaypalStatus, ApiError> {
    let granted = runtime
        .store()
        .load(runtime.id())
        .await
        .ok()
        .flatten()
        .map(|record| crate::company::grants_paypal_explicit(&record.manifest.tools.allow))
        .unwrap_or(false);
    let environment = read(runtime, ENVIRONMENT_SECRET)
        .await?
        .map(|raw| PaypalEnvironment::parse(&raw))
        .unwrap_or_default();
    Ok(PaypalStatus {
        client_id_configured: read(runtime, CLIENT_ID_SECRET).await?.is_some(),
        client_secret_configured: read(runtime, CLIENT_SECRET_SECRET).await?.is_some(),
        environment: environment.as_str().to_string(),
        granted,
        in_build: cfg!(feature = "paypal"),
    })
}

/// `GET …/billing/paypal` — non-secret status only.
async fn get_paypal(company: ScopedCompany) -> Result<Json<PaypalStatus>, ApiError> {
    Ok(Json(paypal_status_of(&company.runtime).await?))
}

/// The write-only PayPal config body.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaypalConfigBody {
    /// REST app client id (write-only). Omit to leave unchanged.
    #[serde(default)]
    client_id: Option<String>,
    /// REST app secret (write-only). Omit to leave unchanged.
    #[serde(default)]
    client_secret: Option<String>,
    /// `sandbox` or `live`. Anything unrecognised stores `sandbox`.
    #[serde(default)]
    environment: Option<String>,
}

/// `PUT …/billing/paypal` — store any supplied credentials, return status.
///
/// Admin-only, like its Chargebee sibling: pointing a company at a different
/// PayPal account changes whose wallet its agents can read.
async fn put_paypal(
    company: AdminScopedCompany,
    Json(body): Json<PaypalConfigBody>,
) -> Result<Json<PaypalStatus>, ApiError> {
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
    write(CLIENT_ID_SECRET, body.client_id.as_deref()).await?;
    write(CLIENT_SECRET_SECRET, body.client_secret.as_deref()).await?;
    if let Some(raw) = body.environment.as_deref() {
        // Normalised through the same parser the client uses, so an unrecognised
        // value is stored as `sandbox` rather than kept verbatim and re-parsed
        // differently somewhere else later.
        runtime
            .secrets()
            .set(
                runtime.id(),
                ENVIRONMENT_SECRET,
                SecretValue(PaypalEnvironment::parse(raw).as_str().to_string()),
            )
            .await?;
    }
    Ok(Json(paypal_status_of(runtime).await?))
}

/// `DELETE …/billing/paypal/key` — clear the stored PayPal credentials.
///
/// The environment is cleared too, so a re-connect starts from the safe default
/// rather than silently inheriting `live` from a previous account.
async fn delete_paypal(company: AdminScopedCompany) -> Result<Json<PaypalStatus>, ApiError> {
    let runtime = &company.runtime;
    for key in [CLIENT_ID_SECRET, CLIENT_SECRET_SECRET, ENVIRONMENT_SECRET] {
        runtime
            .secrets()
            .set(runtime.id(), key, SecretValue(String::new()))
            .await?;
    }
    Ok(Json(paypal_status_of(runtime).await?))
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

    // --- The routes, end to end -------------------------------------------
    //
    // The unit tests below cover the helpers. These drive the real router,
    // because the properties worth holding are route-level: that a credential
    // goes in and never comes back out, that clearing clears ALL of it, and
    // that a member cannot read or write another role's billing settings.

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn state_with_company(home: &std::path::Path) -> AppState {
        use crate::ports::CompanyStore;
        use crate::ports::types::CompanyRecord;

        let id = CompanyId::new("acme");
        let manifest: crate::company::CompanyManifest = ::toml::from_str(
            "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
        )
        .expect("manifest");
        crate::store::FsCompanyStore::new(home.to_path_buf())
            .save(&CompanyRecord {
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desk_tools: Default::default(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                overlay_policy: None,
                disabled_workflows: Vec::new(),
                template_provenance: None,
            })
            .await
            .expect("save");

        let runtime = crate::runtime::RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .expect("runtime");
        let state = AppState::new(crate::AppConfig::default());
        state.registry().insert(id, std::sync::Arc::new(runtime));
        state
    }

    async fn call(
        state: &AppState,
        method: &str,
        uri: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        // `seed_admin` / `seed_session` hand back a ready `Cookie` header value —
        // these routes authenticate a signed-in human, not a bearer token.
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", cookie);
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .expect("request");
        let response = crate::server::router(state.clone())
            .oneshot(request)
            .await
            .expect("routed");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn a_saved_credential_is_reported_as_configured_and_never_returned() {
        let home = ::tempfile::tempdir().expect("tempdir");
        let state = state_with_company(home.path()).await;
        let admin = crate::server::test_support::seed_admin(&state, "acme").await;

        // Nothing stored yet.
        let (status, before) = call(
            &state,
            "GET",
            "/api/v1/companies/acme/billing/chargebee",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{before}");
        assert_eq!(before["apiKeyConfigured"], false);
        assert_eq!(before["webhookConfigured"], false);

        let (status, saved) = call(
            &state,
            "PUT",
            "/api/v1/companies/acme/billing/chargebee",
            &admin,
            Some(json!({
                "apiKey": "cb_live_supersecret",
                "site": "https://acme-test.chargebee.com",
                "webhookSecret": "cbuser:cbpass",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");

        // The whole contract of this surface: it reports WHETHER a credential
        // is stored, never what it is. A response that echoed the key back
        // would put it in the browser, the network log and any screen share.
        let (_, after) = call(
            &state,
            "GET",
            "/api/v1/companies/acme/billing/chargebee",
            &admin,
            None,
        )
        .await;
        assert_eq!(after["apiKeyConfigured"], true);
        assert_eq!(after["webhookConfigured"], true);
        // The site is the one NON-secret field, and comes back normalised.
        assert_eq!(after["site"], "acme-test");
        for rendered in [saved.to_string(), after.to_string()] {
            assert!(!rendered.contains("cb_live_supersecret"), "{rendered}");
            assert!(!rendered.contains("cbpass"), "{rendered}");
        }
    }

    #[tokio::test]
    async fn clearing_removes_the_webhook_secret_too_not_just_the_key() {
        // The route is named `…/key`, which reads as if it clears only the API
        // key — leaving a webhook credential behind would keep the endpoint
        // live while the UI reported the integration as cleared.
        let home = ::tempfile::tempdir().expect("tempdir");
        let state = state_with_company(home.path()).await;
        let admin = crate::server::test_support::seed_admin(&state, "acme").await;

        call(
            &state,
            "PUT",
            "/api/v1/companies/acme/billing/chargebee",
            &admin,
            Some(json!({
                "apiKey": "cb_key",
                "site": "acme-test",
                "webhookSecret": "cbuser:cbpass",
            })),
        )
        .await;
        let (status, cleared) = call(
            &state,
            "DELETE",
            "/api/v1/companies/acme/billing/chargebee/key",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cleared}");
        assert_eq!(cleared["apiKeyConfigured"], false);
        assert_eq!(cleared["webhookConfigured"], false, "{cleared}");
        assert_eq!(
            cleared["site"],
            Value::Null,
            "the site is cleared as well: {cleared}"
        );
    }

    #[tokio::test]
    async fn paypal_clears_its_environment_so_a_reconnect_starts_at_sandbox() {
        // Inheriting `live` from a previous account is the failure worth
        // preventing here: the next connection would read real money.
        let home = ::tempfile::tempdir().expect("tempdir");
        let state = state_with_company(home.path()).await;
        let admin = crate::server::test_support::seed_admin(&state, "acme").await;

        call(
            &state,
            "PUT",
            "/api/v1/companies/acme/billing/paypal",
            &admin,
            Some(json!({
                "clientId": "AY_id",
                "clientSecret": "EL_secret",
                "environment": "live",
            })),
        )
        .await;
        let (_, live) = call(
            &state,
            "GET",
            "/api/v1/companies/acme/billing/paypal",
            &admin,
            None,
        )
        .await;
        assert_eq!(live["environment"], "live");
        assert_eq!(live["clientSecretConfigured"], true);
        assert!(!live.to_string().contains("EL_secret"), "{live}");

        let (status, cleared) = call(
            &state,
            "DELETE",
            "/api/v1/companies/acme/billing/paypal/key",
            &admin,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cleared}");
        assert_eq!(cleared["clientIdConfigured"], false);
        assert_eq!(cleared["clientSecretConfigured"], false);
        assert_eq!(cleared["environment"], "sandbox", "{cleared}");
    }

    #[tokio::test]
    async fn a_member_may_read_the_status_but_never_write_a_credential() {
        // The split is deliberate. `GET` carries no secret — booleans, the site
        // slug, the webhook URL — so a member seeing "not connected" is how they
        // know to ask an admin. Writing is another matter: a member who could
        // `PUT` here would point the company's invoicing at a Chargebee site
        // they control, and one who could `DELETE` could silently stop every
        // payment notification.
        let home = ::tempfile::tempdir().expect("tempdir");
        let state = state_with_company(home.path()).await;
        let member = crate::server::test_support::seed_session(
            &state,
            "acme",
            crate::ports::users::UserRole::Member,
        )
        .await;

        for uri in [
            "/api/v1/companies/acme/billing/chargebee",
            "/api/v1/companies/acme/billing/paypal",
        ] {
            let (status, answer) = call(&state, "GET", uri, &member, None).await;
            assert_eq!(status, StatusCode::OK, "GET {uri}: {answer}");
        }

        for (method, uri, body) in [
            (
                "PUT",
                "/api/v1/companies/acme/billing/chargebee",
                Some(json!({"apiKey": "cb_key", "site": "attacker-site"})),
            ),
            (
                "PUT",
                "/api/v1/companies/acme/billing/paypal",
                Some(json!({"clientId": "AY_id", "clientSecret": "EL_secret"})),
            ),
            (
                "DELETE",
                "/api/v1/companies/acme/billing/chargebee/key",
                None,
            ),
            ("DELETE", "/api/v1/companies/acme/billing/paypal/key", None),
        ] {
            let (status, answer) = call(&state, method, uri, &member, body).await;
            assert!(
                status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED,
                "{method} {uri} answered {status}: {answer}"
            );
        }

        // And nothing the member attempted was written. Read the store
        // directly rather than through another principal: the refusals above
        // are only worth having if they refused the WRITE, not merely the
        // response.
        let runtime = state
            .registry()
            .get(&CompanyId::new("acme"))
            .expect("company");
        for key in [
            API_KEY_SECRET,
            SITE_SECRET,
            WEBHOOK_SECRET_KEY,
            CLIENT_ID_SECRET,
            CLIENT_SECRET_SECRET,
            ENVIRONMENT_SECRET,
        ] {
            let stored = runtime
                .secrets()
                .get(runtime.id(), key)
                .await
                .expect("read secret");
            assert!(stored.is_none(), "{key} was written by a member");
        }
    }

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
    fn a_paypal_status_never_serializes_a_credential() {
        let status = PaypalStatus {
            client_id_configured: true,
            client_secret_configured: true,
            environment: "sandbox".to_string(),
            granted: true,
            in_build: true,
        };
        let json = serde_json::to_string(&status).expect("serializes");
        assert!(json.contains("clientIdConfigured"));
        assert!(json.contains("sandbox"));
        // No field may carry either half of the credential itself.
        assert!(!json.contains("clientId\""), "{json}");
        assert!(!json.contains("clientSecret\""), "{json}");
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
