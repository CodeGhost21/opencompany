//! Per-tenant Composio connection management (issue #110, epic #26 Cell D): read
//! the company's Composio status and set a per-company OAuth bearer token.
//!
//! ## Where the credential normally comes from
//!
//! On the hosted platform a company needs to paste nothing. The instance already
//! authenticates with a platform-minted, audience-bound identity, and Composio
//! calls present that — the backend derives the Composio entity from it. The read
//! shape reports this as `credentialSource: "attested"`, and there is nothing
//! stored on the instance to leak, rotate, or lose.
//!
//! ## `PUT …/composio/token` — the BYO override
//!
//! The write route stays, and it is **not** the hosted path. It is:
//!
//! * the **per-company BYO override** — a company that has its own Composio
//!   account/token can set it here, and it wins over the instance identity for
//!   that company only; and
//! * the escape hatch for running this repo **standalone**. This repository is
//!   public and people do run it that way, but self-hosting is **unsupported**:
//!   there is no platform identity to present, so pasting a token is the only way
//!   to reach Composio at all. Do not read this as parity with the hosted
//!   product — it is a hatch, not a deployment mode, and nothing else about
//!   standalone operation is supported this milestone.
//!
//! ## Who a connection belongs to (issue #403)
//!
//! **A connection belongs to the company, and is admin-managed.** The account
//! reached through it is the account the company's *agents* act through, so it
//! is company property in the same sense the roster and the manifest are — not
//! the personal property of whichever member happened to click Connect.
//!
//! This is not a new position; it is the one the code already took and did not
//! enforce. The native OAuth plane's connect-time identity check
//! ([`account_mismatch`](super::connections)) compares the connected account
//! against `bootstrap_admins` — the company's *admin* addresses — which only
//! makes sense if a connection is the company's. Issue #316 settled "one
//! operator, one connected account"; this is the layer above it, and it
//! resolves the same way.
//!
//! Two consequences, both deliberate:
//!
//! * **Writes require an admin.** `PUT …/composio/token` and
//!   `POST …/composio/authorize` take [`AdminScopedCompany`]. A member is
//!   refused with a message that says why.
//! * **Reads stay open to any member.** `GET …/composio` and
//!   `GET …/composio/connections` carry no credential — only a tier name,
//!   non-secret routing, and which providers are connected. Knowing *that*
//!   Gmail is connected is what lets a member understand why an agent can read
//!   mail; being able to change it is the part that needed an owner. The split
//!   mirrors the roster, where `GET …/team` is open and the budget writes are
//!   not.
//!
//! Either way the token is **write-only** over the API: set through the `token`
//! field, stored in the secret store under
//! [`TOKEN_KEY`](crate::company::composio::TOKEN_KEY), and **never** echoed. The
//! read shape carries only `credentialSource` plus non-secret routing (backend
//! URL, toolkit allowlist) — never a token, and never a file path. A set / rotate
//! / clear takes effect on the agents' **next turn** with no restart (the harness
//! re-resolves the credential each turn and rebuilds the roster when the
//! *identity* behind it changes).

use axum::Json;
use axum::Router;
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};

use crate::AppState;
// `token_configured` is gone from this list deliberately: the status route no
// longer re-derives the credential tier from booleans, it asks the resolver
// (`resolve_credential`) — see `credential_source_for` below.
use crate::company::composio::{CatalogEntry, backend_url_or_default, store_token};
use crate::company::credentials::{CredentialSource, TinyhumansTokenSource};
use crate::company::runtime::CompanyRuntime;
use crate::ports::types::CompanyEvent;
use crate::server::error::ApiError;
use crate::server::ops::composio_toolkits::{self, CatalogSource, OpenModeToolkits};
use crate::server::ops::{AdminScopedCompany, ScopedCompany, scoped};

/// The reminder attached to every mutating response.
const SWITCH_NOTE: &str =
    "Agents pick up the new Composio token on their next turn — no restart needed.";

/// The toolkits the console should offer for a company, whether that answer
/// came from open mode, and where the list came from.
///
/// A non-empty manifest allowlist is authoritative and is offered **verbatim**:
/// a company that deliberately narrowed its belt sees exactly what it chose, the
/// backend catalog is not consulted, and nothing can widen it. That is not a
/// performance shortcut — it is the boundary. Widening a restrictive manifest
/// from a catalog fetch would silently hand a company providers it decided
/// against.
///
/// Empty means **open mode**, where the manifest is deferring to the backend's
/// own server-enforced allowlist. The honest answer there is the backend's live
/// catalog ([`composio_toolkits`]), fetched once per
/// [`CATALOG_TTL`](composio_toolkits::CATALOG_TTL) and marked
/// [`CatalogSource::Fallback`] with a reason if it cannot be had.
///
/// Returning the triple (rather than letting the console infer any of it) is the
/// whole point of the fix: the console must never have to guess which of two
/// opposite meanings an empty list carries, nor whether the list it is rendering
/// is the real catalog.
///
/// This is a **console affordance only**. Agent-side toolkit admission is
/// [`toolkit_allowed`](crate::harness::composio), which still treats an empty
/// manifest list as "allow every toolkit" — unchanged by any of this.
async fn effective_toolkits(
    runtime: &CompanyRuntime,
    manifest: &[String],
) -> (bool, OpenModeToolkits) {
    if manifest.is_empty() {
        (true, open_mode_toolkits(runtime).await)
    } else {
        (
            false,
            OpenModeToolkits {
                // A manifest allowlist is slugs and nothing else — the company
                // wrote it by hand, and the catalog is deliberately not
                // consulted here (it must not be able to widen a list that
                // narrowed on purpose). So these entries carry no metadata, and
                // the console falls back to its own typography for them.
                toolkits: manifest.iter().map(CatalogEntry::from_slug).collect(),
                source: CatalogSource::Manifest,
                notice: None,
            },
        )
    }
}

/// Open mode's provider list: the backend's live catalog, cached, or an
/// honestly-marked fallback.
///
/// The cache is consulted before the (cfg-split) fetch and records the outcome
/// either way, so a status poll during an outage costs a map lookup rather than
/// another fetch timeout of waiting.
async fn open_mode_toolkits(runtime: &CompanyRuntime) -> OpenModeToolkits {
    let key = catalog_cache_key(runtime);
    if let Some(cached) = composio_toolkits::cache().lookup(&key, std::time::Instant::now()) {
        return OpenModeToolkits::from_outcome(cached);
    }
    let outcome = fetch_catalog(runtime).await;
    composio_toolkits::cache().store(&key, outcome.clone(), std::time::Instant::now());
    OpenModeToolkits::from_outcome(outcome)
}

/// Drop this company's cached catalog.
///
/// Exposed because the catalog is a property of the **credential**, not of the
/// Composio token alone: changing the company's TinyHumans key
/// ([`ops::company_key`](super::company_key)) can change which account the
/// backend resolves and therefore which catalog it serves. Both writes evict
/// through here rather than each reaching into the cache with its own key
/// derivation.
pub(crate) fn evict_catalog_cache(runtime: &CompanyRuntime) {
    composio_toolkits::cache().evict(&catalog_cache_key(runtime));
}

/// This company's catalog cache key. The backend URL is resolved the same way
/// the status DTO resolves it, so a company repointed at a different backend
/// does not read the old backend's catalog.
fn catalog_cache_key(runtime: &CompanyRuntime) -> String {
    use crate::app::config::EnvSource;
    let env = crate::app::config::ProcessEnv;
    let backend_url = backend_url_or_default(
        env.get(crate::company::composio::COMPOSIO_BACKEND_URL_ENV),
        env.get(crate::company::composio::TINYHUMANS_API_URL_ENV),
    );
    composio_toolkits::cache_key(runtime.id(), &backend_url)
}

/// Fetch the backend's toolkit catalog for this company, bounded by
/// `composio_toolkits::FETCH_TIMEOUT`.
///
/// `Err` is a plain-language reason the console can show — never a bare
/// fall-through to a short list that would look authoritative.
#[cfg(feature = "composio")]
async fn fetch_catalog(runtime: &CompanyRuntime) -> Result<Vec<CatalogEntry>, String> {
    // No credential of any tier means there is nothing to dial the backend
    // with. Say that, rather than spending the timeout to discover it.
    //
    // But say only that when it is what happened. `resolve_tenant` answers
    // `Conflict` for "nothing configured" and propagates anything else — a
    // secret-store read failure among them. Collapsing every error into "no
    // credential yet" would tell an operator whose store hiccupped that they
    // never set a key, which is the confident-wrong-answer this credential work
    // exists to remove (see `company_key::resolve`).
    let config = resolve_tenant(runtime).await.map_err(|err| {
        if matches!(err.0, crate::error::OpenCompanyError::Conflict(_)) {
            "this company has no Composio credential yet, so the catalog cannot be read".to_string()
        } else {
            format!(
                "this company's Composio credential could not be resolved: {}",
                err.0
            )
        }
    })?;
    let fetch = crate::harness::composio::list_catalog_toolkits(&config);
    match tokio::time::timeout(composio_toolkits::FETCH_TIMEOUT, fetch).await {
        Err(_) => Err(format!(
            "the Composio backend did not answer within {}s",
            composio_toolkits::FETCH_TIMEOUT.as_secs()
        )),
        Ok(Err(err)) => Err(err.to_string()),
        Ok(Ok(toolkits)) if toolkits.is_empty() => {
            Err("the Composio backend returned an empty catalog".to_string())
        }
        Ok(Ok(toolkits)) => Ok(toolkits),
    }
}

/// Without the `composio` feature there is no client to fetch a catalog with.
/// The status route still answers (reporting `inBuild:false`), and it says so
/// rather than presenting the fallback as the backend's list.
#[cfg(not(feature = "composio"))]
async fn fetch_catalog(_runtime: &CompanyRuntime) -> Result<Vec<CatalogEntry>, String> {
    Err("Composio is not compiled into this build".to_string())
}

/// Builds the Composio management route fragment.
///
/// The read/write-token plane (`GET …/composio`, `PUT …/composio/token`) is
/// always present. The per-provider OAuth sign-in plane (`POST
/// …/composio/authorize`, `GET …/composio/connections`) is **also** always
/// present in the route table — mirroring how `get_status` stays wired and
/// reports `inBuild:false` rather than `#[cfg]`-ing itself out — but its
/// handlers only reach the live Composio client under the `composio` feature;
/// otherwise they answer `409 Conflict` "not in this build".
pub fn router() -> Router<AppState> {
    scoped("/composio", get(get_status))
        .merge(scoped("/composio/token", put(set_token)))
        .merge(scoped("/composio/authorize", post(authorize)))
        .merge(scoped("/composio/connections", get(connections)))
}

/// The company's Composio status as the console renders it. **Never** carries the
/// token — only the non-secret `credentialSource` plus routing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComposioStatusDto {
    /// Whether the `composio` feature is compiled into this build at all (the
    /// tools only exist under it). `false` lets the console show a "not in this
    /// build" state rather than implying a missing credential.
    in_build: bool,
    /// Whether the company **explicitly** grants the `composio` namespace (a `*`
    /// wildcard does NOT count).
    granted: bool,
    /// Where this company's Composio credential comes from:
    ///
    /// * `attested` — the instance's platform identity (nothing is stored here);
    /// * `static` — a token this company pasted, or a static instance key;
    /// * `none` — no credential can be obtained, so no tools are wired.
    ///
    /// A tier name, never a credential and never a path.
    credential_source: CredentialSource,
    /// The effective Composio backend URL (env override or default). Non-secret.
    backend_url: String,
    /// The manifest toolkit allowlist verbatim (empty = defer to the backend
    /// allowlist, i.e. open mode). Kept as-is so an operator can still see what
    /// the manifest literally says; the console renders
    /// [`Self::effective_toolkits`] instead.
    toolkits: Vec<String>,
    /// Whether this company is in **open mode** — an empty manifest allowlist,
    /// meaning the backend's own allowlist governs and every toolkit it permits
    /// is reachable (issue #397).
    ///
    /// The console needs this told to it rather than inferred: an empty
    /// `toolkits` means *allow everything*, which is the opposite of what an
    /// empty list reads as.
    open_mode: bool,
    /// The toolkits the console offers as provider rows — the manifest list when
    /// it is non-empty, else the backend's live catalog (or, when that cannot be
    /// fetched, a built-in fallback flagged by [`Self::catalog_source`]). In open
    /// mode this is still not a hard limit: any slug the backend permits can be
    /// authorized by typing it.
    effective_toolkits: Vec<String>,
    /// The same providers as [`Self::effective_toolkits`], in the same order,
    /// carrying whatever display metadata the backend published for each —
    /// name, description, logo URL, and Composio's own category names (issue
    /// #600).
    ///
    /// **Additive, and deliberately so.** The slug list above is the contract
    /// every existing consumer reads and the only thing an authorize call
    /// needs; this is the render model beside it. Replacing the slug list with
    /// this one would have bought nothing and broken that contract.
    ///
    /// Empty metadata is a real state, not a bug: a manifest allowlist, a
    /// fallback list, and a backend predating the dynamic catalog all yield
    /// slug-only entries, and the console renders those with its own
    /// typography.
    ///
    /// The categories are forwarded **verbatim**, uninterpreted. The console
    /// buckets them by substring, which is what lets a Composio integration
    /// added tomorrow land in the right group with no change on either side of
    /// this wire.
    effective_catalog: Vec<CatalogEntry>,
    /// Where [`Self::effective_toolkits`] came from — `manifest`, `backend`, or
    /// `fallback` (issue #397).
    ///
    /// The console must be able to distinguish "these are the hundred providers
    /// the backend permits" from "the catalog could not be read, here are eight
    /// we know of". Both render as a list of slugs; only one of them is
    /// authoritative, and a console that could not tell them apart would present
    /// the second as though it were the first.
    catalog_source: CatalogSource,
    /// Why the list is a fallback, in plain language for the operator. `None`
    /// unless [`Self::catalog_source`] is `fallback`.
    catalog_notice: Option<String>,
}

/// A mutating response: the resulting status plus the switch reminder.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MutationResponse {
    status: ComposioStatusDto,
    note: String,
}

/// Set-token body. `token` is write-only intake (never returned): a non-empty
/// value rotates the company's BYO override, an explicit empty string clears it
/// (reverting to the instance identity, where there is one).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetToken {
    token: String,
}

/// `POST …/composio/authorize` body: the toolkit slug (`gmail` / `slack` /
/// `github` / …) to begin an OAuth handoff for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizeBody {
    toolkit: String,
}

/// `POST …/composio/authorize` response: the Composio-hosted connect URL the
/// operator opens in a browser tab. Composio runs the OAuth itself; there is no
/// local callback — the console polls [`ConnectionDto`] until the toolkit
/// reports connected.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorizeDto {
    connect_url: String,
}

/// One per-toolkit connected state in the `GET …/composio/connections`
/// response: `connected` is true when the company has at least one active
/// connection for that toolkit.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionDto {
    toolkit: String,
    connected: bool,
}

/// Which tier a company's Composio credential comes from.
///
/// Asks the resolver itself rather than restating its precedence. The console
/// must never be able to name a tier the agents are not on, and a second copy of
/// the rule is a second place to forget to update — which is how a status route
/// ends up confidently reporting a credential that no longer resolves.
///
/// Takes the instance identity already resolved, rather than an `&dyn
/// EnvSource`: a trait object with no `Send + Sync` bound held across the await
/// below makes the whole handler future non-`Send`, which axum rejects. Callers
/// resolve it from whichever environment they mean, so the matrix stays testable
/// without mutating the process environment.
async fn credential_source_for(
    runtime: &CompanyRuntime,
    token_source: Option<std::sync::Arc<TinyhumansTokenSource>>,
) -> Result<CredentialSource, ApiError> {
    Ok(crate::company::composio::resolve_credential(
        runtime.id(),
        runtime.secrets().as_ref(),
        token_source,
    )
    .await
    .map_err(ApiError)?
    .source())
}

/// Resolves the Composio status DTO for a company.
async fn effective_status(runtime: &CompanyRuntime) -> Result<ComposioStatusDto, ApiError> {
    let record = runtime.store().load(runtime.id()).await.map_err(ApiError)?;
    let (granted, toolkits) = match record {
        Some(record) => (
            crate::company::grants_composio_explicit(&record.manifest.tools.allow),
            record.manifest.tools.composio.toolkits.clone(),
        ),
        None => (false, Vec::new()),
    };
    let env = crate::app::config::ProcessEnv;
    let (env_url, api_url) = {
        use crate::app::config::EnvSource;
        (
            env.get(crate::company::composio::COMPOSIO_BACKEND_URL_ENV),
            env.get(crate::company::composio::TINYHUMANS_API_URL_ENV),
        )
    };
    let credential_source = credential_source_for(
        runtime,
        TinyhumansTokenSource::from_env(&env).map(std::sync::Arc::new),
    )
    .await?;
    let (open_mode, effective) = effective_toolkits(runtime, &toolkits).await;
    Ok(ComposioStatusDto {
        in_build: cfg!(feature = "composio"),
        granted,
        credential_source,
        backend_url: backend_url_or_default(env_url, api_url),
        toolkits,
        open_mode,
        effective_toolkits: effective.slugs(),
        effective_catalog: effective.toolkits,
        catalog_source: effective.source,
        catalog_notice: effective.notice,
    })
}

/// `GET …/composio` — the company's Composio status.
async fn get_status(company: ScopedCompany) -> Result<Json<ComposioStatusDto>, ApiError> {
    Ok(Json(effective_status(company.runtime.as_ref()).await?))
}

/// `PUT …/composio/token` — set / rotate / clear this company's BYO token. See
/// the module docs: the hosted path needs no token, and standalone operation
/// (where this is the only option) is unsupported.
///
/// **Admin-only** (issue #403). This is the sharpest write in the module: the
/// token it stores is the identity every one of the company's agents presents
/// to Composio, so whoever sets it decides which account those agents act
/// through. That is a decision made *for* the company, not a member's own, and
/// [`AdminScopedCompany`] is what says so in the signature.
async fn set_token(
    company: AdminScopedCompany,
    Json(body): Json<SetToken>,
) -> Result<Json<MutationResponse>, ApiError> {
    let runtime = company.runtime.as_ref();
    store_token(runtime.id(), runtime.secrets().as_ref(), &body.token)
        .await
        .map_err(ApiError)?;
    // The credential decides which Composio entity the backend resolves, so a
    // set / rotate / clear can change which catalog this company gets. Drop the
    // cached one rather than serving the previous account's answer for up to
    // `CATALOG_TTL` — the response below re-reads the status, so the operator
    // sees the new list immediately.
    evict_catalog_cache(runtime);
    // After the store, so the journal records a completed change. An empty
    // value is a clear, not a set — the two are worth telling apart in an
    // audit trail, since one grants access and the other withdraws it.
    let change = if body.token.trim().is_empty() {
        "credential_cleared"
    } else {
        "credential_set"
    };
    journal(&company, change, None).await?;
    Ok(Json(MutationResponse {
        status: effective_status(runtime).await?,
        note: SWITCH_NOTE.to_string(),
    }))
}

/// Records who changed the company's tool access (issue #403).
///
/// Propagates a journal failure rather than swallowing it, unlike the
/// best-effort workflow journaling. The point of this record is that a change
/// to what the company's agents connect through is never invisible; an audit
/// line that quietly fails to be written is the one failure mode that would
/// defeat it. `MemoryFactDeleted` — the other write journaled *for* the audit
/// trail — propagates for the same reason.
async fn journal(
    company: &AdminScopedCompany,
    change: &str,
    toolkit: Option<String>,
) -> Result<(), ApiError> {
    company
        .runtime
        .events()
        .append(
            company.id(),
            CompanyEvent::ToolAccessChanged {
                change: change.to_string(),
                toolkit,
                by: Some(company.actor()),
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(())
}

/// `POST …/composio/authorize` — begin a per-provider OAuth handoff and return
/// the Composio-hosted connect URL the operator opens in a browser tab.
///
/// Composio runs the OAuth flow itself — there is **no** local callback route.
/// The console opens the URL and polls [`connections`] until the toolkit
/// reports connected. Under a non-`composio` build this answers `409 Conflict`
/// (see [`router`]).
///
/// **Admin-only** (issue #403). The connection this begins belongs to the
/// company — it is the account its agents will act through from then on — so
/// the person who chooses it has to be someone entitled to choose on the
/// company's behalf. Note that nothing downstream can re-check this: Composio
/// runs its own OAuth with no callback here, so unlike the native connections
/// plane there is no later point at which the connecting identity could be
/// compared against the company's operators. Connect time is the only boundary
/// there is.
async fn authorize(
    company: AdminScopedCompany,
    Json(body): Json<AuthorizeBody>,
) -> Result<Json<AuthorizeDto>, ApiError> {
    let dto = authorize_impl(company.runtime.as_ref(), body.toolkit.clone()).await?;
    // Only once a connect URL was actually minted — a refused or unbuildable
    // authorize changed nothing and does not belong in the trail.
    journal(
        &company,
        "provider_authorization_started",
        Some(body.toolkit),
    )
    .await?;
    Ok(dto)
}

/// `GET …/composio/connections` — the company's per-toolkit connected state,
/// one [`ConnectionDto`] per toolkit that has at least one connection. The
/// console cross-references this against the granted `toolkits` to render each
/// provider row's connected/sign-in state. `409 Conflict` on a non-`composio`
/// build.
async fn connections(company: ScopedCompany) -> Result<Json<Vec<ConnectionDto>>, ApiError> {
    connections_impl(company.runtime.as_ref()).await
}

/// Resolve the per-tenant Composio config (bearer + backend URL + toolkit
/// allowlist) the way the harness roster build does, so the console dials the
/// backend with the exact same tenant identity. A `409 Conflict` when no
/// credential of any tier can be resolved — neither this company's own stored
/// token nor a platform identity — in which case the operator must paste a token
/// before OAuth.
#[cfg(feature = "composio")]
pub(crate) async fn resolve_tenant(
    runtime: &CompanyRuntime,
) -> Result<crate::harness::composio::TenantComposio, ApiError> {
    use crate::app::config::EnvSource;

    let record = runtime.store().load(runtime.id()).await.map_err(ApiError)?;
    let toolkits = record
        .map(|record| record.manifest.tools.composio.toolkits.clone())
        .unwrap_or_default();
    let env = crate::app::config::ProcessEnv;
    let backend_env = env.get(crate::company::composio::COMPOSIO_BACKEND_URL_ENV);
    let api_env = env.get(crate::company::composio::TINYHUMANS_API_URL_ENV);
    // Resolved here rather than through `TenantComposio::resolve` so a store
    // read failure stays distinguishable from "nothing configured". This is the
    // path that *establishes* a connection, and a connection lives on the
    // backend keyed by the account the bearer resolves to — so guessing here
    // would attribute a company's Gmail to whichever identity happened to
    // resolve during the outage. The roster path can afford to shrug and
    // withhold tools; this one must say what went wrong and connect nothing.
    let credential = crate::company::composio::resolve_credential(
        runtime.id(),
        runtime.secrets().as_ref(),
        crate::company::TinyhumansTokenSource::from_env(&env).map(std::sync::Arc::new),
    )
    .await
    .map_err(ApiError)?;
    if !credential.configured() {
        return Err(ApiError(crate::error::OpenCompanyError::Conflict(
            "no Composio credential is available for this company — set the company's TinyHumans \
             credential, or paste its own Composio token"
                .to_string(),
        )));
    }
    Ok(crate::harness::composio::TenantComposio::new(
        backend_url_or_default(backend_env, api_env),
        credential,
        toolkits,
    ))
}

#[cfg(feature = "composio")]
async fn authorize_impl(
    runtime: &CompanyRuntime,
    toolkit: String,
) -> Result<Json<AuthorizeDto>, ApiError> {
    let config = resolve_tenant(runtime).await?;
    let connect_url = crate::harness::composio::authorize_connect_url(&config, &toolkit)
        .await
        .map_err(|err| {
            ApiError(crate::error::OpenCompanyError::TinyHumans {
                code: "composio_authorize".to_string(),
                message: err.to_string(),
            })
        })?;
    Ok(Json(AuthorizeDto { connect_url }))
}

#[cfg(feature = "composio")]
async fn connections_impl(runtime: &CompanyRuntime) -> Result<Json<Vec<ConnectionDto>>, ApiError> {
    let config = resolve_tenant(runtime).await?;
    let states = crate::harness::composio::list_connection_states(&config)
        .await
        .map_err(|err| {
            ApiError(crate::error::OpenCompanyError::TinyHumans {
                code: "composio_connections".to_string(),
                message: err.to_string(),
            })
        })?;
    Ok(Json(
        states
            .into_iter()
            .map(|(toolkit, connected)| ConnectionDto { toolkit, connected })
            .collect(),
    ))
}

/// A `409 Conflict` "Composio is not in this build" — the OAuth plane's off-state
/// under a non-`composio` build. Mirrors the status route's `inBuild:false`
/// semantics rather than pretending nothing is connected.
#[cfg(not(feature = "composio"))]
fn not_in_build() -> ApiError {
    ApiError(crate::error::OpenCompanyError::Conflict(
        "Composio is not compiled into this build".to_string(),
    ))
}

#[cfg(not(feature = "composio"))]
async fn authorize_impl(
    _runtime: &CompanyRuntime,
    _toolkit: String,
) -> Result<Json<AuthorizeDto>, ApiError> {
    Err(not_in_build())
}

#[cfg(not(feature = "composio"))]
async fn connections_impl(_runtime: &CompanyRuntime) -> Result<Json<Vec<ConnectionDto>>, ApiError> {
    Err(not_in_build())
}

#[cfg(test)]
mod tests {
    use super::{
        CatalogEntry, CatalogSource, ComposioStatusDto, CredentialSource, credential_source_for,
    };
    use crate::server::ops::composio_toolkits;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::company::CompanyManifest;
    use crate::ports::types::{CompanyId, CompanyRecord};
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    const TOKEN: &str = "composio-tenant-bearer-SECRET-xyz";

    fn home() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("oc-composio-")
            .tempdir()
            .expect("tempdir")
    }

    async fn state_with_manifest(home: &std::path::Path, manifest_toml: &str) -> AppState {
        state_with_manifest_id(home, "acme", manifest_toml).await
    }

    /// The same, under an explicit company id.
    ///
    /// The catalog cache is process-wide and keyed by company, so the tests that
    /// seed it need ids of their own — two tests sharing `acme` would share an
    /// entry and race.
    async fn state_with_manifest_id(
        home: &std::path::Path,
        company: &str,
        manifest_toml: &str,
    ) -> AppState {
        use crate::ports::CompanyStore;
        let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
        let store = FsCompanyStore::new(home.to_path_buf());
        let id = CompanyId::new(company);
        store
            .save(&CompanyRecord {
                id: id.clone(),
                manifest: manifest.clone(),
                ledger: Vec::new(),
                lifecycle: "running".to_string(),
                overlay_agents: Vec::new(),
                overlay_desk_members: Vec::new(),
                overlay_desk_order: Vec::new(),
                overlay_desks: Vec::new(),
                overlay_workflows: Vec::new(),
                overlay_budgets: Vec::new(),
                disabled_workflows: Vec::new(),
                template_provenance: None,
            })
            .await
            .unwrap();
        let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        let state = AppState::new(AppConfig::default());
        state.registry().insert(id, std::sync::Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, company).await;
        state
    }

    async fn send(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, String) {
        send_for(state, "acme", method, uri, body).await
    }

    /// [`send`] against a named company's session.
    async fn send_for(
        state: &AppState,
        company: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, String) {
        send_as(
            state,
            method,
            uri,
            body,
            Auth::Cookie(crate::server::test_support::fixed_cookie(company)),
        )
        .await
    }

    /// How a request presents itself, so the role boundary can be driven with
    /// an admin session, a member session, or a machine credential.
    enum Auth {
        Cookie(String),
        Bearer(String),
    }

    async fn send_as(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
        auth: Auth,
    ) -> (StatusCode, Value, String) {
        let request = Request::builder().method(method).uri(uri);
        let request = match auth {
            Auth::Cookie(cookie) => request.header("cookie", cookie),
            Auth::Bearer(token) => request.header("authorization", format!("Bearer {token}")),
        };
        let request = match body {
            Some(body) => request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
            None => request.body(Body::empty()).unwrap(),
        };
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let raw = String::from_utf8_lossy(&bytes).to_string();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, value, raw)
    }

    /// Issue #397: an **empty** manifest allowlist means "defer to the backend"
    /// — allow everything — so the status must report open mode and hand the
    /// console a non-empty starting set. The old console gate keyed off
    /// `toolkits.length > 0` and therefore rendered nothing in exactly the case
    /// where everything is permitted.
    #[tokio::test]
    async fn an_empty_toolkit_list_is_open_mode_and_still_offers_providers() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        // No `[tools.composio]` section at all — the shape 19 of 20 templates ship.
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
        )
        .await;

        let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(dto["granted"], true);
        assert_eq!(dto["toolkits"], json!([]), "the manifest list is verbatim");
        assert_eq!(dto["openMode"], true);
        let offered = dto["effectiveToolkits"]
            .as_array()
            .expect("open mode carries an effective toolkit list");
        assert!(
            !offered.is_empty(),
            "open mode means allow-everything, so the console must be offered providers"
        );
        assert!(
            offered.iter().any(|t| t == "github"),
            "the fallback set covers the common providers: {offered:?}"
        );
        // No credential and (in the default build) no client, so the catalog
        // cannot be read. The list is still offered — but it says so.
        assert_eq!(
            dto["catalogSource"], "fallback",
            "an unfetchable catalog must never be reported as the backend's: {dto}"
        );
        assert!(
            dto["catalogNotice"]
                .as_str()
                .is_some_and(|n| n.contains("incomplete")),
            "a fallback tells the operator it may be incomplete: {dto}"
        );
    }

    /// The company registered under `company` in `state`.
    fn runtime_of(state: &AppState, company: &str) -> std::sync::Arc<super::CompanyRuntime> {
        state
            .registry()
            .get(&CompanyId::new(company))
            .expect("company is registered")
    }

    /// A hundred-provider catalog, the shape the backend actually returns —
    /// each entry carrying the display metadata #600 stopped discarding.
    fn hundred_entries() -> Vec<CatalogEntry> {
        (0..100)
            .map(|i| CatalogEntry {
                slug: format!("provider{i:03}"),
                name: format!("Provider {i:03}"),
                description: format!("Does provider-{i:03} things."),
                logo: Some(format!("https://logos.example.test/provider{i:03}")),
                categories: vec!["productivity".to_string()],
            })
            .collect()
    }

    /// Just the slugs of [`hundred_entries`], for asserting on the slug list
    /// the wire has always carried.
    fn hundred_slugs() -> Vec<String> {
        hundred_entries().into_iter().map(|e| e.slug).collect()
    }

    /// The heart of the reopened issue: in open mode the console is offered the
    /// **backend's** catalog, not a list maintained by hand in this repo.
    ///
    /// The cache is seeded rather than a backend stood up, because what is under
    /// test is the decision — "open mode serves the fetched catalog" — and not
    /// the HTTP call, which has its own test beside
    /// `list_catalog_toolkits`. Seeding proves the thing that regressed: that a
    /// fetched answer actually reaches the response instead of being discarded
    /// in favour of the constant.
    #[tokio::test]
    async fn open_mode_serves_the_fetched_catalog_rather_than_a_hardcoded_list() {
        let home_dir = home();
        let state = state_with_manifest_id(
            home_dir.path(),
            "catalogco",
            "[company]\nname = \"Catalog Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
        )
        .await;
        let catalog = hundred_entries();
        composio_toolkits::cache().store(
            &super::catalog_cache_key(runtime_of(&state, "catalogco").as_ref()),
            Ok(catalog.clone()),
            std::time::Instant::now(),
        );

        let (status, dto, raw) =
            send_for(&state, "catalogco", "GET", "/api/v1/company/composio", None).await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(dto["openMode"], true);
        assert_eq!(
            dto["catalogSource"], "backend",
            "a real catalog is reported as the backend's answer: {dto}"
        );
        assert_eq!(
            dto["catalogNotice"],
            Value::Null,
            "nothing to apologise for"
        );
        assert_eq!(
            dto["effectiveToolkits"],
            json!(hundred_slugs()),
            "open mode must serve what the backend permits, verbatim"
        );
        assert!(
            dto["effectiveToolkits"].as_array().unwrap().len()
                > composio_toolkits::FALLBACK_TOOLKITS.len(),
            "the fetched catalog is far longer than the built-in list — serving the \
             constant here is the exact regression this test exists to catch: {dto}"
        );
    }

    /// The important one. A company that deliberately narrowed its belt must
    /// **not** be silently widened by the catalog.
    ///
    /// The same hundred-slug catalog is in the cache; this company asked for
    /// `gmail` and gets `gmail`. Anything that unioned, defaulted-to, or fell
    /// back to the catalog for an explicit manifest would hand the company
    /// ninety-nine providers it decided against.
    #[tokio::test]
    async fn an_explicit_allowlist_is_never_widened_by_the_catalog() {
        let home_dir = home();
        let state = state_with_manifest_id(
            home_dir.path(),
            "narrowco",
            "[company]\nname = \"Narrow Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
        )
        .await;
        composio_toolkits::cache().store(
            &super::catalog_cache_key(runtime_of(&state, "narrowco").as_ref()),
            Ok(hundred_entries()),
            std::time::Instant::now(),
        );

        let (status, dto, raw) =
            send_for(&state, "narrowco", "GET", "/api/v1/company/composio", None).await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(dto["openMode"], false);
        assert_eq!(dto["effectiveToolkits"], json!(["gmail"]));
        assert_eq!(dto["toolkits"], json!(["gmail"]));
        assert_eq!(
            dto["catalogSource"], "manifest",
            "the company's own list is the source, and the catalog is not consulted: {dto}"
        );
    }

    /// A catalog that cannot be fetched degrades to the built-in list — and the
    /// response says so, both in `catalogSource` and in words the console can
    /// show the operator. Silently serving eight slugs that look like the whole
    /// catalog is the failure this pins shut.
    #[tokio::test]
    async fn an_unfetchable_catalog_is_marked_degraded_not_passed_off_as_real() {
        let home_dir = home();
        let state = state_with_manifest_id(
            home_dir.path(),
            "degradedco",
            "[company]\nname = \"Degraded Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
        )
        .await;
        composio_toolkits::cache().store(
            &super::catalog_cache_key(runtime_of(&state, "degradedco").as_ref()),
            Err("connection refused".to_string()),
            std::time::Instant::now(),
        );

        let (status, dto, raw) = send_for(
            &state,
            "degradedco",
            "GET",
            "/api/v1/company/composio",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(dto["openMode"], true);
        assert_eq!(
            dto["catalogSource"], "fallback",
            "the built-in list must never claim to be the backend's: {dto}"
        );
        let notice = dto["catalogNotice"]
            .as_str()
            .expect("a degraded list must explain itself");
        assert!(notice.contains("connection refused"), "{notice}");
        assert!(notice.contains("may be incomplete"), "{notice}");
        // Still usable: the operator gets something to click, just not a claim.
        assert_eq!(
            dto["effectiveToolkits"],
            json!(composio_toolkits::FALLBACK_TOOLKITS)
        );
    }

    /// A credential change drops the cached catalog: a rotated BYO token can
    /// resolve to a different Composio account, and serving the previous
    /// account's list for the rest of the TTL is the stale-by-construction
    /// answer this issue is about.
    ///
    /// Driven through a *clear* rather than a set so the status re-read at the
    /// end of the write has no credential to dial with — the assertion is about
    /// the eviction, and a test that reached the network to make it would be a
    /// different test.
    #[tokio::test]
    async fn a_credential_change_evicts_the_cached_catalog() {
        let home_dir = home();
        let state = state_with_manifest_id(
            home_dir.path(),
            "rotateco",
            "[company]\nname = \"Rotate Co\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n",
        )
        .await;
        let key = super::catalog_cache_key(runtime_of(&state, "rotateco").as_ref());
        composio_toolkits::cache().store(&key, Ok(hundred_entries()), std::time::Instant::now());

        let (status, resp, raw) = send_for(
            &state,
            "rotateco",
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": "" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_ne!(
            resp["status"]["catalogSource"], "backend",
            "the pre-change catalog must not survive the change: {resp}"
        );
    }

    /// The other half of #397: a company that deliberately narrowed its belt
    /// sees exactly what it chose, and is NOT in open mode. A fix that offered
    /// the curated set to everyone would silently widen every restrictive
    /// manifest.
    #[tokio::test]
    async fn an_explicit_toolkit_list_is_offered_verbatim_and_is_not_open_mode() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
        )
        .await;

        let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(dto["openMode"], false);
        assert_eq!(dto["effectiveToolkits"], json!(["gmail"]));
        assert_eq!(dto["toolkits"], json!(["gmail"]));
    }

    /// The `credentialSource` matrix as the console consumes it, plus the
    /// write-only contract. Runs with no platform identity in the environment, so
    /// the instance contributes nothing and the company's own token is the only
    /// credential in play: `none` → `static` → `none`.
    #[tokio::test]
    async fn credential_source_tracks_the_byo_token_and_never_carries_it() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\", \"github\"]\n",
        )
        .await;

        // Initial status: granted, toolkits surfaced, no credential obtainable.
        let (status, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(dto["granted"], true);
        assert_eq!(dto["credentialSource"], "none");
        assert_eq!(dto["toolkits"], json!(["gmail", "github"]));
        assert!(dto.get("backendUrl").is_some());
        assert!(
            dto.get("token").is_none(),
            "status must never carry a token"
        );
        assert!(
            dto.get("tokenConfigured").is_none(),
            "the boolean surface is replaced by credentialSource"
        );

        // Set the write-only BYO token → the static tier.
        let (status, resp, raw) = send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(resp["status"]["credentialSource"], "static");
        assert!(!raw.contains(TOKEN), "PUT response leaked the token: {raw}");

        // GET reflects it and still never carries the token.
        let (_, dto, raw) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(dto["credentialSource"], "static");
        assert!(!raw.contains(TOKEN), "GET status leaked the token: {raw}");

        // Clearing with "" reverts to whatever the instance offers — here nothing.
        let (_, resp, _) = send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": "" })),
        )
        .await;
        assert_eq!(resp["status"]["credentialSource"], "none");
    }

    /// The hosted shape, driven through the env seam (no process mutation): a
    /// company that pasted nothing reads `attested` from the instance identity,
    /// its own TinyHumans key outranks that, its own Composio token outranks
    /// both, and with none of the three the answer is `none`.
    ///
    /// Drives the **real** resolver through a real secret store rather than a
    /// restatement of its precedence. A pure function that merely mirrored the
    /// rule would keep passing after the resolver lost a tier — which is exactly
    /// what the negative control for issue #586 caught.
    #[tokio::test]
    async fn credential_source_matrix_follows_the_resolver_precedence() {
        use crate::app::config::MapEnv;
        use crate::company::company_key;
        use crate::company::composio::store_token;

        let dir = tempfile::Builder::new()
            .prefix("oc-dto-")
            .tempdir()
            .expect("tempdir");
        let path = dir.path().join("token");
        std::fs::write(&path, "projected-instance-token").unwrap();
        let projected = MapEnv::new([(
            crate::company::credentials::TOKEN_FILE_ENV,
            path.display().to_string(),
        )]);

        /// The instance identity a given environment resolves to.
        fn source_of(
            env: &dyn crate::app::config::EnvSource,
        ) -> Option<std::sync::Arc<super::TinyhumansTokenSource>> {
            super::TinyhumansTokenSource::from_env(env).map(std::sync::Arc::new)
        }

        let home_dir = home();
        let state = state_with_manifest_id(home_dir.path(), "matrix", GRANTED).await;
        let runtime = state
            .registry()
            .get(&CompanyId::new("matrix"))
            .expect("registered");
        let secrets = runtime.secrets();
        let id = runtime.id().clone();

        // Nothing stored + a projected instance identity → attested.
        assert_eq!(
            credential_source_for(&runtime, source_of(&projected))
                .await
                .unwrap(),
            CredentialSource::Attested
        );
        // Nothing stored at all → nothing obtainable, so no tools.
        assert_eq!(
            credential_source_for(&runtime, source_of(&MapEnv::default()))
                .await
                .unwrap(),
            CredentialSource::None
        );
        // A static instance key is the static tier.
        assert_eq!(
            credential_source_for(
                &runtime,
                source_of(&MapEnv::new([(
                    crate::company::credentials::API_KEY_ENV,
                    "th_static"
                )]))
            )
            .await
            .unwrap(),
            CredentialSource::Static
        );

        // The company's own TinyHumans key outranks the instance identity — a
        // company with a key set connects providers as *itself*, not as the pod
        // it happens to run in (issue #586)…
        company_key::store_key(&id, secrets.as_ref(), "th_company")
            .await
            .unwrap();
        assert_eq!(
            credential_source_for(&runtime, source_of(&projected))
                .await
                .unwrap(),
            CredentialSource::Company
        );
        // …and is the whole credential when the instance carries none, which is
        // the case this issue exists to fix.
        assert_eq!(
            credential_source_for(&runtime, source_of(&MapEnv::default()))
                .await
                .unwrap(),
            CredentialSource::Company
        );

        // The company's own Composio token outranks everything.
        store_token(&id, secrets.as_ref(), "byo-composio")
            .await
            .unwrap();
        assert_eq!(
            credential_source_for(&runtime, source_of(&projected))
                .await
                .unwrap(),
            CredentialSource::Static
        );
        assert_eq!(
            credential_source_for(&runtime, source_of(&MapEnv::default()))
                .await
                .unwrap(),
            CredentialSource::Static
        );

        // Clearing it falls back exactly one tier, not all the way to nothing.
        store_token(&id, secrets.as_ref(), "").await.unwrap();
        assert_eq!(
            credential_source_for(&runtime, source_of(&MapEnv::default()))
                .await
                .unwrap(),
            CredentialSource::Company
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The DTO's whole surface, serialized: a tier name and non-secret routing.
    /// No token, and no token-file path either — the console never needs to know
    /// where on disk the instance's identity lives.
    #[test]
    fn the_dto_carries_a_tier_name_and_nothing_secret() {
        let dto = ComposioStatusDto {
            in_build: true,
            granted: true,
            credential_source: CredentialSource::Attested,
            backend_url: "https://api.tinyhumans.ai".to_string(),
            toolkits: vec!["gmail".to_string()],
            open_mode: false,
            effective_toolkits: vec!["gmail".to_string()],
            effective_catalog: vec![CatalogEntry::from_slug("gmail")],
            catalog_source: CatalogSource::Manifest,
            catalog_notice: None,
        };
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["credentialSource"], "attested");
        assert_eq!(json["catalogSource"], "manifest");
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            vec![
                "inBuild",
                "granted",
                "credentialSource",
                "backendUrl",
                "toolkits",
                "openMode",
                "effectiveToolkits",
                "effectiveCatalog",
                "catalogSource",
                "catalogNotice"
            ],
            "the read shape must stay exactly this: {keys:?}"
        );
    }

    /// A `*` wildcard grant must NOT count as a composio grant on the status
    /// route (mirrors the harness build gate).
    #[tokio::test]
    async fn wildcard_grant_does_not_count_as_composio() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"*\"]\n",
        )
        .await;
        let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(dto["granted"], false, "{dto}");
    }

    /// The OAuth sign-in plane is always wired into the route table (like the
    /// status route), so `POST …/composio/authorize` is never a 404. Without a
    /// usable Composio client it conflicts (`409`): on the default build because
    /// the feature is not compiled in; under the `composio` feature because no
    /// per-tenant token is configured yet. Either way the console gets a clear,
    /// non-404 signal rather than a missing route.
    #[tokio::test]
    async fn authorize_route_conflicts_without_build_or_token() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
        )
        .await;

        let (status, body, raw) = send(
            &state,
            "POST",
            "/api/v1/company/composio/authorize",
            Some(json!({ "toolkit": "gmail" })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{raw}");
        assert_eq!(body["code"], "conflict", "{body}");
    }

    // --- Who may change what the company connects through (issue #403) -------

    /// A company granting composio with one provider — the shape every
    /// authorization test below drives.
    const GRANTED: &str = "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n";

    /// The regression this issue is about: a signed-in member who is not an
    /// admin cannot change what the company's agents connect through — neither
    /// by replacing the credential they present, nor by starting an OAuth
    /// handoff that would make their own account the company's connection.
    ///
    /// Both halves matter. The reported symptom was the connect flow, but the
    /// token route is the sharper one: it repoints the company's entire tool
    /// surface at whatever account the caller controls.
    #[tokio::test]
    async fn a_member_cannot_change_what_the_company_connects_through() {
        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;
        let member = crate::server::test_support::seed_session(
            &state,
            "acme",
            crate::ports::UserRole::Member,
        )
        .await;

        let (status, body, raw) = send_as(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
            Auth::Cookie(member.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
        assert_eq!(body["code"], "forbidden", "{body}");
        assert!(
            body["error"].as_str().unwrap_or_default().contains("admin"),
            "the refusal has to say why it was refused: {body}"
        );

        let (status, body, raw) = send_as(
            &state,
            "POST",
            "/api/v1/company/composio/authorize",
            Some(json!({ "toolkit": "gmail" })),
            Auth::Cookie(member.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
        assert_eq!(body["code"], "forbidden", "{body}");

        // And the refusal is real, not merely a different status: the token
        // never landed, so the company's credential is untouched.
        let (_, dto, _) = send(&state, "GET", "/api/v1/company/composio", None).await;
        assert_eq!(
            dto["credentialSource"], "none",
            "a refused write must not have stored anything: {dto}"
        );
    }

    /// The other side of the boundary, and the reason this is a role check
    /// rather than a removal: an admin still does both things. `authorize`
    /// answering `409` here is the *build* refusing (no `composio` feature),
    /// which is exactly the point — it got past the guard and failed later, on
    /// a different axis than the member's `403`.
    #[tokio::test]
    async fn an_admin_is_unaffected() {
        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;

        let (status, resp, raw) = send(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(resp["status"]["credentialSource"], "static");

        let (status, _, raw) = send(
            &state,
            "POST",
            "/api/v1/company/composio/authorize",
            Some(json!({ "toolkit": "gmail" })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "an admin reaches the build check, not the role check: {raw}"
        );
    }

    /// Reads stay open to any member, and this pins that decision either way.
    ///
    /// Knowing *that* Gmail is connected is what lets a member understand why
    /// an agent can read mail; it carries no credential. Only the ability to
    /// change it needed an owner. If a future change decides reads are
    /// sensitive too, this test is the thing that has to be edited on purpose.
    #[tokio::test]
    async fn a_member_may_still_read_the_composio_status_and_connections() {
        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;
        let member = crate::server::test_support::seed_session(
            &state,
            "acme",
            crate::ports::UserRole::Member,
        )
        .await;

        let (status, dto, raw) = send_as(
            &state,
            "GET",
            "/api/v1/company/composio",
            None,
            Auth::Cookie(member.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
        assert_eq!(dto["granted"], true);
        assert!(
            dto.get("token").is_none(),
            "a member's read must not carry a credential either: {dto}"
        );

        // 409 (no client in this build), NOT 403 — the read is not role-gated.
        let (status, _, raw) = send_as(
            &state,
            "GET",
            "/api/v1/company/composio/connections",
            None,
            Auth::Cookie(member),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    }

    /// A machine credential may set the company's token — and is **named** in
    /// the trail when it does.
    ///
    /// This is the deliberate half of the issue's "cannot, or if it may, the
    /// write is attributed". Refusing the hosting control plane a route it
    /// already sits above (it provisions the tenant and holds its database
    /// credentials) would be ceremony, not a boundary, and it would contradict
    /// the two-principal model in `docs/spec/runtime/config.md`. What it does
    /// not get is anonymity: the entry names the tenant, so a machine-made
    /// change is as reviewable afterwards as a human one.
    #[tokio::test]
    async fn a_machine_credential_is_named_when_it_sets_the_token() {
        use crate::server::platform_auth::{
            PlatformAuthConfig, PlatformClaims, UnsignedTenantVerifier,
        };

        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED)
            .await
            .with_platform_auth(PlatformAuthConfig::new(std::sync::Arc::new(
                UnsignedTenantVerifier::new("test-platform-secret"),
            )));
        let token = UnsignedTenantVerifier::tenant_token(&PlatformClaims {
            tenant: "tenant:platform".to_string(),
            scopes: std::collections::HashSet::from(["platform".to_string()]),
            companies: None,
        });

        let (status, _, raw) = send_as(
            &state,
            "PUT",
            "/api/v1/company/composio/token",
            Some(json!({ "token": TOKEN })),
            Auth::Bearer(token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");

        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let events = runtime
            .events()
            .read_from(runtime.id(), crate::ports::types::EventSeq::new(0), 100)
            .await
            .unwrap();
        let by = events
            .iter()
            .find_map(|stored| match &stored.event {
                crate::ports::types::CompanyEvent::ToolAccessChanged { by, .. } => by.clone(),
                _ => None,
            })
            .expect("the machine write is journaled");
        assert_eq!(
            by.kind,
            crate::ports::types::ActorKind::System,
            "a machine is a machine, not a person: {by:?}"
        );
        assert_eq!(by.id, "tenant:platform", "the tenant is named: {by:?}");
    }

    /// Every accepted change to the company's tool access names the person who
    /// made it, so "how did we come to be connected through that account" is
    /// answerable afterwards.
    ///
    /// Set and clear are journaled as distinct words — one grants access and
    /// the other withdraws it, and an audit trail that conflated them would be
    /// worth little.
    #[tokio::test]
    async fn a_credential_change_records_who_made_it() {
        use crate::ports::types::{ActorKind, CompanyEvent, EventSeq};

        let home_dir = home();
        let state = state_with_manifest(home_dir.path(), GRANTED).await;

        for token in [TOKEN, ""] {
            let (status, _, raw) = send(
                &state,
                "PUT",
                "/api/v1/company/composio/token",
                Some(json!({ "token": token })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{raw}");
        }

        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let events = runtime
            .events()
            .read_from(runtime.id(), EventSeq::new(0), 100)
            .await
            .unwrap();
        let changes: Vec<(String, Option<String>)> = events
            .iter()
            .filter_map(|stored| match &stored.event {
                CompanyEvent::ToolAccessChanged {
                    change,
                    toolkit,
                    by,
                } => {
                    let by = by.as_ref().expect("an admin write is always attributed");
                    assert_eq!(by.kind, ActorKind::User, "attributed to a person");
                    assert!(!by.id.is_empty(), "the person is named");
                    Some((change.clone(), toolkit.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            changes,
            vec![
                ("credential_set".to_string(), None),
                ("credential_cleared".to_string(), None),
            ],
            "a set and a clear are distinct entries in the trail"
        );

        // The trail is an audit record, not a second place a secret lives.
        let raw = serde_json::to_string(&events).unwrap();
        assert!(!raw.contains(TOKEN), "the journal leaked the token: {raw}");
    }

    /// `GET …/composio/connections` is likewise always wired and conflicts
    /// (`409`) when there is no usable client — no build feature or no token.
    #[tokio::test]
    async fn connections_route_conflicts_without_build_or_token() {
        let home_dir = home();
        let home = home_dir.path().to_path_buf();
        let state = state_with_manifest(
            &home,
            "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[tools]\nallow = [\"composio\"]\n[tools.composio]\ntoolkits = [\"gmail\"]\n",
        )
        .await;

        let (status, body, raw) =
            send(&state, "GET", "/api/v1/company/composio/connections", None).await;
        assert_eq!(status, StatusCode::CONFLICT, "{raw}");
        assert_eq!(body["code"], "conflict", "{body}");
    }
}
