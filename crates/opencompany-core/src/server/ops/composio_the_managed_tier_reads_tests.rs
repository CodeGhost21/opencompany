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

/// The `attested` row of the matrix, which the route cannot reach without
/// mutating the process environment.
///
/// Drives the **exact pair of calls** `effective_status` makes — `access_for`
/// for the effective tier, `resolve_credential` for the managed one — against
/// a company already switched to BYOK, through the env seam. A company whose
/// only credential is the instance identity presents its own Composio key and
/// still reports `attested` as what managed would fall back to.
#[tokio::test]
async fn the_managed_tier_reads_the_instance_identity_under_byok() {
    use crate::app::config::MapEnv;
    use crate::company::composio::{resolve_credential, store_api_key};

    let dir = tempfile::Builder::new()
        .prefix("oc-managed-tier-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-instance-token").unwrap();
    let projected = MapEnv::new([(
        crate::company::credentials::TOKEN_FILE_ENV,
        path.display().to_string(),
    )]);
    let source = super::TinyhumansTokenSource::from_env(&projected).map(std::sync::Arc::new);

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "attestedbyok", GRANTED).await;
    let runtime = runtime_of(&state, "attestedbyok");

    // Managed, with only the instance identity: both answers are `attested`.
    assert_eq!(
        access_for(&runtime, source.clone()).await.unwrap().1,
        CredentialSource::Attested
    );
    assert_eq!(
        resolve_credential(runtime.id(), runtime.secrets().as_ref(), source.clone())
            .await
            .unwrap()
            .source(),
        CredentialSource::Attested
    );

    // Switch to BYOK. The effective tier becomes the company's own Composio
    // key; the managed chain is unchanged and still answers `attested`.
    store_api_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "ak_not_a_real_key_0123456789",
    )
    .await
    .unwrap();
    let (mode, effective) = access_for(&runtime, source.clone()).await.unwrap();
    assert_eq!(mode, super::ComposioMode::Byok);
    assert_eq!(effective, CredentialSource::Static);
    assert_eq!(
        resolve_credential(runtime.id(), runtime.secrets().as_ref(), source)
            .await
            .unwrap()
            .source(),
        CredentialSource::Attested,
        "selecting BYOK must not change what the managed route would resolve to"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ── The draft-key check on `PUT …/composio/api-key` (#2275) ──────────

/// A key Composio rejects is **not stored**, and the refusal says so in the
/// classifier's own words.
///
/// There is no rollback here because there is no write: the probe runs on
/// the draft, before the store, which is the deliberate departure from the
/// inference connect flow (that one writes first because its probe resolves
/// the key by slug). The proof is the GET afterwards — the company is still
/// on the managed route.
#[tokio::test]
async fn a_rejected_key_is_refused_and_nothing_is_stored() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeauth", GRANTED).await;
    super::probe_override::set(
        "probeauth",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    let (code, body, raw) = send_for(
        &state,
        "probeauth",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "{raw}");
    assert_eq!(body["code"], "invalid_request", "{body}");
    assert!(
        body["error"]
            .as_str()
            .is_some_and(
                |message| message.contains(crate::company::composio_probe::describe(
                    crate::company::composio_probe::ComposioProbeClass::Auth
                ))
            ),
        "the refusal is the classifier's copy, not an interpolated upstream string: {body}"
    );
    assert!(
        !body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Unauthorized"),
        "the upstream reason belongs in the debug log, not in the refusal: {body}"
    );
    assert!(
        !raw.contains(BYOK_KEY),
        "a refusal must not echo the key back: {raw}"
    );

    let (_, dto, raw) =
        send_for(&state, "probeauth", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(
        dto["mode"], "managed",
        "a refused key must leave the company exactly where it was: {dto}"
    );
    assert!(!raw.contains(BYOK_KEY), "the GET leaked the key: {raw}");
}

/// A check that fails for any reason **other** than the credential keeps the
/// key: it is plausibly fine and only the connection is in question. The
/// write lands, and the response carries the class and the advisory beside
/// it.
///
/// The case in the fixture is the one the ordering rule exists for — a
/// gateway in the path — which must never read as a bad key.
#[tokio::test]
async fn a_key_that_could_not_be_checked_is_stored_with_an_advisory() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeadvise", GRANTED).await;
    super::probe_override::set("probeadvise", Err("502 Bad Gateway".to_string()));

    let (code, resp, raw) = send_for(
        &state,
        "probeadvise",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok", "the key was stored: {resp}");
    assert_eq!(resp["probeClass"], "unknown", "{resp}");
    assert_eq!(
        resp["advisory"],
        crate::company::composio_probe::describe(
            crate::company::composio_probe::ComposioProbeClass::Unknown
        )
    );
    assert!(
        !raw.contains("Bad Gateway"),
        "the upstream text belongs in the debug log, not in operator copy: {raw}"
    );
    assert!(!raw.contains(BYOK_KEY), "the PUT leaked the key: {raw}");
}

/// A clean check stores the key and says nothing extra — the two additive
/// fields are **omitted**, not null, so a consumer that predates them reads
/// the body it always did.
#[tokio::test]
async fn a_clean_check_stores_the_key_and_adds_nothing_to_the_response() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeclean", GRANTED).await;
    super::probe_override::set("probeclean", Ok(()));

    let (code, resp, raw) = send_for(
        &state,
        "probeclean",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok");
    assert!(resp.get("advisory").is_none(), "{resp}");
    assert!(resp.get("probeClass").is_none(), "{resp}");
}

/// `skipVerify` is the "add anyway" escape: a key whose check the route
/// would have refused is stored, unexamined, when the caller says so.
///
/// Defaulting matters as much as the behaviour — every test above sends no
/// `skipVerify` at all and gets the checked path, which is what pins the
/// `#[serde(default)]` false.
#[tokio::test]
async fn skip_verify_stores_a_key_the_check_would_have_refused() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeskip", GRANTED).await;
    super::probe_override::set(
        "probeskip",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    let (code, resp, raw) = send_for(
        &state,
        "probeskip",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789", "skipVerify": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "byok", "{resp}");
    assert!(
        resp.get("advisory").is_none(),
        "nothing was checked, so there is nothing to advise about: {resp}"
    );
}

/// Clearing is **never** checked. Withdrawing a credential is always
/// allowed, and a probe that could refuse a clear would strand a company on
/// a key it had already decided against.
#[tokio::test]
async fn clearing_a_key_is_never_checked() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "probeclear", GRANTED).await;
    super::probe_override::set("probeclear", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "probeclear",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "ak_not_a_real_key_0123456789" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    // Now make any probe destructive. The clear must still go through.
    // Guarded while the company is on BYOK (in-use-guards.md §2); this
    // test is about the probe never blocking a clear, not the guard, so
    // it confirms.
    super::probe_override::set(
        "probeclear",
        Err("Composio answered 401 Unauthorized".to_string()),
    );
    let (code, resp, raw) = send_for(
        &state,
        "probeclear",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["mode"], "managed", "{resp}");
    assert!(resp.get("advisory").is_none(), "{resp}");
}

// ── `POST …/composio/api-key/test` — check, never change ─────────────

/// A company on the managed route has no key of its own to check, and a
/// company in BYOK with a blank slot has none either. Both answer the same
/// permanent `not_configured`, which is what lets the console disable the
/// control instead of offering a check that can only fail.
#[tokio::test]
async fn testing_a_key_that_is_not_there_says_so_rather_than_probing() {
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testnokey", GRANTED).await;
    // Destructive if it ever ran. It must not run.
    super::probe_override::set(
        "testnokey",
        Err("Composio answered 401 Unauthorized".to_string()),
    );

    // Managed — the default, nothing stored.
    let (code, body, raw) = send_for(
        &state,
        "testnokey",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "not_configured", "{body}");

    // BYOK selected with an empty slot — the same answer, because the same
    // thing is true: there is no key here to check.
    let runtime = runtime_of(&state, "testnokey");
    runtime
        .secrets()
        .set(
            runtime.id(),
            crate::company::composio::MODE_KEY,
            crate::ports::types::SecretValue(crate::company::composio::BYOK_MODE.to_string()),
        )
        .await
        .unwrap();
    let (code, body, raw) = send_for(
        &state,
        "testnokey",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], "not_configured", "{body}");
}

/// The assertion the route exists to keep honest: a **failed** check —
/// including the destructive class — leaves the stored key and the stored
/// mode byte-identical. Testing a credential and withdrawing it are
/// separate acts, and nothing on this path may conflate them.
#[tokio::test]
async fn a_failed_check_changes_absolutely_nothing() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    use crate::company::composio::{BYOK_KEY_KEY, LEGACY_API_KEY_KEY, MODE_KEY};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testkeeps", GRANTED).await;
    super::probe_override::set("testkeeps", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "testkeeps",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    let runtime = runtime_of(&state, "testkeeps");
    async fn slots(
        runtime: &super::CompanyRuntime,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let read = |key: &'static str| async move {
            runtime
                .secrets()
                .get(runtime.id(), key)
                .await
                .unwrap()
                .map(|crate::ports::types::SecretValue(v)| v)
        };
        (
            read(MODE_KEY).await,
            read(BYOK_KEY_KEY).await,
            read(LEGACY_API_KEY_KEY).await,
        )
    }
    let before = slots(&runtime).await;

    // Now make the check fail in the ONE class that is destructive on the
    // write route, and check again.
    super::probe_override::set(
        "testkeeps",
        Err("Composio answered 401 Unauthorized".to_string()),
    );
    let (code, body, raw) = send_for(
        &state,
        "testkeeps",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], false, "{body}");
    assert_eq!(body["probeClass"], "auth", "{body}");
    assert_eq!(
        body["message"],
        crate::company::composio_probe::describe_verdict(
            crate::company::composio_probe::ComposioProbeClass::Auth
        ),
        "a check that stored nothing must not report that it saved: {body}"
    );
    assert!(
        !raw.contains(BYOK_KEY),
        "the check leaked the stored key: {raw}"
    );

    assert_eq!(
        slots(&runtime).await,
        before,
        "a failed check must leave the stored mode and key byte-identical"
    );

    // And the company is still on its own account, as the status says.
    let (_, dto, raw) =
        send_for(&state, "testkeeps", "GET", "/api/v1/company/composio", None).await;
    assert_eq!(dto["mode"], "byok", "{dto}");
    assert!(!raw.contains(BYOK_KEY), "the GET leaked the key: {raw}");
}

/// A key Composio accepts answers `ok` and nothing else — no class, no
/// sentence, and no credential.
#[tokio::test]
async fn a_clean_check_answers_ok_and_carries_no_credential() {
    const BYOK_KEY: &str = "ak_not_a_real_key_0123456789";
    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "testclean", GRANTED).await;
    super::probe_override::set("testclean", Ok(()));
    let (code, _, raw) = send_for(
        &state,
        "testclean",
        "PUT",
        "/api/v1/company/composio/api-key",
        Some(json!({ "apiKey": BYOK_KEY })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    let (code, body, raw) = send_for(
        &state,
        "testclean",
        "POST",
        "/api/v1/company/composio/api-key/test",
        None,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], true, "{body}");
    assert!(body.get("probeClass").is_none(), "{body}");
    assert!(body.get("message").is_none(), "{body}");
    assert!(!raw.contains(BYOK_KEY), "the check leaked the key: {raw}");
}

// ── Storage addresses and the legacy fallback (#2306) ──────────────

/// A token `PUT` writes both addresses, and a legacy-only value still reads
/// as configured before that first write.
#[tokio::test]
async fn a_token_put_mirrors_to_the_legacy_slot() {
    use crate::company::composio::{LEGACY_TOKEN_KEY, TINYHUMANS_KEY_KEY};

    let home_dir = home();
    let state = state_with_manifest_id(home_dir.path(), "legacytoken", GRANTED).await;
    let runtime = runtime_of(&state, "legacytoken");
    runtime
        .secrets()
        .set(
            runtime.id(),
            LEGACY_TOKEN_KEY,
            crate::ports::types::SecretValue("th-not-a-real-key".into()),
        )
        .await
        .unwrap();

    let (_, dto, raw) = send_for(
        &state,
        "legacytoken",
        "GET",
        "/api/v1/company/composio",
        None,
    )
    .await;
    assert_eq!(dto["credentialSource"], "static", "{raw}");

    let (code, _, raw) = send_for(
        &state,
        "legacytoken",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "th-not-a-real-key-2" })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");

    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await.as_deref(),
        Some("th-not-a-real-key-2")
    );
    assert_eq!(
        read_slot(&runtime, LEGACY_TOKEN_KEY).await.as_deref(),
        Some("th-not-a-real-key-2")
    );
    assert!(
        !raw.contains("th-not-a-real-key"),
        "the PUT leaked the token: {raw}"
    );

    // Guarded while the company is on the managed route
    // (in-use-guards.md §2); this test is about the mirror write, not
    // the guard, so it confirms.
    let (code, _, raw) = send_for(
        &state,
        "legacytoken",
        "PUT",
        "/api/v1/company/composio/token",
        Some(json!({ "token": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{raw}");
    assert_eq!(
        read_slot(&runtime, TINYHUMANS_KEY_KEY).await.as_deref(),
        Some("")
    );
    assert_eq!(
        read_slot(&runtime, LEGACY_TOKEN_KEY).await.as_deref(),
        Some("")
    );
}

/// Raw slot contents for a company's runtime, blank-or-absent collapsed to
/// `None` only when truly absent (a stored `""` reads back as `Some("")`).
async fn read_slot(runtime: &super::CompanyRuntime, key: &'static str) -> Option<String> {
    runtime
        .secrets()
        .get(runtime.id(), key)
        .await
        .unwrap()
        .map(|crate::ports::types::SecretValue(v)| v)
}
