//! Per-tenant Composio credential + backend routing (issue #110, epic #26 Cell
//! D). Always compiled (so the console read/write plane can manage the token
//! even in the default build); the live agent tools that consume it live in the
//! feature-gated [`harness::composio`](crate::harness::composio).
//!
//! The per-tenant OAuth bearer token is **write-only**: it is set through the
//! console `PUT …/composio/token` route, stored under [`TOKEN_KEY`], and never
//! returned. The read shape carries only a `tokenConfigured` boolean. The token
//! has **no environment fallback** — a missing token means no tools (fail
//! closed), never a borrowed identity. Only the backend URL may be overridden
//! from the environment.

use std::sync::Arc;

use serde::Serialize;

use crate::Result;
use crate::company::company_key;
use crate::company::credentials::{Credential, TinyhumansTokenSource};
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

/// The canonical per-company Composio credential key. The per-tenant OAuth
/// bearer token is stored here (write-only via the console); the value is the
/// raw token string.
pub const TOKEN_KEY: &str = "composio/token";

/// The explicit environment override for the Composio backend URL. Only the
/// **URL** has an env path — the **token** deliberately does not (fail-closed
/// isolation). When unset, resolution falls back to the tenant's shared API
/// base ([`TINYHUMANS_API_URL_ENV`]) so staging Composio follows staging.
pub const COMPOSIO_BACKEND_URL_ENV: &str = "OPENCOMPANY_COMPOSIO_BACKEND_URL";

/// The tenant's shared TinyHumans API base URL (the same backend inference and
/// the rest of the app already use). Used as the Composio backend fallback when
/// [`COMPOSIO_BACKEND_URL_ENV`] is unset, so a staging tenant's Composio calls
/// go to staging instead of the hardcoded prod default.
pub const TINYHUMANS_API_URL_ENV: &str = "TINYHUMANS_API_URL";

/// Default backend base URL for the Composio routes when neither the explicit
/// override nor the tenant API base is set. Mirrors the media backend's default
/// host (prod).
pub const DEFAULT_BACKEND_URL: &str = "https://api.tinyhumans.ai";

/// The effective Composio backend URL, resolved in this order (first non-empty,
/// trimmed, wins):
///
/// 1. `env_override` — [`COMPOSIO_BACKEND_URL_ENV`], the explicit override.
/// 2. `api_url` — [`TINYHUMANS_API_URL_ENV`], the tenant's shared backend base,
///    so Composio follows staging/prod with the rest of the app.
/// 3. [`DEFAULT_BACKEND_URL`] (prod) — last resort.
///
/// Credential-free — safe to surface on the console read plane.
pub fn backend_url_or_default(env_override: Option<String>, api_url: Option<String>) -> String {
    [env_override, api_url]
        .into_iter()
        .flatten()
        .map(|u| u.trim().to_string())
        .find(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_BACKEND_URL.to_string())
}

/// Store (or rotate/clear) the per-tenant Composio token. A non-empty value
/// rotates it; an empty string clears it. Write-only — the value is never read
/// back over the API.
pub async fn store_token(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    token: &str,
) -> Result<()> {
    secrets
        .set(company, TOKEN_KEY, SecretValue(token.trim().to_string()))
        .await
}

/// The credential this company's Composio calls present, or [`Credential::None`]
/// when none can be obtained at all.
///
/// The **one** derivation of that answer, and the reason it lives here rather
/// than in the feature-gated harness: both callers need it in every build.
/// [`TenantComposio::resolve`](crate::harness::composio::TenantComposio::resolve)
/// builds the agent-facing config from it, and the console status route
/// ([`ops::composio`](crate::server::ops::composio)) reports its
/// [`source`](Credential::source) — so the tier the console shows an operator
/// cannot disagree with the identity the agents actually present. Two functions
/// that merely *mirrored* each other's precedence would drift the first time a
/// tier was added to one of them, which is exactly the failure issue #586 exists
/// to remove.
///
/// Precedence: the company's own Composio token ([`TOKEN_KEY`], the BYO escape
/// hatch) wins; otherwise the shared brokered-credential seam
/// [`company_key::resolve`] answers — the company's own TinyHumans key, else this
/// instance's platform identity, else nothing.
///
/// A store read error **propagates** rather than degrading to the next tier —
/// see [`company_key::resolve`] for why an unreadable store must not silently
/// change which account a call is attributed to.
pub async fn resolve_credential(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    token_source: Option<Arc<TinyhumansTokenSource>>,
) -> Result<Credential> {
    let byo = match secrets.get(company, TOKEN_KEY).await? {
        Some(SecretValue(token)) => Credential::from_value(token),
        None => Credential::None,
    };
    Ok(match byo {
        // The company's own Composio token always wins.
        byo @ Credential::Value(_) => byo,
        // Everything else is the shared seam's answer, so a rotated company key
        // reaches Composio the same cycle it reaches every other brokered
        // surface.
        _ => company_key::resolve(company, secrets, token_source).await?,
    })
}

/// Whether a non-empty per-tenant token is stored — never the token itself.
pub async fn token_configured(company: &CompanyId, secrets: &dyn SecretStore) -> Result<bool> {
    Ok(secrets
        .get(company, TOKEN_KEY)
        .await?
        .map(|SecretValue(token)| !token.trim().is_empty())
        .unwrap_or(false))
}

/// The [`SecretStore`] key holding this company's per-toolkit default
/// connections — a JSON object `{"gmail": "ca_123"}` written by the console
/// (issue #820).
///
/// Stored the way `inference/config` is: one small JSON blob per company,
/// alongside the credential it qualifies, rather than a new port. It is a
/// *preference*, not a secret — the ids in it are already handed to the console
/// by `GET …/composio/connections`, and are useless without the bearer that
/// scopes them. It lives in the secret store because that is the one per-company
/// key/value plane this repo has, and because keeping it beside [`TOKEN_KEY`]
/// means a company's Composio state moves, backs up and is deleted as one thing.
pub const DEFAULTS_KEY: &str = "composio/defaults";

/// This company's chosen connection per toolkit: `gmail` → a Composio connection
/// id (issue #820).
///
/// Absent for a toolkit means **no company has expressed an intent**, and the
/// execute path then sends no connection id at all, leaving the resolution to
/// Composio exactly as before. That absence is the ordinary case and is not a
/// degraded one — one account per toolkit needs no choice — so nothing here
/// invents a default from the connection list. A default that the product does
/// not actually make would be a claim the harness could not honour, which is the
/// failure #820 was filed about.
pub type ComposioDefaults = std::collections::BTreeMap<String, String>;

/// This company's stored per-toolkit defaults, or an empty map.
///
/// A blob that will not parse is treated as *no defaults* rather than an error:
/// the only writer is [`set_default`] / [`clear_default`], so unparseable means
/// hand-edited or from a future shape, and the honest response on the agent path
/// is to fall back to Composio's own resolution rather than to withhold the
/// tools. It is logged, not swallowed silently.
pub async fn load_defaults(
    company: &CompanyId,
    secrets: &dyn SecretStore,
) -> Result<ComposioDefaults> {
    let Some(SecretValue(raw)) = secrets.get(company, DEFAULTS_KEY).await? else {
        return Ok(ComposioDefaults::new());
    };
    if raw.trim().is_empty() {
        return Ok(ComposioDefaults::new());
    }
    match serde_json::from_str::<ComposioDefaults>(&raw) {
        Ok(defaults) => Ok(defaults
            .into_iter()
            .map(|(toolkit, id)| (toolkit.trim().to_ascii_lowercase(), id.trim().to_string()))
            .filter(|(toolkit, id)| !toolkit.is_empty() && !id.is_empty())
            .collect()),
        Err(err) => {
            tracing::warn!(
                company = %company,
                error = %err,
                "[composio] stored connection defaults did not parse; treating this company as \
                 having expressed no preference"
            );
            Ok(ComposioDefaults::new())
        }
    }
}

/// Pin `toolkit` to `connection_id`, replacing whatever it named before, and
/// return the resulting map.
///
/// The caller is responsible for checking that the id names a connection this
/// company actually holds — see
/// [`set_default_connection`](crate::harness::composio::set_default_connection),
/// which is the only path the console reaches this through. Storing an id blind
/// would let a typo silently redirect every send for a toolkit to nothing.
pub async fn set_default(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    toolkit: &str,
    connection_id: &str,
) -> Result<ComposioDefaults> {
    let mut defaults = load_defaults(company, secrets).await?;
    defaults.insert(
        toolkit.trim().to_ascii_lowercase(),
        connection_id.trim().to_string(),
    );
    save_defaults(company, secrets, &defaults).await?;
    Ok(defaults)
}

/// Drop `toolkit`'s pin — back to letting Composio resolve the account — and
/// return the resulting map.
pub async fn clear_default(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    toolkit: &str,
) -> Result<ComposioDefaults> {
    let mut defaults = load_defaults(company, secrets).await?;
    defaults.remove(&toolkit.trim().to_ascii_lowercase());
    save_defaults(company, secrets, &defaults).await?;
    Ok(defaults)
}

/// Drop every pin naming `connection_id`, and report whether anything went.
///
/// Called when an account is revoked: a pin to a connection that no longer
/// exists would be sent on the next execute and refused by Composio, turning a
/// disconnect of the *other* account into a broken toolkit.
pub async fn forget_connection(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    connection_id: &str,
) -> Result<bool> {
    let connection_id = connection_id.trim();
    let mut defaults = load_defaults(company, secrets).await?;
    let before = defaults.len();
    defaults.retain(|_, id| id != connection_id);
    if defaults.len() == before {
        return Ok(false);
    }
    save_defaults(company, secrets, &defaults).await?;
    Ok(true)
}

async fn save_defaults(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    defaults: &ComposioDefaults,
) -> Result<()> {
    // An empty map is stored as an empty string rather than `{}`, matching how
    // every other value here is cleared: `SecretStore` has no delete, and the
    // loader already reads empty as "nothing pinned".
    let raw = if defaults.is_empty() {
        String::new()
    } else {
        serde_json::to_string(defaults).map_err(|err| {
            crate::error::OpenCompanyError::Store(format!(
                "could not serialize composio defaults: {err}"
            ))
        })?
    };
    secrets.set(company, DEFAULTS_KEY, SecretValue(raw)).await
}

/// One provider in the catalog the console renders, carrying the backend's own
/// display metadata rather than a bare slug (issue #600).
///
/// ## Why this lives here and not in the harness
///
/// It is produced by `harness::composio::list_catalog_toolkits` and consumed by
/// the always-compiled status route, and the harness compiles only under the
/// `openhuman` feature. Same reason [`TOKEN_KEY`] and
/// [`backend_url_or_default`] live here: the console plane must keep working in
/// a default build that links none of the live tools.
///
/// ## Why it is not `composio_catalog::CatalogToolkit`
///
/// That type describes the same backend entry for an *agent*, and it drops the
/// logo URL on purpose — a URL a model can never act on costs tokens to no end.
/// The logo and the categories are the entire point of this one: they are what
/// let 123 providers be a browsable grid instead of 123 stacked rows. It also
/// carries no connected flag, because the console learns that from
/// `GET …/composio/connections` — live per-company state, not catalog data.
///
/// Serialized straight into the status DTO, so these field names are the
/// console's wire contract.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    /// Toolkit slug, e.g. `googlecalendar`. The key every host call is made
    /// with, and the only field the backend always publishes.
    pub slug: String,
    /// Human-readable name, e.g. `Google Calendar`. Empty when the backend
    /// published none — the console then falls back to its own typography.
    pub name: String,
    /// One-line description. Empty when unpublished. The console searches it
    /// alongside the name, so an operator who knows what a provider *does* can
    /// find it without knowing what it is called.
    pub description: String,
    /// Composio-hosted logo URL. `None` when unpublished.
    pub logo: Option<String>,
    /// Composio's own category names, e.g. `["productivity", "email"]`.
    ///
    /// Forwarded **verbatim** and uninterpreted. The console buckets them by
    /// substring, and it does so precisely because that means a Composio
    /// integration added tomorrow lands in the right group with no code change
    /// on either side of this wire.
    pub categories: Vec<String>,
}

impl CatalogEntry {
    /// An entry for a provider the backend published a slug and nothing else
    /// for.
    ///
    /// Three real callers, so "no metadata" is a first-class state rather than
    /// a reason to drop the provider: a manifest allowlist (hand-written slugs,
    /// and the catalog is deliberately never consulted for it), the fallback
    /// list (which exists *because* the metadata could not be fetched), and a
    /// backend predating the dynamic catalog (which sends no `catalog[]` at
    /// all). The console renders all three with its own typography.
    pub fn from_slug(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_url_prefers_override_then_api_url_then_default() {
        // Neither set → prod default.
        assert_eq!(backend_url_or_default(None, None), DEFAULT_BACKEND_URL);

        // Explicit override wins over everything.
        assert_eq!(
            backend_url_or_default(
                Some("https://custom.example".into()),
                Some("https://staging-api.tinyhumans.ai".into())
            ),
            "https://custom.example"
        );

        // No override → follow the tenant API base (the staging case).
        assert_eq!(
            backend_url_or_default(None, Some("https://staging-api.tinyhumans.ai".into())),
            "https://staging-api.tinyhumans.ai"
        );

        // Whitespace/empty override falls through to the api_url fallback.
        assert_eq!(
            backend_url_or_default(
                Some("  ".into()),
                Some("https://staging-api.tinyhumans.ai".into())
            ),
            "https://staging-api.tinyhumans.ai"
        );

        // Whitespace/empty api_url falls through to the prod default.
        assert_eq!(
            backend_url_or_default(Some("".into()), Some("   ".into())),
            DEFAULT_BACKEND_URL
        );

        // api_url is trimmed before use.
        assert_eq!(
            backend_url_or_default(None, Some("  https://staging-api.tinyhumans.ai  ".into())),
            "https://staging-api.tinyhumans.ai"
        );
    }

    #[derive(Default)]
    struct MemSecrets {
        map: std::sync::Mutex<std::collections::HashMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl SecretStore for MemSecrets {
        async fn get(&self, _c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
            Ok(self
                .map
                .lock()
                .unwrap()
                .get(key)
                .map(|v| SecretValue(v.clone())))
        }
        async fn set(&self, _c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
            self.map.lock().unwrap().insert(key.to_string(), value.0);
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_company_with_no_stored_preference_pins_nothing() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        assert!(load_defaults(&company, &secrets).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_pin_round_trips_and_is_replaced_rather_than_appended() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();

        let after = set_default(&company, &secrets, "gmail", "ca_ops")
            .await
            .unwrap();
        assert_eq!(after.get("gmail").map(String::as_str), Some("ca_ops"));
        assert_eq!(
            load_defaults(&company, &secrets).await.unwrap(),
            after,
            "the stored blob is what the setter reported"
        );

        // A second toolkit is additive; naming gmail again replaces it.
        set_default(&company, &secrets, "slack", "ca_workspace")
            .await
            .unwrap();
        let after = set_default(&company, &secrets, "gmail", "ca_billing")
            .await
            .unwrap();
        assert_eq!(after.get("gmail").map(String::as_str), Some("ca_billing"));
        assert_eq!(
            after.get("slack").map(String::as_str),
            Some("ca_workspace"),
            "pinning one toolkit must not disturb another"
        );
    }

    #[tokio::test]
    async fn toolkits_are_normalized_so_a_pin_is_found_by_the_slug_prefix() {
        // `slug_toolkit` lowercases (`GMAIL_SEND_EMAIL` → `gmail`), so a pin
        // stored under `GMail` would be invisible to the execute path.
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        set_default(&company, &secrets, " GMail ", " ca_ops ")
            .await
            .unwrap();
        assert_eq!(
            load_defaults(&company, &secrets)
                .await
                .unwrap()
                .get("gmail")
                .map(String::as_str),
            Some("ca_ops")
        );
    }

    #[tokio::test]
    async fn clearing_a_pin_returns_the_toolkit_to_composios_own_resolution() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        set_default(&company, &secrets, "gmail", "ca_ops")
            .await
            .unwrap();
        let after = clear_default(&company, &secrets, "gmail").await.unwrap();
        assert!(after.is_empty());
        assert!(load_defaults(&company, &secrets).await.unwrap().is_empty());
        // Clearing what was never pinned is not an error.
        assert!(
            clear_default(&company, &secrets, "gmail")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn revoking_the_pinned_account_drops_the_pin() {
        // Otherwise the next execute sends an id Composio no longer knows, and
        // disconnecting the *other* account breaks the toolkit.
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        set_default(&company, &secrets, "gmail", "ca_ops")
            .await
            .unwrap();
        set_default(&company, &secrets, "slack", "ca_workspace")
            .await
            .unwrap();

        assert!(
            !forget_connection(&company, &secrets, "ca_unrelated")
                .await
                .unwrap(),
            "revoking an unpinned account changes nothing"
        );
        assert!(
            forget_connection(&company, &secrets, "ca_ops")
                .await
                .unwrap()
        );

        let left = load_defaults(&company, &secrets).await.unwrap();
        assert_eq!(left.get("slack").map(String::as_str), Some("ca_workspace"));
        assert!(!left.contains_key("gmail"));
    }

    #[tokio::test]
    async fn an_unparseable_blob_reads_as_no_preference() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        secrets
            .set(&company, DEFAULTS_KEY, SecretValue("not json".into()))
            .await
            .unwrap();
        assert!(
            load_defaults(&company, &secrets).await.unwrap().is_empty(),
            "a hand-edited blob must fall back to Composio's resolution, not withhold the tools"
        );
    }
}
