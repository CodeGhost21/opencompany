//! The company's Smithery directory credential, read and written (issue #1287).
//!
//! Always compiled, unlike the rest of this module's routes. The credential is a
//! secret-store slot and a console field; neither needs the `mcp` feature, and a
//! build without it still has to let an admin set the key that a later build
//! with it will use. This mirrors [`ops::composio`](crate::server::ops::composio),
//! whose token plane is likewise always present while the tools that spend it
//! are gated.
//!
//! ## Authority
//!
//! Writes take [`AdminScopedCompany`], reads take [`ScopedCompany`] — the same
//! split every other company credential uses. Setting this key decides which
//! Smithery account the company's directory calls are billed to and attributed
//! to, which is an owner's decision; knowing *whether* one is set is what lets a
//! member understand why the directory is thin, and carries nothing secret.
//!
//! ## What it does and does not reach
//!
//! Discovery only. See [`company::smithery`](crate::company::smithery) — an
//! installed server connects with its own stored credentials, so clearing this
//! key stops new browsing and leaves running servers alone. The notices below
//! say so, because the opposite is the natural guess and would make an operator
//! afraid to rotate.

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::company::runtime::CompanyRuntime;
use crate::company::smithery::{DirectoryKeySource, key_configured, resolve, store_key};
use crate::ports::types::CompanyEvent;
use crate::server::error::ApiError;
use crate::server::ops::{AdminScopedCompany, ScopedCompany, scoped};

/// Said when this company has its own key, or could set one.
const CONSEQUENCE: &str = "This key is what lets the MCP directory show hosted servers. Without \
     one only the open registry is searched, and almost everything in it runs as a local \
     subprocess this host cannot launch — so the browse list comes back nearly empty. It is used \
     for browsing and installing only: servers already installed keep their own credentials, so \
     rotating or clearing this does not disconnect them.";

/// Said when no company key is set but the host process carries one.
const SHARED: &str = "The MCP directory is working on a key set for this whole host, so every \
     company on it browses through the same Smithery account. Set this company's own key to \
     browse under its own account instead.";

/// Said when nothing resolves anywhere — the honest degraded state.
const DEGRADED: &str = "No Smithery key is set, so the MCP directory searches only the open \
     registry and will show very few servers. Add this company's Smithery key to browse the \
     hosted ones.";

/// Returned after a write: what changes, and when.
const SWITCH_NOTE: &str = "Saved. The next directory search uses it — nothing needs restarting.";

/// Builds the directory-credential route fragment.
pub(super) fn router() -> Router<AppState> {
    scoped("/mcp/registry/credential", get(get_status).put(set_key))
}

/// The directory credential as the console renders it. **Never** carries the key.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryCredentialDto {
    /// Whether this company has its **own** key stored. `false` does not mean
    /// the directory is off — read `source`.
    configured: bool,
    /// Which key a directory call presents: `company`, `environment` (one shared
    /// by every company on this host), or `none`.
    source: DirectoryKeySource,
    /// The consequence, stated plainly, or the degraded state. Worded by the
    /// host so the console cannot drift from what the host actually does.
    notice: String,
}

/// A mutating response: the resulting status plus the switch reminder.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MutationResponse {
    status: DirectoryCredentialDto,
    note: String,
}

/// Set-key body. Write-only intake: a non-empty value sets or rotates, an
/// explicit empty string clears.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetKey {
    key: String,
}

/// Resolves the status DTO for a company.
///
/// `configured` and `source` answer different questions on purpose and are both
/// reported: the first is "did *this company* set one", the second is "what does
/// a search actually present". A host with `SMITHERY_API_KEY` set reads
/// `configured: false, source: "environment"`, and both halves are true.
/// Collapsing them into one boolean is how the Composio panel came to claim no
/// credential while calls were succeeding (issue #886).
async fn effective_status(runtime: &CompanyRuntime) -> Result<DirectoryCredentialDto, ApiError> {
    let secrets = runtime.secrets();
    let configured = key_configured(runtime.id(), secrets.as_ref())
        .await
        .map_err(ApiError)?;
    let source = resolve(
        runtime.id(),
        secrets.as_ref(),
        &crate::app::config::ProcessEnv,
    )
    .await
    .map_err(ApiError)?
    .source();
    Ok(DirectoryCredentialDto {
        configured,
        source,
        notice: match source {
            DirectoryKeySource::Company => CONSEQUENCE.to_string(),
            DirectoryKeySource::Environment => SHARED.to_string(),
            DirectoryKeySource::None => DEGRADED.to_string(),
        },
    })
}

/// `GET …/mcp/registry/credential` — whether this company has its own Smithery
/// key, and which key its directory calls present.
async fn get_status(company: ScopedCompany) -> Result<Json<DirectoryCredentialDto>, ApiError> {
    Ok(Json(effective_status(company.runtime.as_ref()).await?))
}

/// `PUT …/mcp/registry/credential` — set / rotate / clear the write-only
/// Smithery key. **Admin-only**.
async fn set_key(
    company: AdminScopedCompany,
    Json(body): Json<SetKey>,
) -> Result<Json<MutationResponse>, ApiError> {
    let runtime = company.runtime.as_ref();
    store_key(runtime.id(), runtime.secrets().as_ref(), &body.key)
        .await
        .map_err(ApiError)?;
    // After the store, so the journal records a completed change. A clear and a
    // set are told apart for the same reason `ops::company_key` tells them
    // apart: one widens what the company can reach and the other narrows it, and
    // an audit line that cannot say which is most of the way to not having one.
    let change = if body.key.trim().is_empty() {
        "smithery_key_cleared"
    } else {
        "smithery_key_set"
    };
    journal(&company, change).await?;
    Ok(Json(MutationResponse {
        status: effective_status(runtime).await?,
        note: SWITCH_NOTE.to_string(),
    }))
}

/// Records who changed the directory credential.
///
/// Propagates a journal failure rather than swallowing it, matching the sibling
/// credential routes: the record exists so that a change to what the company
/// browses and bills through is never invisible.
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
