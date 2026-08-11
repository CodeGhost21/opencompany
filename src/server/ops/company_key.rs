//! The company's own TinyHumans credential, over HTTP (issue #586): read whether
//! one is set and which identity brokered calls currently present, and set /
//! rotate / clear it.
//!
//! The credential itself is [`company_key`](crate::company::company_key); this
//! is its console plane. **Write-only**: the key goes in through the `key`
//! field, lands in the secret store, and is never echoed — the read shape
//! carries only a `configured` boolean plus the non-secret tier name.
//!
//! **Admin-only** on the write, for the same reason
//! [`ops::composio`](super::composio)'s token write is: this key is the identity
//! every one of the company's agents presents to every surface the platform
//! brokers, and it is the company's wallet — whoever sets it decides which
//! account those agents act through and which account pays. That is a decision
//! made *for* the company, not a member's own, and [`AdminScopedCompany`] is
//! what says so in the signature.
//!
//! A set / rotate / clear takes effect on the agents' **next cycle** with no
//! restart: every brokered surface re-resolves through
//! [`company_key::resolve`](crate::company::company_key::resolve) and the roster
//! fingerprint moves with the key's value.

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::company_key::{key_configured, resolve, store_key};
use crate::company::credentials::CredentialSource;
use crate::company::runtime::CompanyRuntime;
use crate::ports::types::CompanyEvent;
use crate::server::error::ApiError;
use crate::server::ops::{AdminScopedCompany, ScopedCompany, scoped};

/// The reminder attached to every mutating response.
const SWITCH_NOTE: &str = "Agents present the new credential on their next cycle — no restart \
     needed. It reaches every surface the platform brokers at once.";

/// What an admin most needs to understand before pasting: this key is the
/// company's wallet, and membership in the company is what grants access to it.
const CONSEQUENCE: &str = "This is the company's TinyHumans credential. Every member's agents act and spend through \
     it, and every provider connected with it belongs to the company rather than to the person \
     who connected it. Spend arrives as one account, so it cannot be attributed per member.";

/// Said instead when nothing is configured and the instance carries no identity
/// either — the honest degraded state, rather than a picker that will fail.
const DEGRADED: &str = "No credential is set for this company and this instance carries no \
     platform identity, so providers cannot be connected or used. Set the company's TinyHumans \
     key to enable them.";

/// Builds the company-credential route fragment.
pub fn router() -> Router<AppState> {
    scoped("/credential", get(get_status).put(set_key))
}

/// The company's credential status as the console renders it. **Never** carries
/// the key — only whether one is stored and which tier a brokered call presents.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialStatusDto {
    /// Whether this company has its **own** TinyHumans key stored. False does
    /// not mean no credential — see `source`.
    configured: bool,
    /// Which identity a brokered call presents right now: `company` (this
    /// company's own key), `attested` / `static` (this instance's platform
    /// identity), or `none`.
    source: CredentialSource,
    /// The consequence of setting this key, stated plainly, or the degraded
    /// state when nothing can be presented at all.
    notice: String,
}

/// A mutating response: the resulting status plus the switch reminder.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MutationResponse {
    status: CredentialStatusDto,
    note: String,
}

/// Set-key body. `key` is write-only intake (never returned): a non-empty value
/// sets or rotates it, an explicit empty string clears it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetKey {
    key: String,
}

/// Resolves the credential status DTO for a company.
///
/// `source` comes from the **same** resolver the harness and the Composio routes
/// use, so the console can never report a tier the agents are not actually on.
async fn effective_status(runtime: &CompanyRuntime) -> Result<CredentialStatusDto, ApiError> {
    let secrets = runtime.secrets();
    let configured = key_configured(runtime.id(), secrets.as_ref())
        .await
        .map_err(ApiError)?;
    let env = crate::app::config::ProcessEnv;
    let source = resolve(
        runtime.id(),
        secrets.as_ref(),
        crate::company::TinyhumansTokenSource::from_env(&env).map(std::sync::Arc::new),
    )
    .await
    .source();
    Ok(CredentialStatusDto {
        configured,
        source,
        notice: if source == CredentialSource::None {
            DEGRADED.to_string()
        } else {
            CONSEQUENCE.to_string()
        },
    })
}

/// `GET …/credential` — whether this company has its own key, and which identity
/// its brokered calls present.
async fn get_status(company: ScopedCompany) -> Result<Json<CredentialStatusDto>, ApiError> {
    Ok(Json(effective_status(company.runtime.as_ref()).await?))
}

/// `PUT …/credential` — set / rotate / clear the company's write-only TinyHumans
/// credential. **Admin-only** — see the module docs.
async fn set_key(
    company: AdminScopedCompany,
    Json(body): Json<SetKey>,
) -> Result<Json<MutationResponse>, ApiError> {
    let runtime = company.runtime.as_ref();
    store_key(runtime.id(), runtime.secrets().as_ref(), &body.key)
        .await
        .map_err(ApiError)?;
    // The credential decides which account the backend resolves, so a change can
    // change which Composio catalog this company gets. Drop the cached one
    // rather than serving the previous account's answer for up to `CATALOG_TTL`.
    super::composio::evict_catalog_cache(runtime);
    // After the store, so the journal records a completed change. An empty value
    // is a clear, not a set — worth telling apart in an audit trail, since one
    // grants access and the other withdraws it.
    let change = if body.key.trim().is_empty() {
        "credential_cleared"
    } else {
        "credential_set"
    };
    journal(&company, change).await?;
    Ok(Json(MutationResponse {
        status: effective_status(runtime).await?,
        note: SWITCH_NOTE.to_string(),
    }))
}

/// Records who changed the company's credential (issue #403's discipline).
///
/// Propagates a journal failure rather than swallowing it, mirroring
/// [`ops::composio`](super::composio): the point of this record is that a change
/// to what the company's agents act through is never invisible, and an audit
/// line that quietly fails to be written is the one failure mode that would
/// defeat it.
async fn journal(company: &AdminScopedCompany, change: &str) -> Result<(), ApiError> {
    company
        .runtime
        .events()
        .append(
            company.id(),
            CompanyEvent::ToolAccessChanged {
                change: change.to_string(),
                toolkit: None,
                by: Some(company.actor()),
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(())
}

#[cfg(test)]
mod test;
