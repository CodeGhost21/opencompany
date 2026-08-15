//! GraphQL request context: the shared auth principal and its authorization
//! helpers.
//!
//! [`GqlAuth`] is the single principal type both the REST extractors in
//! [`platform_auth`](crate::server::platform_auth) and the GraphQL handler
//! resolve to. [`resolve_claims`] is the one place a bearer header is turned
//! into a principal; the REST extractors ([`PlatformOrOperatorAuth`],
//! [`PlatformScope`]) map its output onto their existing `Option<PlatformClaims>`
//! shape so their behavior is byte-for-byte unchanged.
//!
//! ## Why a human session has two carriers
//!
//! A session cookie is `SameSite=Lax`, which means the browser will not attach
//! it to a cross-site request no matter what CORS says — and a desktop webview
//! is cross-site with every server it talks to. Adding an allowed origin does
//! not fix that; the cookie is simply never sent.
//!
//! So a non-browser client had no way to authenticate *as a person* at all. The
//! only header credential is the platform bearer, and
//! [`ScopedCompany`](crate::server::ops::scope::ScopedCompany) maps that to
//! `actor: None` — every write it makes lands in the journal with nobody's name
//! on it. A desktop built on the platform bearer would be permanently
//! anonymous, which
//! [`AdminScopedCompany`](crate::server::ops::scope::AdminScopedCompany) argues
//! at length is unacceptable.
//!
//! Hence the header carrier: the same token, the same TTL, the same revocation,
//! resolving to the same `GqlAuth::User`. Only the envelope differs.
//!
//! **The header is accepted, but nothing here issues one.** A session token
//! reaches a client only as `Set-Cookie`, where `HttpOnly` keeps it away from
//! JavaScript. Handing the raw token to a browser would defeat that, so no
//! route does; the device-pairing flow is the intended issuer for clients that
//! need the header form.

use async_graphql::ErrorExtensions;
use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::request::Parts;

use crate::AppState;
use crate::ports::types::CompanyId;
use crate::ports::{UserRole, UserStatus};
use crate::server::platform_auth::{PlatformClaims, bearer};

/// The request's TCP peer, when [`axum::serve`] was wired with
/// `into_make_service_with_connect_info` — `None` otherwise (an embedded
/// caller with no real socket, or a test exercising a router directly). Unlike
/// [`axum::extract::ConnectInfo`], never fails to extract: a handler naming
/// this as a parameter works identically whether connect info is wired or not,
/// which is what lets [`local_owner`]'s peer gate be additive rather than a
/// hard requirement on every caller.
pub(crate) struct MaybePeer(pub(crate) Option<std::net::SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for MaybePeer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybePeer(
            parts
                .extensions
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

/// A human collaborator, authenticated by a session cookie.
///
/// Scoped to exactly one company: unlike a platform tenant, which may own
/// several, a user *is* a member of one company and a session is minted for
/// that company alone.
#[derive(Clone, Debug)]
pub struct UserPrincipal {
    /// The company this session was minted in. Valid nowhere else.
    pub company: CompanyId,
    /// The authenticated [`UserRecord::id`](crate::ports::UserRecord).
    pub user_id: String,
    /// The user's normalized email.
    pub email: String,
    /// What the user may do inside their company.
    pub role: UserRole,
    /// Whether an admin issued a temporary password this user must replace.
    ///
    /// Carried on the principal so the check is a boundary at the extractor
    /// rather than a convention the console is trusted to honor.
    pub must_change_password: bool,
    /// The session's token hash, so logout can revoke exactly this session.
    pub session_token_hash: String,
    /// Whether a browser or a paired device presented this session.
    ///
    /// ## Why this is a field and not a `GqlAuth::Device` variant
    ///
    /// A device is the *same person* with the same rights, reached from a
    /// different machine — so the default must be that everything a browser
    /// session may do, a device may do. A new enum variant inverts that: every
    /// one of the ~15 `GqlAuth::` match sites would have to name devices
    /// explicitly, and each site the author forgets or wildcards is a decision
    /// made by accident.
    ///
    /// The direction of that accident is what settles it. As a field, a device
    /// is a `GqlAuth::User`, so
    /// [`PlatformScope`](crate::server::platform_auth)'s existing
    /// `GqlAuth::User(_) => forbidden()` already refuses it — **fail-closed,
    /// with no new code**. As a variant, a forgotten arm somewhere on the
    /// platform plane fails *open*, and it would fail open silently.
    ///
    /// Routes that genuinely need to treat a device differently — refusing to
    /// let one change the account password, say — ask [`Self::is_device`]. That
    /// is an opt-in restriction on a small number of routes, rather than an
    /// opt-in permission on all of them.
    pub credential: crate::ports::SessionKind,
}

impl UserPrincipal {
    /// Whether this user may invite, revoke, and remove other users.
    pub fn may_administer(&self) -> bool {
        self.role.may_administer()
    }

    /// Whether a paired device, rather than a browser, presented this session.
    pub fn is_device(&self) -> bool {
        self.credential.is_device()
    }
}

/// The authenticated principal for a request.
///
/// - `Platform`: a verified platform/tenant token — the *machine* credential,
///   used by the hosting layer.
/// - `User`: a human collaborator with a session cookie, scoped to one company.
///
/// There is no unauthenticated principal. There used to be `Dev` (no operator
/// token configured ⇒ allow everything) and `Operator` (a shared bearer), but
/// the operator token could never actually be set, so `Dev` was the only
/// reachable state and every deployment served every route to anyone. Humans
/// authenticate as themselves now; nothing is open.
#[derive(Clone, Debug)]
pub enum GqlAuth {
    /// Platform mode with verified tenant claims.
    Platform(PlatformClaims),
    /// A human collaborator of exactly one company.
    User(UserPrincipal),
}

/// The one failure [`resolve_claims`] can return: an unauthenticated request.
/// The REST extractors map this to `401`; the GraphQL handler to an error.
#[derive(Clone, Copy, Debug)]
pub struct Unauthorized;

/// Turns an `Authorization` header into a **machine** [`GqlAuth`] principal.
///
/// Only platform mode has machine credentials: the bearer must verify, else
/// [`Unauthorized`]. Without `platform_auth` configured there is no machine
/// credential to present, so this always fails and callers must come in as a
/// human via [`resolve_principal`].
///
/// **It can never return [`GqlAuth::User`], and that is load-bearing.** Routes
/// that mean to serve only the hosting layer (provisioning, suspension) resolve
/// through this and therefore cannot be reached by a session cookie, no matter
/// what it contains — not because each route checks, but because the type it
/// receives cannot represent a human.
pub fn resolve_claims(headers: &HeaderMap, state: &AppState) -> Result<GqlAuth, Unauthorized> {
    let platform = state.config().platform_auth.as_ref().ok_or(Unauthorized)?;
    let token = bearer(headers).ok_or(Unauthorized)?;
    let claims = platform.verifier.verify(token).map_err(|_| Unauthorized)?;
    Ok(GqlAuth::Platform(claims))
}

/// Resolves the full principal: a valid session wins, else the machine
/// credentials from [`resolve_claims`].
///
/// `company` is the addressed company when the caller knows it (the REST
/// routes, from the path or the sole registered company). Pass `None` when it
/// is not knowable at resolution time — the GraphQL handler's company argument
/// lives in the request body — and the carrier selects it: the cookie *name*,
/// or the company embedded in the header value. With several session cookies
/// present and no addressed company (only reachable in local dev, where one
/// origin serves many companies) no user is resolved, because guessing which
/// one the request meant would be worse than degrading.
///
/// A present-but-invalid session falls through to the bearer path rather than
/// failing the request: a stale cookie must not brick an operator sharing the
/// origin.
///
/// `peer` is the request's TCP peer, when the caller can name one — see
/// [`local_owner`] for why it matters and why its absence is not itself a
/// refusal.
///
/// **This is the only function that can produce a human principal**, and every
/// route meaning to serve people must resolve through it rather than
/// [`resolve_claims`].
pub async fn resolve_principal(
    headers: &HeaderMap,
    state: &AppState,
    company: Option<&CompanyId>,
    peer: Option<std::net::SocketAddr>,
) -> Result<GqlAuth, Unauthorized> {
    if let Some(owner) = local_owner(state, company, peer, headers).await {
        return Ok(GqlAuth::User(owner));
    }
    if let Some(user) = resolve_session(headers, state, company).await {
        return Ok(GqlAuth::User(user));
    }
    resolve_claims(headers, state)
}

/// The implicit owner of a company whose [`AuthMode`] is `None`, if that is what
/// this request addresses.
///
/// `None` mode is the packaged desktop app: a loopback host, one person, no
/// sign-in. There is no credential to present, so the principal is not resolved
/// *from* the request at all — everything that reaches this company is that one
/// person, by configuration.
///
/// It is still a real [`UserRecord`], materialized on first use, rather than a
/// principal invented per request. Chat attribution, task assignment, the audit
/// trail and the admin surfaces all key off
/// [`UserRecord::id`](crate::ports::UserRecord), and a synthetic id that no store
/// row backs would come apart at the first one of them that looked the user up.
///
/// This is checked **before** the session and bearer paths, and that order is
/// the point: in `none` mode there is no session to find and no bearer to
/// verify, so falling through would produce an unauthenticated request against a
/// host whose whole premise is that its only caller is its owner.
///
/// `peer` and `headers` are two independent gates, on top of the bind-time
/// refusal ([`serve_on`](crate::server::routes::serve_on)) that refuses to
/// boot or provision a `none`-mode company on a routable bind. Neither
/// substitutes for it — a deployment-time statement about the *configured*
/// bind is the only one of the three that can refuse before a company is ever
/// live — and neither substitutes for the other:
///
/// - **`peer`**, when the caller can name the request's TCP peer, refuses a
///   non-loopback one. This catches a directly reachable socket — e.g. a bug
///   in a future registration path that lands `none` mode on a routable bind
///   without going through the guard above.
/// - **`headers`** refuses a request carrying `X-Forwarded-For`,
///   `X-Forwarded-Host`, or RFC 7239 `Forwarded`. Their presence means
///   something in front of this process terminated a connection this process
///   did not — which is exactly the case `peer` cannot see: a same-host
///   reverse proxy connects to a loopback-bound listener over loopback too, so
///   the peer it presents always reads as loopback regardless of where the
///   original client actually was. An *undeclared* proxy in front of an
///   otherwise-correctly-loopback-bound host is the likeliest way to end up
///   here by accident, and this is the one of the three gates that catches it.
///
/// When neither signal is present (an embedded caller with no real socket and
/// no forwarding in front of it, or a test), neither refuses on its own — the
/// bind-time guard is still the one that ran, and both of these are additive
/// to it, not a replacement.
pub(crate) async fn local_owner(
    state: &AppState,
    company: Option<&CompanyId>,
    peer: Option<std::net::SocketAddr>,
    headers: &HeaderMap,
) -> Option<UserPrincipal> {
    // With no addressed company (the GraphQL handler, whose company argument is
    // in the request body) fall back to the sole registered one. A `none`-mode
    // host serves exactly one company — that is what the desktop app is — so
    // there is no second candidate to guess between.
    let runtime = match company {
        Some(id) => state.registry().get(id)?,
        None => state.registry().sole()?,
    };
    if runtime.auth_mode().has_login() {
        return None;
    }
    if let Some(peer) = peer
        && !peer.ip().is_loopback()
    {
        tracing::warn!(
            company = %runtime.id(),
            peer = %peer,
            "refused to resolve the none-mode local owner for a non-loopback peer"
        );
        return None;
    }
    const FORWARDING_HEADERS: [&str; 3] = ["x-forwarded-for", "x-forwarded-host", "forwarded"];
    if let Some(name) = FORWARDING_HEADERS
        .iter()
        .find(|name| headers.contains_key(**name))
    {
        tracing::warn!(
            company = %runtime.id(),
            header = %name,
            "refused to resolve the none-mode local owner for a request carrying a \
             proxy-forwarding header"
        );
        return None;
    }
    let user = match crate::server::users::routes::local_owner_record(&runtime).await {
        Ok(user) => user,
        Err(error) => {
            // Swallowed to `None` rather than propagated — `local_owner` composes
            // with `resolve_session` and `resolve_claims` as three strategies
            // that each degrade to "try the next one", not three that can fail
            // the request outright. But silent is not the same as safe: without
            // this, a store outage here reads identically to "not `none` mode"
            // and an operator has nothing to distinguish them by.
            tracing::warn!(
                company = %runtime.id(),
                %error,
                "could not materialize the none-mode local owner"
            );
            return None;
        }
    };
    Some(UserPrincipal {
        company: runtime.id().clone(),
        user_id: user.id,
        email: user.email,
        role: user.role,
        // No admin can set a temporary password here — there is no admin but
        // this person, and no password at all.
        must_change_password: false,
        // No session was presented and none exists, so there is nothing for
        // logout to revoke. The logout route refuses in this mode rather than
        // pretending, which is why an empty hash cannot silently match a real
        // session: no lookup is ever made with it.
        session_token_hash: String::new(),
        credential: crate::ports::SessionKind::Browser,
    })
}

/// Resolves a session — from either carrier — to a live user, or `None`.
///
/// Returns `None` — never an error — for every failure: no session presented,
/// unknown token, expired session, vanished user, suspended user. Callers fall
/// back to machine credentials.
async fn resolve_session(
    headers: &HeaderMap,
    state: &AppState,
    company: Option<&CompanyId>,
) -> Option<UserPrincipal> {
    let (company, token) = session_carrier(headers, company)?;
    authenticate_session(state, company, &token).await
}

/// Extracts the company and raw token a request presents a session with.
///
/// The header wins over the cookie when both are present and usable. A desktop
/// client sends only the header and a browser sends only the cookie, so the
/// order matters in exactly one case — a webview that has somehow acquired
/// both — where the explicit carrier should beat the ambient one.
///
/// A header naming a company other than the addressed one falls through to the
/// cookie rather than failing the request, matching the existing policy for a
/// present-but-invalid cookie: a credential for somewhere else must not brick a
/// request that had a valid one all along.
fn session_carrier(
    headers: &HeaderMap,
    company: Option<&CompanyId>,
) -> Option<(CompanyId, String)> {
    use crate::server::users::cookie::{
        company_from_cookie_name, parse_cookies, session_from_header,
    };

    if let Some((named, token)) = session_from_header(headers)
        && company.is_none_or(|addressed| *addressed == named)
    {
        return Some((named, token));
    }

    let cookies = parse_cookies(headers);
    // Resolve which company's cookie to read: the addressed one when known,
    // else the sole session cookie present.
    match company {
        Some(id) => {
            let name = crate::server::users::cookie::session_cookie_name(id)?;
            Some((id.clone(), cookies.get(&name)?.clone()))
        }
        None => {
            let mut sessions = cookies
                .iter()
                .filter_map(|(name, value)| {
                    company_from_cookie_name(name).map(|id| (CompanyId::new(id), value.clone()))
                })
                .collect::<Vec<_>>();
            // Exactly one, or we cannot know which company was meant.
            if sessions.len() != 1 {
                return None;
            }
            Some(sessions.remove(0))
        }
    }
}

/// Turns a `(company, raw token)` pair into a live [`UserPrincipal`].
///
/// The single authentication path for every carrier. Kept separate from
/// [`session_carrier`] so that adding a carrier cannot accidentally add a
/// *weaker* check alongside it — liveness, the re-read, and the status gate
/// below are reached by construction rather than by each caller remembering.
async fn authenticate_session(
    state: &AppState,
    company: CompanyId,
    token: &str,
) -> Option<UserPrincipal> {
    let runtime = state.registry().get(&company)?;
    let token_hash = crate::server::users::token::sha256_hex(token);
    // Lookup is *by* hash and scoped to the company: a session minted for
    // another company simply is not in this partition.
    let session = runtime
        .sessions()
        .find_by_token_hash(&company, &token_hash)
        .await
        .ok()??;
    let now = crate::ports::now_millis();
    if !session.is_live(now) {
        return None;
    }
    // Re-read the user on every request. This is what makes suspension and
    // removal take effect immediately rather than whenever the cookie happens
    // to expire — the cost is a second store read per authenticated request.
    let user = runtime
        .users()
        .get_user(&company, &session.user_id)
        .await
        .ok()??;
    if user.status != UserStatus::Active {
        return None;
    }
    Some(UserPrincipal {
        company,
        user_id: user.id,
        email: user.email,
        role: user.role,
        must_change_password: user.must_change_password,
        session_token_hash: token_hash,
        // From the stored record, not from the carrier. A browser session
        // replayed as a header is still a browser session, and a device that
        // somehow arrived in a cookie is still a device — what the credential
        // *is* was decided when it was minted.
        credential: session.kind,
    })
}

impl GqlAuth {
    /// Authorizes addressing a specific company under this principal.
    ///
    /// Dev/operator and platform-scope principals may address any company; a
    /// tenant principal may address a company only when it owns it (the registry
    /// ownership map records `id -> tenant`) and its own allow-list permits it.
    /// Mirrors [`authorize_address`](crate::server::platform_auth::authorize_address).
    pub fn authorize(&self, state: &AppState, company: &CompanyId) -> async_graphql::Result<()> {
        match self {
            GqlAuth::Platform(claims) => {
                if claims.has_platform_scope() {
                    return Ok(());
                }
                let owner = state.owner_of(company);
                if owner.as_deref() == Some(crate::app::canonical_tenant(&claims.tenant))
                    && claims.may_address(company)
                {
                    Ok(())
                } else {
                    Err(forbidden())
                }
            }
            // A user belongs to one company and may address that one only. The
            // storage partition already makes a cross-company session
            // unresolvable; this is the second, explicit line of defense.
            GqlAuth::User(user) => {
                if user.company == *company {
                    Ok(())
                } else {
                    Err(forbidden())
                }
            }
        }
    }

    /// The companies this principal may see, filtered from the registry.
    ///
    /// Dev/operator and platform-scope principals see every registered company;
    /// a tenant principal sees only the companies it owns and may address.
    pub fn visible_companies(&self, state: &AppState) -> Vec<CompanyId> {
        let all = state.registry().list();
        match self {
            GqlAuth::Platform(claims) if claims.has_platform_scope() => all,
            GqlAuth::Platform(claims) => all
                .into_iter()
                .filter(|id| {
                    state.owner_of(id).as_deref()
                        == Some(crate::app::canonical_tenant(&claims.tenant))
                        && claims.may_address(id)
                })
                .collect(),
            // A user sees their own company and nothing else — not even that
            // other companies exist on this host.
            GqlAuth::User(user) => all.into_iter().filter(|id| *id == user.company).collect(),
        }
    }
}

/// A `403`-equivalent GraphQL error carrying `extensions.code = "forbidden"`.
pub fn forbidden() -> async_graphql::Error {
    async_graphql::Error::new("forbidden").extend_with(|_, e| e.set("code", "forbidden"))
}

/// A GraphQL error marking a resolver whose backing surface is not yet wired,
/// carrying `extensions.code = "NOT_WIRED"`. The console's bare-catch treats it
/// like a `404` and falls back to its sample.
pub fn not_wired(field: &str) -> async_graphql::Error {
    async_graphql::Error::new(format!("{field} is not wired"))
        .extend_with(|_, e| e.set("code", "NOT_WIRED"))
}
