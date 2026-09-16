use super::{
    CatalogEntry, CatalogSource, ComposioMode, ComposioStatusDto, CredentialSource,
    TinyhumansTokenSource, access_for,
};
use crate::company::runtime::CompanyRuntime;
use crate::server::error::ApiError;
use crate::server::ops::composio_toolkits;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
#[cfg(feature = "composio")]
use axum::routing::post;
#[cfg(feature = "composio")]
use axum::{Json, Router};
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

/// A loopback Composio authorize endpoint for the feature-enabled route
/// test. Keeping the HTTP boundary real proves the handler reaches the
/// client after the admin guard, without allowing a unit test to dial the
/// production backend.
#[cfg(feature = "composio")]
async fn spawn_authorize_backend() -> String {
    let app = Router::new().route(
        "/agent-integrations/composio/authorize",
        post(|Json(body): Json<Value>| async move {
            assert_eq!(body["toolkit"], "gmail");
            Json(json!({
                "success": true,
                "data": {
                    "connectUrl": "https://composio.test/connect/gmail",
                    "connectionId": "gmail-connection"
                }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
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
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
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
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
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

/// A managed-token clear needs no confirmation while the company is on
/// BYOK: `composio/mode` no longer selects the managed slot, so this
/// clear is not touching what any live call resolves through, and the
/// response carries no `usedBy` — omitted, not null or empty.
#[tokio::test]
async fn clearing_the_managed_token_while_mode_is_byok_needs_no_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "clearunguarded", GRANTED).await;
    let runtime = runtime_of(&state, "clearunguarded");
    crate::company::composio::store_token(runtime.id(), runtime.secrets().as_ref(), TOKEN)
        .await
        .unwrap();
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "clearunguarded",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "{body}");
    assert_eq!(
        crate::company::composio::load_tinyhumans_key(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        None
    );
}

/// Setting or rotating a non-empty managed token is never guarded — only
/// a clear can strand anything the mode currently resolves through.
#[tokio::test]
async fn setting_a_managed_token_is_never_guarded_even_while_mode_is_managed() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "setnoguard", GRANTED).await;

    let (status, body, raw) = send_for(
        &state,
        "setnoguard",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "a set is not guarded: {body}");
}

/// The first move onto BYOK needs no confirmation from Composio's own
/// guard: before the write `composio/mode` still reads `"managed"`, so
/// `composio/byok/key` is not the slot the mode currently selects, and
/// setting it strands nothing (in-use-guards.md §2 — the criterion is
/// the mode BEFORE the write, not the one the write is heading to). The
/// probe is forced clean so this does not depend on network access under
/// the `composio` feature.
#[tokio::test]
async fn switching_to_byok_while_mode_is_managed_needs_no_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "byokswitchunguarded", GRANTED).await;
    let runtime = runtime_of(&state, "byokswitchunguarded");
    super::probe_override::set("byokswitchunguarded", Ok(()));

    let (status, body, raw) = send_for(
        &state,
        "byokswitchunguarded",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(body.get("usedBy").is_none(), "{body}");
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// Round-2 review, comment 4012457339 (keys rework #2306): `set_api_key`
/// writes `composio/mode`, the same fact `company_key::fan_out`'s own
/// in-use check reads to decide whether an unconfirmed account-key clear
/// may touch `composio/tinyhumans/key`. Before this fix the route took no
/// lock at all, so a confirmed mode switch landing here and a concurrent,
/// unconfirmed account-key clear could each act on the OTHER's pre-image
/// — the clear sees the old mode and decides the managed slot is
/// inactive, the switch then makes it active, and it is left keyless with
/// neither request ever having confirmed that outcome.
///
/// This proves the route now shares `company_key::fan_out`'s own
/// `slot_guard`: while a test holds that company's guard directly (the
/// same acquisition `fan_out` makes for the whole of an account-key
/// save), the real `PUT …/composio/api-key` handler must not complete —
/// it has to be waiting on the same lock, not racing past it.
#[tokio::test]
async fn set_api_key_blocks_while_the_account_keys_fan_out_lock_is_held() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "composio-lock-blocks", GRANTED).await;
    let runtime = runtime_of(&state, "composio-lock-blocks");
    super::probe_override::set("composio-lock-blocks", Ok(()));

    // The exact acquisition `company_key::fan_out` makes for the whole of
    // an account-key save.
    let held = crate::company::company_key::slot_guard(runtime.id()).await;

    let request = send_for(
        &state,
        "composio-lock-blocks",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    );
    tokio::pin!(request);
    tokio::select! {
        _ = &mut request => panic!(
            "set_api_key must not write composio/mode while the account-key \
             fan-out's own lock is held elsewhere"
        ),
        _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
    }

    drop(held);
    let (status, _body, raw) =
        tokio::time::timeout(std::time::Duration::from_millis(1000), request)
            .await
            .expect("set_api_key proceeds once the lock is released");
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// Giving the managed route back while BYOK is active IS refused without
/// confirmation: `composio/mode` already reads `"byok"`, which is
/// exactly the slot `composio/byok/key` being cleared belongs to.
#[tokio::test]
async fn switching_back_to_managed_while_mode_is_byok_is_refused_without_confirmation() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "managedswitchguard", GRANTED).await;
    let runtime = runtime_of(&state, "managedswitchguard");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "managedswitchguard",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "in_use", "{body}");
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );

    // Refused: the company is still on BYOK, key untouched.
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Byok
    );
}

/// The same switch back to managed, confirmed, lands and echoes
/// `usedBy` (in-use-guards.md §3).
#[tokio::test]
async fn a_confirmed_switch_back_to_managed_succeeds_and_echoes_used_by() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "managedswitchconfirmed", GRANTED).await;
    let runtime = runtime_of(&state, "managedswitchconfirmed");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();

    let (status, body, raw) = send_for(
        &state,
        "managedswitchconfirmed",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["usedBy"],
        json!({ "surfaces": ["composio"] }),
        "{body}"
    );
    assert_eq!(
        crate::company::composio::load_mode(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap(),
        crate::company::composio::ComposioMode::Managed
    );
}

/// Rotating a key while staying on the SAME route is never guarded — a
/// rotate is not a switch at all (in-use-guards.md §2), so the mode
/// match is never even consulted.
#[tokio::test]
async fn rotating_an_already_byok_key_is_never_guarded() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "byokrotate", GRANTED).await;
    let runtime = runtime_of(&state, "byokrotate");
    crate::company::composio::store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();
    super::probe_override::set("byokrotate", Ok(()));

    let (status, body, raw) = send_for(
        &state,
        "byokrotate",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_9999999999" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        body.get("usedBy").is_none(),
        "a same-route rotation is not a switch: {body}"
    );
}
