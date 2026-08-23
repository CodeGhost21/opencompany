//! Device pairing: enrolling a non-browser client as a person.
//!
//! A desktop client cannot hold a session cookie. `SameSite=Lax` means the
//! browser never sends one cross-site, and a Tauri webview is cross-site with
//! every server it talks to. The header carrier in
//! [`cookie`](super::cookie::SESSION_HEADER) is how such a client presents a
//! session — this module is how it *gets* one.
//!
//! ## The direction of the flow, and why
//!
//! The obvious shape is the OAuth device flow: the client starts, shows a code,
//! and polls until a human approves. That needs **two** secrets — a short code
//! for the human to read out and a high-entropy one for the client to poll with
//! — because a client polling with the displayed code lets anyone who glimpses
//! the screen race it and steal the credential. Two secrets means pending
//! server-side state that fits no port here.
//!
//! Turning the flow around removes the problem instead of solving it:
//!
//! 1. A **signed-in human** asks their console for a pairing code
//!    (`POST …/devices`). Authentication already happened; nothing is
//!    pending approval.
//! 2. They paste it into the desktop client.
//! 3. The client redeems it (`POST …/devices/claim`) and receives a session
//!    token exactly once.
//!
//! One secret, no polling, and the code is only ever shown to someone already
//! authenticated — where the device-flow variant shows it to an anonymous
//! starter. It also maps exactly onto
//! [`LoginCodeStore`](crate::ports::LoginCodeStore), whose
//! [`consume`](crate::ports::LoginCodeStore::consume) is already the atomic,
//! single-use redemption this needs, and whose `email` field is already known
//! at creation time here.
//!
//! ## Why pairing codes cannot be login codes
//!
//! Both live in that one store, so the two are kept apart by hashing under a
//! domain separator (see [`pair_code_hash`]) rather than by a flag. A login
//! code presented to `claim` does not hash to anything `claim` looks up, and a
//! pairing code presented to `/auth/verify` does not either — they are
//! different keyspaces, not the same keyspace with a check someone could
//! forget.
//!
//! Neither redemption is a privilege escalation in the first place (anyone
//! holding a magic link can sign in and then pair), but interchangeable
//! credentials are the kind of thing that stops being harmless after the next
//! feature.
//!
//! ## What a paired device is
//!
//! Not a separate credential system: a [`SessionRecord`] with
//! [`SessionKind::Device`], a label, and a longer
//! [TTL](super::token::DEVICE_TTL_MILLIS). Everything that revokes a session
//! therefore revokes a device — suspension, admin password reset, and the
//! user's own password change all call
//! [`delete_for_user`](crate::ports::SessionStore::delete_for_user) or iterate
//! the same list, with no device-specific path to remember.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::error::OpenCompanyError;
use crate::ports::{SessionKind, generate_id, now_millis};
use crate::server::error::ApiError;
use crate::server::users::routes::{current_user, no_session};
use crate::server::users::scope::{PublicCompany, public_scoped};
use crate::server::users::token;

/// How long a pairing code stays redeemable.
///
/// Shorter than a login code's window: a magic link has to survive mail
/// delivery and a distracted human, while a pairing code is copied from one
/// window to another in the same sitting. The code is 256 bits either way, so
/// this is not a brute-force budget — it is how long a code left on screen, or
/// in a clipboard, stays live.
pub const DEVICE_PAIR_TTL_MILLIS: u64 = 5 * 60 * 1000;

/// The maximum length of a client-supplied device label kept on the record.
const MAX_LABEL_CHARS: usize = 200;

/// Hashes a pairing code into its own keyspace.
///
/// The domain prefix is what keeps pairing codes and login codes from being
/// interchangeable while sharing one store. It must never be dropped "because
/// the hash is the same anyway" — that sameness is precisely the bug.
fn pair_code_hash(code: &str) -> String {
    token::sha256_hex(&format!("device-pair:{code}"))
}

pub fn router() -> Router<AppState> {
    public_scoped("/devices", get(list_devices).post(start_pairing))
        .merge(public_scoped("/devices/claim", post(claim_device)))
        .merge(public_scoped("/devices/{deviceId}", delete(revoke_device)))
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingCode {
    /// The plaintext code, returned exactly once to the signed-in human who
    /// asked for it. Only its hash is stored.
    code: String,
    expires_at_millis: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimDevice {
    code: String,
    /// A human-chosen name for this machine. Untrusted, display-only.
    #[serde(default)]
    label: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimedDevice {
    /// The session token, returned exactly once. The client stores it in the
    /// OS keychain and presents it in
    /// [`SESSION_HEADER`](super::cookie::SESSION_HEADER).
    token: String,
    /// The company this credential is for. Returned so the client can build the
    /// `<company>.<token>` header value without having to know how the addressed
    /// company was resolved — it may have come from the single-company alias.
    company: String,
    device_id: String,
    expires_at_millis: u64,
}

/// A paired device as shown to its owner. Carries no secret.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSummary {
    id: String,
    label: Option<String>,
    created_at_millis: u64,
    expires_at_millis: u64,
    /// Whether this is the device making the request, so a client can avoid
    /// offering to revoke the credential it is currently using without warning.
    current: bool,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

/// Mints a pairing code for the signed-in user.
///
/// Deliberately available to any member, not just admins: pairing enrolls a
/// machine as *yourself*, granting nothing you do not already have. Requiring an
/// admin would make an admin the bottleneck for a self-service action, and would
/// not add a check that means anything.
async fn start_pairing(
    company: PublicCompany,
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::server::graphql::auth::MaybePeer(peer): crate::server::graphql::auth::MaybePeer,
) -> Result<Json<PairingCode>, crate::server::Rejection> {
    let runtime = company.runtime.clone();
    let Some(principal) = current_user(&headers, &state, runtime.id(), peer).await else {
        return Err(no_session().into());
    };

    // A device may not mint a pairing code. Otherwise one compromised desktop
    // could quietly enrol further machines that survive revoking it — the
    // credential would reproduce, and revocation would stop being a lever. Only
    // a browser session, which expires on its own, can start a pairing.
    if principal.is_device() {
        return Err(ApiError(OpenCompanyError::InvalidRequest(
            "a paired device cannot pair another device; sign in on the web console".to_string(),
        ))
        .into_response());
    }

    let now = now_millis();
    let code = token::mint_login_code(&token::OsTokens);
    let record = crate::ports::LoginCodeRecord {
        id: generate_id(),
        code_hash: pair_code_hash(&code),
        email: principal.email.clone(),
        created_at_millis: now,
        expires_at_millis: now + DEVICE_PAIR_TTL_MILLIS,
        consumed_at_millis: None,
    };
    runtime
        .login_codes()
        .create(runtime.id(), &record)
        .await
        .map_err(|e| ApiError(e).into_response().into())?;

    Ok(Json(PairingCode {
        code,
        expires_at_millis: record.expires_at_millis,
    }))
}

/// Redeems a pairing code for a device session.
///
/// Unauthenticated by necessity — the client has no credential yet, which is
/// the whole point. The code is the credential: 256 bits, single-use through
/// [`consume`](crate::ports::LoginCodeStore::consume), and short-lived. That is
/// the same argument the magic link rests on.
async fn claim_device(
    company: PublicCompany,
    Json(body): Json<ClaimDevice>,
) -> Result<Json<ClaimedDevice>, crate::server::Rejection> {
    let runtime = company.runtime.clone();
    let id = runtime.id();
    let now = now_millis();

    // Atomic: two clients racing the same code cannot both be enrolled.
    let Some(record) = runtime
        .login_codes()
        .consume(id, &pair_code_hash(&body.code), now)
        .await
        .map_err(|e| ApiError(e).into_response().into())?
    else {
        return Err(invalid_pairing_code().into());
    };

    // Re-read the user at redemption. The code may have been minted minutes
    // before the account was suspended or removed, and a code in flight must
    // not outlive the account it names.
    let user = runtime
        .users()
        .find_user_by_email(id, &record.email)
        .await
        .map_err(|e| ApiError(e).into_response().into())?
        .filter(|u| u.status == crate::ports::UserStatus::Active)
        .ok_or_else(invalid_pairing_code)?;

    let label = body
        .label
        .map(|l| l.chars().take(MAX_LABEL_CHARS).collect::<String>())
        .filter(|l| !l.trim().is_empty());

    let token = super::routes::create_session(&runtime, &user, SessionKind::Device, label, None)
        .await
        .map_err(|e| ApiError(e).into_response().into())?;

    // Find the record just written, for its id and expiry. Looking it up by
    // hash rather than threading it back keeps `create_session` returning only
    // the secret, so no caller is tempted to log the record beside it.
    let hash = token::sha256_hex(&token);
    let session = runtime
        .sessions()
        .find_by_token_hash(id, &hash)
        .await
        .map_err(|e| ApiError(e).into_response().into())?
        .ok_or_else(|| {
            ApiError(OpenCompanyError::Store(
                "the device session vanished immediately after it was written".to_string(),
            ))
            .into_response()
        })?;

    Ok(Json(ClaimedDevice {
        token,
        company: id.as_ref().to_string(),
        device_id: session.id,
        expires_at_millis: session.expires_at_millis,
    }))
}

/// Lists the signed-in user's paired devices.
async fn list_devices(
    company: PublicCompany,
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::server::graphql::auth::MaybePeer(peer): crate::server::graphql::auth::MaybePeer,
) -> Result<Json<Vec<DeviceSummary>>, crate::server::Rejection> {
    let runtime = company.runtime.clone();
    let Some(principal) = current_user(&headers, &state, runtime.id(), peer).await else {
        return Err(no_session().into());
    };
    let sessions = runtime
        .sessions()
        .list_for_user(runtime.id(), &principal.user_id)
        .await
        .map_err(|e| ApiError(e).into_response().into())?;

    Ok(Json(
        sessions
            .into_iter()
            // Browser sessions are listed by the sessions surface; this one is
            // about machines a person deliberately enrolled.
            .filter(|s| s.kind.is_device())
            .map(|s| DeviceSummary {
                current: s.token_hash == principal.session_token_hash,
                id: s.id,
                label: s.label,
                created_at_millis: s.created_at_millis,
                expires_at_millis: s.expires_at_millis,
            })
            .collect(),
    ))
}

/// Revokes one paired device.
///
/// Scoped to the caller's own devices: the lookup is over
/// `list_for_user(principal.user_id)`, so an id belonging to somebody else is
/// simply not found. That is the same shape as every other per-user surface
/// here — an ownership check that cannot be skipped because the query never saw
/// the other rows.
async fn revoke_device(
    company: PublicCompany,
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::server::graphql::auth::MaybePeer(peer): crate::server::graphql::auth::MaybePeer,
    Path(params): Path<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, crate::server::Rejection> {
    let runtime = company.runtime.clone();
    let Some(principal) = current_user(&headers, &state, runtime.id(), peer).await else {
        return Err(no_session().into());
    };
    let device_id = params.get("deviceId").cloned().unwrap_or_default();

    let sessions = runtime
        .sessions()
        .list_for_user(runtime.id(), &principal.user_id)
        .await
        .map_err(|e| ApiError(e).into_response().into())?;
    let Some(target) = sessions
        .into_iter()
        .find(|s| s.id == device_id && s.kind.is_device())
    else {
        return Err(ApiError(OpenCompanyError::InvalidRequest(
            "no such device".to_string(),
        ))
        .into_response());
    };

    let removed = runtime
        .sessions()
        .delete(runtime.id(), &target.id)
        .await
        .map_err(|e| ApiError(e).into_response().into())?;
    Ok(Json(serde_json::json!({ "revoked": removed })))
}

/// One indistinguishable failure for every way a claim can fail.
///
/// Unknown code, expired code, already-redeemed code, suspended user, removed
/// user — all the same response. Separating them would tell an anonymous caller
/// which codes once existed and which accounts are live, and this route is
/// reachable without any credential at all.
fn invalid_pairing_code() -> Response {
    ApiError(OpenCompanyError::InvalidRequest(
        "invalid or expired pairing code".to_string(),
    ))
    .into_response()
}
