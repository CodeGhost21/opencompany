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

use serde::Serialize;

use crate::Result;
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

/// Whether a non-empty per-tenant token is stored — never the token itself.
pub async fn token_configured(company: &CompanyId, secrets: &dyn SecretStore) -> Result<bool> {
    Ok(secrets
        .get(company, TOKEN_KEY)
        .await?
        .map(|SecretValue(token)| !token.trim().is_empty())
        .unwrap_or(false))
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
}
