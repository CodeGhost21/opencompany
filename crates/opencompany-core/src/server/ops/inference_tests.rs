use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;

use crate::company::CompanyManifest;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

const TOKEN: &str = "sk-super-secret-inference-token-XYZ";

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-inference-")
        .tempdir()
        .expect("tempdir")
}

fn manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
}

/// A manifest whose **only** inference lives in the default harness's
/// `[harness.inference]` — no company-level `[inference]` section at all.
/// `openhuman`-gated like its only caller: under the default build the
/// harness-wiring path the fix targets is compiled out, and an unused
/// helper would trip `clippy -D warnings`.
#[cfg(feature = "openhuman")]
fn manifest_with_harness_inference() -> CompanyManifest {
    toml::from_str(
        r#"[company]
name = "Acme"
[policy]
mode = "full"

[[harness]]
id = "embedded"
kind = "built_in"
default = true

[harness.inference]
provider = "openai_compatible"
base_url = "https://byo.example/v1"
"#,
    )
    .unwrap()
}

/// Commits `manifest` as `id`'s record — what `manifest_inference` reads.
async fn save_record(home: &std::path::Path, id: &CompanyId, manifest: &CompanyManifest) {
    use crate::ports::CompanyStore;
    FsCompanyStore::new(home.to_path_buf())
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
}

async fn state_with_company(home: &std::path::Path) -> AppState {
    state_with_company_named(home, "acme").await
}

/// A company under a caller-chosen id.
///
/// Almost every test here can share `acme`, but the catalog cache is
/// process-global and keyed on the company, and storing a key now evicts
/// that company's authenticated entries (Codex review on #2045). A test that
/// rotates a credential therefore wipes the seeded fixtures of every sibling
/// running beside it under the same id — libtest runs these in parallel — so
/// it needs an id of its own rather than an ordering assumption that cannot
/// hold.
async fn state_with_company_named(home: &std::path::Path, name: &str) -> AppState {
    let id = CompanyId::new(name);
    save_record(home, &id, &manifest()).await;
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, name).await;
    state
}

/// [`state_with_company`] over a caller-supplied manifest.
///
/// The strict `validate()` only runs on a first boot with no persisted
/// record (`src/runtime/builder.rs`), and `save_record` writes one first —
/// so this can plant a manifest a fresh company would now be refused. That
/// is the point: an endpoint stored before the refusal existed is exactly
/// the case the redaction half of the rule is for.
async fn state_with_manifest(
    home: &std::path::Path,
    name: &str,
    manifest_toml: &str,
) -> AppState {
    let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
    let id = CompanyId::new(name);
    save_record(home, &id, &manifest).await;
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, name).await;
    state
}

/// [`state_with_company`] over a harness-only-inference manifest. The
/// routes read `manifest_inference` from the saved record, so the company
/// boots on the echo brain here (no pool attached) while the record it
/// reads still carries the harness's `[harness.inference]` — exactly the
/// shape of company the fix targets.
#[cfg(feature = "openhuman")]
async fn state_with_harness_inference(home: &std::path::Path) -> AppState {
    let id = CompanyId::new("acme");
    save_record(home, &id, &manifest_with_harness_inference()).await;
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest_with_harness_inference())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// A rebuilder that rebuilds over the handover, as the binary's does.
struct Working {
    home: std::path::PathBuf,
}

#[async_trait::async_trait]
impl crate::runtime::RuntimeRebuilder for Working {
    async fn rebuild(
        &self,
        _state: &AppState,
        request: crate::runtime::RebuildRequest,
    ) -> crate::Result<crate::company::runtime::CompanyRuntime> {
        RuntimeBuilder::new(self.home.clone(), request.manifest)
            .with_id(request.id)
            .with_handover(request.handover)
            .build()
            .await
    }
}

/// `POST …/inference/restart` rebuilds the runtime in place, so the console's
/// "Restart required" notice has an action behind it rather than being a
/// dead end pointing at a container the operator may not be able to touch.
#[tokio::test]
async fn restart_rebuilds_the_registered_runtime() {
    let home_dir = home();
    let home = home_dir.path();
    let id = CompanyId::new("acme");
    let state = state_with_company(home)
        .await
        .with_rebuilder(std::sync::Arc::new(Working {
            home: home.to_path_buf(),
        }));
    state.set_boot_inputs(id.clone(), crate::runtime::BootInputs::default());
    let before = state.registry().get(&id).expect("registered");

    let (status, resp, raw) =
        send(&state, "POST", "/api/v1/company/inference/restart", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    // A genuinely different runtime is registered — the point of the route.
    let after = state.registry().get(&id).expect("registered");
    assert!(
        !std::sync::Arc::ptr_eq(&before, &after),
        "the restart must actually swap the runtime, not report success and leave it"
    );
    // And it is taking work, rather than stuck in the quiesce window. A
    // company parked there refuses every cycle forever, which is worse than
    // the stale brain the rebuild was replacing.
    assert!(!after.is_quiesced());
    assert!(resp["status"].is_object(), "{raw}");
}

/// Calling it twice is not an error. The console offers the button off a
/// status read, so it can always be a moment stale — refusing when nothing
/// is pending would turn a harmless retry into a failure an operator has to
/// interpret.
#[tokio::test]
async fn restarting_a_healthy_company_is_a_no_op_not_a_refusal() {
    let home_dir = home();
    let home = home_dir.path();
    let id = CompanyId::new("acme");
    let state = state_with_company(home)
        .await
        .with_rebuilder(std::sync::Arc::new(Working {
            home: home.to_path_buf(),
        }));
    state.set_boot_inputs(id, crate::runtime::BootInputs::default());

    for attempt in 1..=2 {
        let (status, _, raw) =
            send(&state, "POST", "/api/v1/company/inference/restart", None).await;
        assert_eq!(status, StatusCode::OK, "attempt {attempt}: {raw}");
    }
}

/// A host that wired no rebuilder cannot do this, and must say so rather
/// than report a success that changed nothing. This is the pre-#290
/// deployment, and the console keeps showing the restart notice.
#[tokio::test]
async fn a_host_that_cannot_rebuild_says_so() {
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) =
        send(&state, "POST", "/api/v1/company/inference/restart", None).await;
    assert_ne!(status, StatusCode::OK, "{raw}");
    assert!(
        raw.contains("restart the process"),
        "the failure must tell the operator what will work instead: {raw}"
    );

    // Critically, the company is still serving. A failed rebuild that left
    // it quiesced would turn a cosmetic dead end into an outage.
    let (status, _, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK);
}

/// `restart_runtime` takes `AdminScopedCompany` in its signature, but
/// nothing here had actually driven a plain member against it over HTTP —
/// every other test in this module authenticates as the seeded admin.
/// Rebuilding a company's runtime on demand is at least as sharp a
/// boundary as any other admin-only write in this module.
#[tokio::test]
async fn a_member_may_not_restart_the_runtime() {
    let home_dir = home();
    let home = home_dir.path();
    let id = CompanyId::new("acme");
    let state = state_with_company(home)
        .await
        .with_rebuilder(std::sync::Arc::new(Working {
            home: home.to_path_buf(),
        }));
    state.set_boot_inputs(id.clone(), crate::runtime::BootInputs::default());
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let before = state.registry().get(&id).expect("registered");

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/company/inference/restart")
        .header("cookie", crate::server::test_support::member_cookie("acme"))
        .body(Body::empty())
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // A refused request must not have rebuilt the runtime either.
    let after = state.registry().get(&id).expect("still registered");
    assert!(
        std::sync::Arc::ptr_eq(&before, &after),
        "a forbidden restart must not swap the runtime"
    );
}

/// Issue #1736: the console cannot offer a restart it has no way to know is
/// available, so the status carries the capability rather than leaving the
/// card to guess from the deployment shape.
///
/// The pairing is the whole point — a flag that is always `false` would
/// satisfy the "no button on a host that cannot" half while silently
/// removing the action from every host that can.
#[tokio::test]
async fn the_status_says_whether_this_host_can_rebuild_in_place() {
    let bare_home = home();
    let bare = state_with_company(bare_home.path()).await;
    let (status, body, raw) = send(&bare, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["canRebuildInPlace"],
        json!(false),
        "a host with no rebuilder must say so, or the console renders a \
         Restart now button whose route can only answer with a config error: {raw}"
    );

    let wired_home = home();
    let wired =
        state_with_company(wired_home.path())
            .await
            .with_rebuilder(std::sync::Arc::new(Working {
                home: wired_home.path().to_path_buf(),
            }));
    let (status, body, raw) = send(&wired, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["canRebuildInPlace"],
        json!(true),
        "a host that wired one must keep offering the action: {raw}"
    );
}

/// Issue #1737: a probe the process already knows cannot authenticate is
/// refused here rather than sent.
///
/// The endpoint is the discard port, so a regression does not merely fail
/// this assertion — it makes an outbound connection, which is the behaviour
/// under test. A keyless `openrouter` carrying its own `base_url` resolves
/// direct and credential-less by construction, so this does not depend on
/// whatever the process environment happens to hold.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_probe_with_no_credential_is_refused_before_it_is_sent() {
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "baseUrl": "http://127.0.0.1:9/v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) =
        send(&state, "POST", "/api/v1/company/inference/test", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(body["code"], json!("no_key"), "{raw}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no Authorization header"),
        "the refusal names the cause the vendor's 401 would have hidden: {raw}"
    );
}

/// The other half of that judgement: an endpoint the operator supplied may
/// legitimately want no bearer, so it is still probed. Refusing there would
/// turn a working local server into a false alarm — worse than the outbound
/// request it saves.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_keyless_custom_endpoint_is_still_probed() {
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openai_compatible", "baseUrl": "http://127.0.0.1:9/v1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) =
        send(&state, "POST", "/api/v1/company/inference/test", None).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{raw}");
    assert_eq!(body["code"], json!("probe_failed"), "{raw}");
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn saved_company_probe_sends_its_resolved_model() {
    let sent_model = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let model_for_route = sent_model.clone();
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
            let sent_model = model_for_route.clone();
            async move {
                *sent_model.lock().unwrap() = body["model"].as_str().map(str::to_string);
                axum::Json(json!({
                    "choices": [{ "message": { "content": "pong" } }]
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": format!("http://{address}/v1"),
            "models": { "chat-v1": "provider/model" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) =
        send(&state, "POST", "/api/v1/company/inference/test", None).await;
    server.abort();

    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(body["ok"], true, "{raw}");
    assert_eq!(
        sent_model.lock().unwrap().as_deref(),
        Some("provider/model")
    );
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn saved_company_probe_keeps_typed_credential_failures() {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "message": "Missing Authentication header",
                        "code": 401
                    }
                })),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": format!("http://{address}/v1"),
            "key": "not-a-real-key",
            "models": { "chat-v1": "provider/model" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) =
        send(&state, "POST", "/api/v1/company/inference/test", None).await;
    server.abort();

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{raw}");
    assert_eq!(body["code"], json!("credential_rejected"), "{raw}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("did carry an Authorization header"),
        "{raw}"
    );
}

/// Issue #1737, the sentence that would have saved an hour: OpenRouter
/// answers a credential it cannot parse with `Missing Authentication
/// header`, which reads as "nothing was sent" and is how the issue came to
/// be filed against the wrong layer. The header *was* sent.
#[cfg(feature = "openhuman")]
#[test]
fn a_401_is_reported_as_a_rejected_credential_rather_than_a_missing_header() {
    let decl = inference::decl_for_probe(
        "openrouter",
        None,
        Some("a-key-for-some-other-vendor"),
        None,
    );
    let error = anyhow::Error::new(tinyinference::Error::Provider(Box::new(
        tinyinference::model::ProviderError {
            provider: "inference".to_string(),
            status: Some(401),
            message: "Missing Authentication header".to_string(),
            ..Default::default()
        },
    )));
    let (message, code) = probe_failure(&decl, &error);

    assert_eq!(code, "credential_rejected");
    assert!(
        message.contains("did carry an Authorization header"),
        "the console must not repeat the vendor's reading back at the operator: {message}"
    );
    assert!(
        message.contains("stored against the provider selected when it was saved"),
        "and it must name the reason a stored key can still be the wrong one: {message}"
    );
    assert!(
        message.contains("Missing Authentication header"),
        "the vendor's own words stay attached as evidence: {message}"
    );
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    send_as(state, "acme", method, uri, body).await
}

/// `send` against a company other than `acme`, for the tests that need an id
/// of their own — see `state_with_company_named`.
async fn send_as(
    state: &AppState,
    company: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", crate::server::test_support::fixed_cookie(company));
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

/// Seed one endpoint's catalog cache so the route answers offline, and
/// deterministically: each test uses a base URL of its own, because the
/// registry is process-wide and a shared key would let one test's positive
/// entry decide another's outcome.
///
/// Seeded in **`acme`'s** scope, because an authenticated read is
/// partitioned per company — every route test here drives the `acme`
/// company from [`state_with_company`], and a seed in the shared/keyless
/// slot would no longer be the entry the route reads.
/// Seed the authenticated catalog cache for a named company — the scope the
/// route reads under.
///
/// Every caller names its own company rather than sharing one: eviction is
/// company-wide, so a fixture seeded under an id another test saves a key
/// for is thrown away at random.
fn seed_catalog_for(company: &str, base_url: &str, ids: &[&str]) {
    crate::server::inference_models::catalog_cache_scoped(base_url, Some(company)).store(
        ids.iter()
            .map(|id| crate::server::inference_models::InferenceModel {
                id: (*id).to_string(),
                name: Some(format!("{id} (display)")),
                context_length: Some(128_000),
            })
            .collect(),
        std::time::Instant::now(),
    );
}

/// The catalog is read from the endpoint **this company** is configured
/// against, not from OpenRouter's public registry.
///
/// This is the defect in one assertion. The route used to call
/// `openrouter_models()` with no reference to the company at all, so a
/// company pointed at a TinyHumans base URL was shown OpenRouter's 421
/// models — `anthropic/claude-sonnet-5` among them — and the endpoint then
/// answered `Model 'anthropic/claude-sonnet-5' is not available`.
#[tokio::test]
async fn model_catalog_route_lists_the_configured_endpoints_own_catalog() {
    const ENDPOINT: &str = "http://127.0.0.1:9/tier-native/v1";
    // Its own company id. Saving a key evicts that company's authenticated
    // catalogs, and a dozen tests in this module save one under `acme`; with
    // a shared id, whichever of them libtest happens to run alongside this
    // one throws the seeded fixture away. Locally the interleaving hid it;
    // CI's found it (Codex review on #2045).
    const COMPANY: &str = "catalog-tiers";
    let home_dir = home();
    let state = state_with_company_named(home_dir.path(), COMPANY).await;
    let (status, _, raw) = send_as(
        &state,
        COMPANY,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": ENDPOINT,
            "key": "test-token",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    // Seeded *after* the save, not before: storing a key evicts this
    // company's authenticated catalogs, because a rotation changes what the
    // endpoint will answer without changing the cache key (Codex review on
    // #2045). Seeding first meant the save threw the fixture away and the
    // route fell through to a real request. This order is also what happens
    // in life — the cache is warmed by a read, which comes after the config
    // exists to be read against.
    seed_catalog_for(
        COMPANY,
        ENDPOINT,
        &["agentic-v1", "chat-v1", "reasoning-v1", "vision-v1"],
    );

    let (status, body, raw) = send_as(
        &state,
        COMPANY,
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["baseUrl"], ENDPOINT,
        "the catalog names the endpoint it came from: {raw}"
    );
    let ids: Vec<&str> = body["models"]
        .as_array()
        .expect("models array")
        .iter()
        .map(|m| m["id"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        ids,
        vec!["agentic-v1", "chat-v1", "reasoning-v1", "vision-v1"],
        "the configured endpoint's own ids, not OpenRouter's: {raw}"
    );
    // Keys rework (#2306), slice 2d: no vocabulary classification is
    // shipped on this DTO any more — the console never read it either.
    assert!(
        body.get("tierVocabulary").is_none() && body.get("tierDefaults").is_none(),
        "{raw}"
    );
}

/// Rotating the key does not let the route answer from the catalog the
/// *previous* credential fetched.
///
/// The cache key holds non-secret ids only, so a rotation is invisible to
/// it: without eviction the pre-rotation catalog would answer for the rest
/// of `MODEL_CATALOG_TTL` and the new bearer would never reach `/models`,
/// so the console could offer models the new account cannot access (Codex
/// review on #2045).
///
/// Asserted through the route rather than the registry — the registry-level
/// boundaries are covered by
/// `rotating_a_credential_evicts_only_that_companys_authenticated_catalogs`.
/// The endpoint is unreachable on purpose: after the eviction there is
/// nothing cached to serve, so the route reports a failure instead of
/// handing back the stale ids, and *that* is the observable difference.
#[tokio::test]
async fn rotating_the_key_does_not_serve_the_previous_credentials_catalog() {
    const ENDPOINT: &str = "http://127.0.0.1:9/rotated/v1";
    // Its own company: eviction is company-wide, so rotating under `acme`
    // would clear the fixtures of every sibling test running in parallel.
    const COMPANY: &str = "rotator";
    let home_dir = home();
    let state = state_with_company_named(home_dir.path(), COMPANY).await;

    let configure = |key: &'static str| {
        send_as(
            &state,
            COMPANY,
            "PUT",
            "/api/v1/company/inference",
            Some(json!({
                "provider": "openai_compatible",
                "baseUrl": ENDPOINT,
                "key": key,
            })),
        )
    };

    let (status, _, raw) = configure("first-token").await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    // What the first credential saw.
    seed_catalog_for(COMPANY, ENDPOINT, &["entitled/first-only"]);

    let (status, body, raw) = send_as(
        &state,
        COMPANY,
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        body["models"][0]["id"], "entitled/first-only",
        "the first credential's catalog is cached and served: {raw}"
    );

    // Rotate. The endpoint and the company are unchanged, so nothing in the
    // cache key moves — only the credential behind it.
    let (status, _, raw) = configure("second-token").await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) = send_as(
        &state,
        COMPANY,
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_ne!(
        body["models"][0]["id"], "entitled/first-only",
        "the rotated credential must not be answered from the old key's catalog: {raw}"
    );
    assert!(
        body["error"].is_string(),
        "with the entry evicted and the endpoint unreachable, the route reports why \
         rather than replaying stale ids: {raw}"
    );
}

/// Reset owes the same eviction a rotation does.
///
/// `revert_config` clears the runtime key so resolution falls back to the
/// manifest's credential. When the manifest points at the same base URL
/// nothing in the cache key moves, so without eviction the route would keep
/// answering from the catalog the *cleared* credential fetched (Codex review
/// on #2045). Its own company id, for the parallelism reason above.
#[tokio::test]
async fn resetting_the_config_does_not_serve_the_cleared_credentials_catalog() {
    const ENDPOINT: &str = "http://127.0.0.1:9/reset/v1";
    const COMPANY: &str = "resetter";
    let home_dir = home();
    let state = state_with_company_named(home_dir.path(), COMPANY).await;

    let (status, _, raw) = send_as(
        &state,
        COMPANY,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": ENDPOINT,
            "key": "before-reset",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    seed_catalog_for(COMPANY, ENDPOINT, &["entitled/before-reset"]);

    let (status, body, raw) = send_as(
        &state,
        COMPANY,
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(body["models"][0]["id"], "entitled/before-reset", "{raw}");

    let (status, _, raw) =
        send_as(&state, COMPANY, "DELETE", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) = send_as(
        &state,
        COMPANY,
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_ne!(
        body["models"][0]["id"], "entitled/before-reset",
        "a reset must not be answered from the cleared credential's catalog: {raw}"
    );
}

/// A provider that cannot be reached must say so. An empty list is not a
/// true statement about a catalog nobody managed to read, and the picker
/// blanking with no explanation is how an operator concludes their provider
/// serves no models.
#[tokio::test]
async fn model_catalog_route_reports_an_unreachable_provider_rather_than_blanking() {
    // The discard port: refuses fast, offline, and deterministically.
    const ENDPOINT: &str = "http://127.0.0.1:9/unreachable/v1";
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": ENDPOINT,
            "key": "test-token",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, body, raw) =
        send(&state, "GET", "/api/v1/company/inference/models", None).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the console needs a body it can render, not a bare 5xx: {raw}"
    );
    assert!(
        body["models"].as_array().is_some_and(|m| m.is_empty()),
        "{raw}"
    );
    // Keys rework (#2306), slice 2d: no vocabulary classification is
    // shipped on this DTO any more.
    assert!(
        body.get("tierVocabulary").is_none() && body.get("tierDefaults").is_none(),
        "{raw}"
    );
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("Could not list models from") && error.contains(ENDPOINT),
        "the failure names the endpoint it could not reach: {raw}"
    );
}

// ---------------------------------------------------------------------
// A credential embedded in an endpoint: refused on the way in, redacted on
// the way out. Three paths, because the leak had three.
// ---------------------------------------------------------------------

/// The credential a test endpoint carries. Obviously fake, and asserted on
/// by substring everywhere below — a leak anywhere is a leak.
const EMBEDDED_PASSWORD: &str = "hunter2";
/// Loopback discard: refuses immediately, offline and deterministically.
const CREDENTIALED_ENDPOINT: &str = "http://alice:hunter2@127.0.0.1:9/unreachable/v1";
const CREDENTIALED_MANIFEST: &str = "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
     [inference]\nprovider = \"openai_compatible\"\n\
     base_url = \"http://alice:hunter2@127.0.0.1:9/unreachable/v1\"\n";

/// Path 1 — it is never stored.
#[tokio::test]
async fn an_endpoint_carrying_a_credential_is_refused_before_anything_is_written() {
    let home_dir = home();
    let state = state_with_company_named(home_dir.path(), "credurl-add").await;

    for body in [
        json!({
            "kind": "custom",
            "label": "Acme gateway",
            "baseUrl": CREDENTIALED_ENDPOINT,
        }),
        // The local-runtime category types its own endpoint too.
        json!({ "kind": "ollama", "baseUrl": CREDENTIALED_ENDPOINT }),
    ] {
        let (status, _, raw) = send_as(
            &state,
            "credurl-add",
            "POST",
            "/api/v1/company/inference/providers",
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {raw}");
        assert!(
            !raw.contains(EMBEDDED_PASSWORD),
            "the refusal echoed the credential it was refusing: {raw}"
        );
    }

    // The draft probe refuses it too, rather than putting a basic-auth
    // credential on the wire to an address the operator named.
    let (status, _, raw) = send_as(
        &state,
        "credurl-add",
        "POST",
        "/api/v1/company/inference/probe",
        Some(json!({ "baseUrl": CREDENTIALED_ENDPOINT })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{raw}");
    assert!(!raw.contains(EMBEDDED_PASSWORD), "{raw}");

    // And nothing landed: no stored endpoint anywhere in the status read.
    let (_, _, raw) = send_as(
        &state,
        "credurl-add",
        "GET",
        "/api/v1/company/inference",
        None,
    )
    .await;
    assert!(
        !raw.contains(EMBEDDED_PASSWORD),
        "a refused endpoint reached the store: {raw}"
    );
}

/// Path 2 — the catalog-read failure note does not re-add it.
///
/// `reqwest` redacts userinfo in its own error `Display`; the handler's own
/// `format!` used to put it back from the endpoint we hold.
#[tokio::test]
async fn a_catalog_read_failure_names_the_endpoint_without_its_credential() {
    let home_dir = home();
    let state =
        state_with_manifest(home_dir.path(), "credurl-models", CREDENTIALED_MANIFEST).await;

    let (status, body, raw) = send_as(
        &state,
        "credurl-models",
        "GET",
        "/api/v1/company/inference/models",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        !raw.contains(EMBEDDED_PASSWORD),
        "the catalog route leaked an embedded credential: {raw}"
    );
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("Could not list models from"),
        "the failure still names the endpoint it could not reach: {raw}"
    );
    assert!(
        error.contains("http://***@127.0.0.1:9/unreachable/v1"),
        "the endpoint is redacted, not dropped — the operator still needs to \
         recognise which one it was: {raw}"
    );
}

/// Path 3 — the read DTO, on the non-admin route every console reader calls.
#[tokio::test]
async fn the_company_status_read_redacts_an_endpoint_credential() {
    let home_dir = home();
    let state =
        state_with_manifest(home_dir.path(), "credurl-status", CREDENTIALED_MANIFEST).await;

    let (status, body, raw) = send_as(
        &state,
        "credurl-status",
        "GET",
        "/api/v1/company/inference",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(
        !raw.contains(EMBEDDED_PASSWORD),
        "`GET …/inference` is `ScopedCompany`, so this is the password on the \
         wire for every console reader: {raw}"
    );
    assert_eq!(
        body["baseUrl"].as_str(),
        Some("http://***@127.0.0.1:9/unreachable/v1"),
        "{raw}"
    );
}

/// The same rule over a provider **row**, which is a different DTO built
/// from a different store.
#[tokio::test]
async fn a_provider_row_redacts_an_endpoint_credential() {
    use crate::company::inference::store;

    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), NO_INFERENCE).await;
    // Written straight to the store, as a record predating the refusal
    // would be — the handler now refuses this endpoint.
    store::put_provider(
        runtime.id(),
        runtime.secrets().as_ref(),
        store::ProviderDraft {
            slug: "acme-gateway".into(),
            label: "Acme gateway".into(),
            kind: "custom".into(),
            base_url: CREDENTIALED_ENDPOINT.into(),
            models: BTreeMap::new(),
            enabled: true,
        },
    )
    .await
    .unwrap();

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    let row = dto
        .providers
        .iter()
        .find(|p| p.slug == "acme-gateway")
        .expect("the planted provider is listed");
    assert_eq!(row.base_url, "http://***@127.0.0.1:9/unreachable/v1");
    assert!(
        !serde_json::to_string(&dto)
            .unwrap()
            .contains(EMBEDDED_PASSWORD),
        "the whole status DTO must be free of it, not just the field we looked at"
    );
}

/// Defect B, end to end: a name past the bound is a 400 before any write,
/// not a 500 with a truncated credential behind it.
#[tokio::test]
async fn a_provider_name_past_the_bound_is_refused_rather_than_breaking_the_store() {
    use crate::company::inference::store::MAX_PROVIDER_NAME_CHARS;

    let home_dir = home();
    let state = state_with_company_named(home_dir.path(), "longname").await;

    // At the bound: accepted, and its credential round-trips through the
    // store — including the clear the delete issues.
    let at_limit = "a".repeat(MAX_PROVIDER_NAME_CHARS);
    let (status, _, raw) = send_as(
        &state,
        "longname",
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": at_limit,
            "baseUrl": UNREACHABLE,
            "key": "sk-not-a-real-key",
            "model": "acme-model",
            "addAnyway": true,
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a name at the bound is legal: {raw}"
    );
    // The company's first (and only) provider auto-became its default
    // (X1), so removing it needs confirmation like any other in-use row.
    let (status, _, raw) = send_as(
        &state,
        "longname",
        "DELETE",
        &format!("/api/v1/company/inference/providers/{at_limit}?confirmInUse=true"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "removing it clears the credential, which is the write that used to \
         fail after truncating it: {raw}"
    );

    // Past the bound: refused, and refused *before* the key is written.
    let past_limit = "a".repeat(MAX_PROVIDER_NAME_CHARS + 1);
    let (status, _, raw) = send_as(
        &state,
        "longname",
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": past_limit,
            "baseUrl": UNREACHABLE,
            "key": "sk-not-a-real-key",
        })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a 500 here is the incident: {raw}"
    );
    assert!(
        raw.contains(&MAX_PROVIDER_NAME_CHARS.to_string()),
        "the refusal says what the limit is: {raw}"
    );
}

// ---------------------------------------------------------------------
// Issue #597 — the card must report the endpoint requests actually reach,
// not the built-in production constant.
// ---------------------------------------------------------------------

/// Where a staging deployment is pointed with `OPENCOMPANY_INFERENCE_URL`.
const STAGING_URL: &str = "https://staging-api.tinyhumans.ai/openai/v1";

/// The platform default a staging tenant is injected with.
fn staging_platform() -> EnvDefault {
    EnvDefault {
        base_url: STAGING_URL.to_string(),
        credential: crate::company::credentials::Credential::from_value(
            "platform-token".to_string(),
        ),
    }
}

/// A built runtime whose committed manifest is `manifest_toml`.
async fn runtime_with(home: &std::path::Path, manifest_toml: &str) -> CompanyRuntime {
    let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
    let id = CompanyId::new("acme");
    save_record(home, &id, &manifest).await;
    RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id)
        .build()
        .await
        .unwrap()
}

const NO_INFERENCE: &str = "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n";

const MANAGED_MANIFEST: &str = "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
     [inference]\nprovider = \"managed\"\n";

#[tokio::test]
async fn unconfigured_company_reports_the_platform_url_not_the_built_in_default() {
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), NO_INFERENCE).await;

    // No platform endpoint on this deployment — the built-in constant is
    // still the only honest answer, and this arm must not regress.
    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert_eq!(dto.base_url, inference::PLATFORM_BASE_URL);

    // Pointed at staging, the card follows — and *only* the URL moves.
    let dto = effective_status_with(&runtime, Some(&staging_platform()), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, STAGING_URL);
    assert_eq!(dto.provider, "managed");
    assert_eq!(
        dto.source, "managed",
        "a platform endpoint is not tenant config"
    );
    assert!(
        !dto.key_configured,
        "the platform token is not a stored tenant key"
    );
    assert!(
        !dto.restart_required,
        "a platform endpoint strands no tenant config behind a restart"
    );
    assert!(
        !dto.harness_reachable,
        "a runtime built without a harness pool cannot reach the design path"
    );
}

/// A company with no profile drafter says so, on the one route the console
/// reads before it decides which Add-teammate dialog to render.
///
/// The console used to answer this question itself, from `cognition`, with
/// `!== "echo"`. `profile_drafter()` is built from `workflow_harness_deps`,
/// which `RuntimeBuilder` assigns in exactly one place — inside the
/// embedded-harness arm — so a `hosted`, `sidecar` or `custom` company has
/// no drafter and the guess was wrong for three of the six paths. Every
/// create through the reduced dialog on one of them cost the operator a
/// sentence, a Create, a wait on a design pass that could only answer
/// `no_model`, and then the full form anyway.
///
/// Pinned against `harness_reachable` deliberately: they are different
/// questions and the DTO carries both, so a future edit that collapses
/// them fails here.
#[tokio::test]
async fn the_status_reports_whether_a_design_pass_can_run() {
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), MANAGED_MANIFEST).await;

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert!(
        !dto.designs_profiles,
        "a runtime with no harness deps has no profile drafter, so the \
         reduced dialog must not be offered"
    );
    assert_eq!(
        dto.designs_profiles,
        designs_profiles(&runtime),
        "the DTO must report the same fact `build_design` acts on"
    );
}

#[tokio::test]
async fn a_managed_manifest_also_inherits_the_platform_url() {
    // The half of #597 the report did not cover: `resolve_endpoint` falls
    // back to the built-in constant for *any* `managed` config that names no
    // base URL of its own, so a tenant with `[inference] provider =
    // "managed"` printed the production URL too — under a `manifest` badge
    // rather than the `managed` one the report reproduced.
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), MANAGED_MANIFEST).await;

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert_eq!(dto.base_url, inference::PLATFORM_BASE_URL);

    let dto = effective_status_with(&runtime, Some(&staging_platform()), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, STAGING_URL);
    assert_eq!(dto.source, "manifest");
    assert!(
        !dto.key_configured,
        "the platform token must not read as a tenant key on a managed manifest"
    );
}

/// Three inputs converge on `keyConfigured`, and exactly one of them must
/// never feed it. Today the tenant sources are a manifest `api_key_secret`
/// and a console `PUT`; #634 makes the console path the ordinary way an
/// admin sets the key on `managed`, which is precisely the provider that
/// inherits the platform credential. So the field has to keep answering
/// "did the *tenant* store a credential" and never "is there a credential",
/// on the one company where both are true at once.
///
/// Pinned here rather than left to review: without it the distinction this
/// PR's two-resolve split exists to preserve is enforced by nothing.
#[tokio::test]
async fn a_platform_token_never_reads_as_a_console_set_key() {
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), MANAGED_MANIFEST).await;
    let platform = staging_platform();

    // The platform credential is doing the outbound work, and the card still
    // says no key is configured — because none of it is the tenant's.
    let dto = effective_status_with(&runtime, Some(&platform), false)
        .await
        .unwrap();
    assert!(
        !dto.key_configured,
        "the platform token is not a stored tenant key"
    );
    assert_eq!(dto.base_url, STAGING_URL);

    // An admin sets one from the console — the write #634's screen performs.
    inference::store_key(runtime.id(), runtime.secrets().as_ref(), "sk-console-set")
        .await
        .unwrap();

    let dto = effective_status_with(&runtime, Some(&platform), false)
        .await
        .unwrap();
    assert!(
        dto.key_configured,
        "a console-set key must read as configured"
    );
    // Same company, same injected platform default, opposite answer — and the
    // key is not merely recorded: it moves the company off the subscription
    // proxy and onto its own OpenRouter account, which is the only way a
    // stored `sk-or-…` could actually be used.
    assert_eq!(dto.base_url, inference::OPENROUTER_BASE_URL);
}

#[tokio::test]
async fn an_explicit_tenant_base_url_outranks_the_platform_default() {
    let home_dir = home();
    let runtime = runtime_with(
        home_dir.path(),
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [inference]\nprovider = \"managed\"\nbase_url = \"https://byo.example/v1\"\n",
    )
    .await;

    let dto = effective_status_with(&runtime, Some(&staging_platform()), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, "https://byo.example/v1");
}

/// A third-party endpoint we hold no credential for uses its own URL
/// verbatim — the platform default is not a fallback for somewhere we cannot
/// authenticate anyway.
#[tokio::test]
async fn a_third_party_provider_ignores_the_platform_default() {
    let home_dir = home();
    let runtime = runtime_with(
        home_dir.path(),
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [inference]\nprovider = \"ollama\"\nbase_url = \"http://localhost:11434/v1\"\n",
    )
    .await;

    let dto = effective_status_with(&runtime, Some(&staging_platform()), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, "http://localhost:11434/v1");
    assert_eq!(dto.source, "manifest");
}

/// `openrouter` is dual-mode, and which mode it is in depends only on
/// whether the tenant holds a key. With none it rides the subscription on
/// the platform endpoint; that is the config a company starts on, and it
/// must work with nothing configured.
#[tokio::test]
async fn keyless_openrouter_rides_the_subscription_and_a_key_goes_direct() {
    let home_dir = home();
    let runtime = runtime_with(
        home_dir.path(),
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [inference]\nprovider = \"openrouter\"\n",
    )
    .await;
    let platform = staging_platform();

    let dto = effective_status_with(&runtime, Some(&platform), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, STAGING_URL, "proxied");
    assert_eq!(dto.slug, "subscription");
    assert!(!dto.key_configured);

    inference::store_key(runtime.id(), runtime.secrets().as_ref(), "sk-or-tenant")
        .await
        .unwrap();

    let dto = effective_status_with(&runtime, Some(&platform), false)
        .await
        .unwrap();
    assert_eq!(dto.base_url, inference::OPENROUTER_BASE_URL, "direct");
    assert_eq!(dto.slug, "openrouter");
    assert!(dto.key_configured);
}

/// The probe's gate stays keyed on *tenant* config. Pointing a deployment at
/// a platform endpoint gives the probe somewhere real to aim, but it must not
/// turn "nothing configured" into a live probe of the platform brain — that
/// 409 is the honest answer to "test my provider" from a company that has
/// not named one.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn probing_an_unconfigured_company_stays_not_configured() {
    let home_dir = home();
    let state = state_with_company(home_dir.path()).await;

    let (status, body, _) = send(&state, "POST", "/api/v1/company/inference/test", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "not_configured");
}

/// A company whose only inference lives in `[harness.inference]` resolves
/// the harness's provider for both status and probe — the same
/// default-harness fallback [`RuntimeBuilder::build`] applies at boot.
///
/// Before the fix `manifest_inference` read only the company-level
/// `[inference]`, so such a company reported `managed`, rejected
/// `/inference/test` as `not_configured`, and mislabeled its status while
/// turns ran on the harness configuration the same record holds.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn harness_only_inference_reports_the_harness_provider_for_status_and_probe() {
    let home_dir = home();
    let state = state_with_harness_inference(home_dir.path()).await;

    // Status resolves the default harness's `[harness.inference]`, not the
    // absent company-level section: the operator sees the provider their
    // turns actually run on.
    let (status, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dto["provider"], "openai_compatible");
    assert_eq!(dto["source"], "manifest");
    assert_ne!(dto["provider"], "managed");

    // The probe resolves the same inference, so it is *not* rejected as
    // `not_configured`; it reaches the (unreachable) host and reports that
    // failure instead.
    let (status, body, _) = send(&state, "POST", "/api/v1/company/inference/test", None).await;
    assert_ne!(status, StatusCode::CONFLICT);
    assert_ne!(body["code"], "not_configured");
}

#[cfg(feature = "openhuman")]
#[test]
fn platform_default_follows_the_injected_inference_url() {
    use crate::app::config::MapEnv;

    let env = MapEnv::new([
        ("TINYHUMANS_API_KEY", "platform-key"),
        ("OPENCOMPANY_INFERENCE_URL", STAGING_URL),
    ]);
    assert_eq!(
        platform_default(&env).map(|d| d.base_url),
        Some(STAGING_URL.to_string())
    );

    // A URL with no credential resolves to nothing — the same answer the
    // harness gives, so the card never advertises an endpoint that would
    // route nowhere.
    let bare = MapEnv::new([("OPENCOMPANY_INFERENCE_URL", STAGING_URL)]);
    assert!(platform_default(&bare).is_none());
}

#[tokio::test]
async fn status_defaults_to_managed_then_switches_to_runtime() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // A company with no manifest/runtime inference reports the managed default.
    let (status, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dto["provider"], "managed");
    assert_eq!(dto["source"], "managed");
    assert_eq!(dto["keyConfigured"], false);
    assert!(dto.get("key").is_none(), "status DTO must not carry a key");
    // Keys rework (#2306), slice 2d: no shipped tier defaults are sent
    // any more — every kind asks for a model explicitly (2c), so there is
    // nothing left to prefill from a guessed vocabulary.
    assert!(dto.get("defaultTierModels").is_none(), "{dto}");

    // Switch to OpenRouter with a write-only key + a tier→model map.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openrouter",
            "models": { "chat-v1": "deepseek/deepseek-chat", "reasoning-v1": "deepseek/deepseek-r1" },
            "key": TOKEN,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["provider"], "openrouter");
    assert_eq!(resp["status"]["slug"], "openrouter");
    assert_eq!(resp["status"]["source"], "runtime");
    assert_eq!(resp["status"]["keyConfigured"], true);
    assert_eq!(
        resp["status"]["models"]["chat-v1"],
        "deepseek/deepseek-chat"
    );
    // The token must NEVER appear in the mutation response body.
    assert!(!raw.contains(TOKEN), "PUT response leaked the token: {raw}");

    // GET reflects the switch and still never carries the token.
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["provider"], "openrouter");
    assert_eq!(dto["source"], "runtime");
    assert_eq!(dto["keyConfigured"], true);
    assert!(!raw.contains(TOKEN), "GET status leaked the token: {raw}");
    assert!(dto.get("defaultTierModels").is_none(), "{dto}");
}

/// Keys rework (#2306) slice 2a, decision Q3: exactly one TinyHumans row
/// is ever shown. A `tinyhumans` row in the index hides the legacy row.
#[tokio::test]
async fn a_listed_tinyhumans_row_hides_the_legacy_managed_row() {
    use crate::company::inference::store;

    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), NO_INFERENCE).await;
    let secrets = runtime.secrets().as_ref();
    store::put_provider(
        runtime.id(),
        secrets,
        store::ProviderDraft {
            slug: inference::MANAGED_SLUG.to_string(),
            label: "TinyHumans".to_string(),
            kind: inference::MANAGED_SLUG.to_string(),
            base_url: "https://api.tinyhumans.ai/agent-integrations/openrouter".to_string(),
            models: crate::company::INFERENCE_TIERS
                .iter()
                .map(|t| ((*t).to_string(), "acme/test-model".to_string()))
                .collect(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    secrets
        .set(
            runtime.id(),
            &store::provider_key_key(inference::MANAGED_SLUG),
            crate::ports::types::SecretValue("th-not-a-real-key".to_string()),
        )
        .await
        .unwrap();

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert_eq!(
        dto.providers
            .iter()
            .filter(|p| p.slug == "tinyhumans")
            .count(),
        1,
        "exactly one tinyhumans row: {:?}",
        dto.providers
    );
    assert!(
        dto.managed.configured,
        "the legacy chain still resolves through the same key slot"
    );
    assert!(
        !dto.managed.legacy_row,
        "a listed tinyhumans row must hide the legacy Managed row"
    );
    assert!(
        !dto.managed.needs_model,
        "needs_model is moot once the legacy row is hidden"
    );
}

/// A company whose only TinyHumans credential is the account key
/// (`tinyhumans/key`, no row) still shows the legacy Managed row — and it
/// never reads as fully configured (D-key-without-row / X5).
#[tokio::test]
async fn an_account_key_only_company_shows_the_legacy_managed_row() {
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), NO_INFERENCE).await;
    crate::company::company_key::store_key(
        runtime.id(),
        runtime.secrets().as_ref(),
        "th-not-a-real-key",
    )
    .await
    .unwrap();

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert!(
        !dto.providers.iter().any(|p| p.slug == "tinyhumans"),
        "no tinyhumans row exists yet: {:?}",
        dto.providers
    );
    assert_eq!(dto.managed.source, "company_account");
    assert!(dto.managed.configured);
    assert!(
        dto.managed.legacy_row,
        "with no tinyhumans row, the legacy row is the only TinyHumans row"
    );
    assert!(
        dto.managed.needs_model,
        "a key with no row must never read as fully configured (X5)"
    );
}

/// A company with nothing configured shows no legacy row at all.
#[tokio::test]
async fn a_company_with_nothing_shows_no_legacy_row() {
    let home_dir = home();
    let runtime = runtime_with(home_dir.path(), NO_INFERENCE).await;

    let dto = effective_status_with(&runtime, None, false).await.unwrap();
    assert!(!dto.managed.configured);
    assert!(!dto.managed.legacy_row);
    assert!(!dto.managed.needs_model);
}

#[tokio::test]
async fn revert_clears_the_runtime_override() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "key": TOKEN })),
    )
    .await;

    let (status, resp, _) = send(&state, "DELETE", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["status"]["provider"], "managed");
    assert_eq!(resp["status"]["source"], "managed");
    assert_eq!(
        resp["status"]["keyConfigured"], false,
        "the reset must clear a stored credential too, or keyConfigured lies"
    );
}

/// The reset is a *full* reset (issue #993): reverting also clears a stored
/// key. This is what keeps a keyless reconfiguration keyless — without it, a
/// stale secret would make the company resolve direct even though the console
/// shows no key, and `DELETE` would strand it with a credential it can never
/// see or clear.
#[tokio::test]
async fn revert_clears_the_key_so_a_keyless_save_rides_the_subscription() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Store a key first, so the reset has something stale to clear.
    let (status, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["status"]["slug"], "openrouter");
    assert_eq!(resp["status"]["keyConfigured"], true);

    let (status, resp, _) = send(&state, "DELETE", "/api/v1/company/inference", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["status"]["keyConfigured"], false);

    // A keyless save afterwards must land on the subscription, not be flung
    // direct by a credential the reset was supposed to remove.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["slug"], "subscription");
    assert_eq!(resp["status"]["keyConfigured"], false);

    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["slug"], "subscription");
    assert_eq!(dto["keyConfigured"], false);
    assert!(!raw.contains(TOKEN), "GET leaked the reset token: {raw}");
}

/// Issue #585: the company's own key — set, rotate, and clear — is the whole
/// point of the screen, and the route must never echo any of the three
/// tokens back.
///
/// Written against the legacy `managed` provider name on purpose: it is what
/// a console built before the rename still sends, and it must keep working.
#[tokio::test]
async fn a_legacy_managed_key_can_be_set_rotated_and_cleared() {
    const ROTATED: &str = "sk-rotated-inference-token-ABC";
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Set: the company pays for its own agents on the platform endpoint.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "managed", "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    // The choice is echoed back as it was made. This used to read
    // `"openrouter"` — the alias resolved — which is what made the managed
    // route unselectable from the console: the card seeds its provider
    // select straight from this field, so saving `managed` and reading back
    // `openrouter` snapped the select (and the managed-only Connect button)
    // back to OpenRouter every time. Where it resolves to is still reported,
    // on the two fields that answer that question.
    assert_eq!(resp["status"]["provider"], "managed");
    assert_eq!(resp["status"]["source"], "runtime");
    assert_eq!(resp["status"]["keyConfigured"], true);
    // Attribution follows the endpoint, not the label — and the endpoint
    // is the platform's (next assertion), so the company is still proxied
    // through it, on its own key rather than the subscription.
    assert_eq!(
        resp["status"]["proxied"], true,
        "a managed key still rides the platform endpoint"
    );
    // The **endpoint stays the platform's**, and this assertion is the
    // fix. `managed` used to normalize onto `openrouter` before the managed
    // branch was consulted, so a company that declared `managed` and stored
    // a key had its requests sent to `openrouter.ai` — carrying, in the
    // credential-link flow that writes exactly this blob, a TinyHumans
    // token. Declaring `managed` means the company pays for its own agents
    // on the TinyHumans brain, which is what this route's own header has
    // said since #585 and what the code now does.
    assert_eq!(
        resp["status"]["baseUrl"],
        crate::company::inference::PLATFORM_BASE_URL
    );
    assert_eq!(
        resp["status"]["slug"], "subscription",
        "the telemetry slug separates the platform endpoint from a direct OpenRouter account"
    );
    assert!(!raw.contains(TOKEN), "PUT leaked the token: {raw}");

    // Rotate: a second key replaces the first, still write-only.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "managed", "key": ROTATED })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["keyConfigured"], true);
    assert!(!raw.contains(TOKEN), "PUT leaked the old token: {raw}");
    assert!(!raw.contains(ROTATED), "PUT leaked the new token: {raw}");

    // Clear: an explicit empty key removes it (the console's "Remove key").
    let (status, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "managed", "key": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["status"]["keyConfigured"], false);

    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["keyConfigured"], false);
    // The selection outlives the key: clearing the credential is not a way
    // of un-choosing the route, and a plain read has to report the same
    // thing the write did or the console will drift from it on reload.
    assert_eq!(dto["provider"], "managed");
    assert_eq!(
        dto["proxied"], true,
        "with the key gone it is back on the subscription"
    );
    assert_eq!(dto["slug"], "subscription");
    for token in [TOKEN, ROTATED] {
        assert!(!raw.contains(token), "GET leaked a token: {raw}");
    }
}

/// Saving the managed brain has to be *visible*, not merely stored.
///
/// The write always landed — `PUT` persisted `managed` verbatim — but every
/// read reported the resolved kind, and `normalize_provider` folds the
/// managed alias onto `openrouter`. The console seeds its provider select
/// from this field verbatim (which is what keeps the select and the header
/// beside it from naming different providers), so the operator pressed Save,
/// got "Inference updated", and watched the card go straight back to
/// OpenRouter — taking the Connect-TinyHumans button, which only the managed
/// route renders, with it.
///
/// A save that cannot be observed is indistinguishable from one that did not
/// happen, so this asserts the round trip on both the mutation response and
/// a fresh read, from a company already saved on another provider.
#[tokio::test]
async fn switching_to_managed_reads_back_as_managed_rather_than_its_alias() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Start somewhere else, so "unchanged" and "reverted to OpenRouter"
    // cannot pass for the same answer.
    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "managed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        resp["status"]["provider"], "managed",
        "the save answers with the choice that was made"
    );

    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(
        dto["provider"], "managed",
        "and it survives a reload: {raw}"
    );
    // Resolution is untouched — this is a read-back fix, not a routing one.
    assert_eq!(dto["slug"], "subscription");
    assert_eq!(dto["proxied"], true);
    assert_eq!(dto["source"], "runtime");
}

#[tokio::test]
async fn invalid_provider_config_is_rejected() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Ollama requires a base_url.
    let (status, err, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "ollama" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"]
            .as_str()
            .unwrap_or_default()
            .contains("base_url"),
        "{err}"
    );
}

#[tokio::test]
async fn key_never_leaks_across_any_response() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, _, put_raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openai_compatible", "baseUrl": "https://byo.example/v1", "key": TOKEN })),
    )
    .await;
    let (_, get_dto, get_raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    // The live probe path returns an error (unreachable host) — assert the
    // scrubbed error body still never contains the token.
    let (_, _, test_raw) = send(&state, "POST", "/api/v1/company/inference/test", None).await;

    for raw in [put_raw, get_raw, test_raw] {
        assert!(!raw.contains(TOKEN), "a response leaked the token: {raw}");
    }

    // The list route, extended here **before** there was anything to leak.
    // The provider list is the newest way a credential could reach a wire,
    // and the point of adding it to this test on the same change that adds
    // the field is that the assertion exists before the mistake can.
    let providers = get_dto["providers"]
        .as_array()
        .expect("the status carries a provider list");
    assert_eq!(
        providers.len(),
        1,
        "one provider: the flat slot, as entry zero"
    );
    let entry_zero = &providers[0];
    assert_eq!(entry_zero["slug"], "openai_compatible");
    assert_eq!(
        entry_zero["keyConfigured"], true,
        "the boolean is the only thing a read may say about a key"
    );
    // Not "no field called `key`" — no field with the VALUE, whatever it is
    // called. A convenience rename would pass the narrower assertion.
    for (name, value) in entry_zero.as_object().expect("a provider object") {
        assert!(
            !value.to_string().contains(TOKEN),
            "provider field `{name}` leaked the token"
        );
    }

    // Every WRITE route, extended on the change that adds them rather than
    // afterwards. A credential reaches this subsystem through four bodies
    // now, and each one is a separate chance to echo it back.
    const SECOND: &str = "sk-not-a-real-key-for-the-second-provider";
    let (_, _, add_raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": "Acme gateway",
            "baseUrl": UNREACHABLE,
            "key": SECOND,
        })),
    )
    .await;
    let (_, _, edit_raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/acme-gateway",
        Some(json!({ "key": SECOND })),
    )
    .await;
    let (_, _, probe_raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/probe",
        Some(json!({ "baseUrl": UNREACHABLE, "key": SECOND })),
    )
    .await;
    let (_, list_dto, list_raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let (_, _, delete_raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme-gateway",
        None,
    )
    .await;

    for raw in [add_raw, edit_raw, probe_raw, list_raw, delete_raw] {
        for token in [TOKEN, SECOND] {
            assert!(!raw.contains(token), "a write route leaked a token: {raw}");
        }
    }
    // And the value assertion again, over a list that now holds two
    // credentials rather than one.
    for provider in list_dto["providers"].as_array().expect("a provider list") {
        for (name, value) in provider.as_object().expect("a provider object") {
            for token in [TOKEN, SECOND] {
                assert!(
                    !value.to_string().contains(token),
                    "provider field `{name}` leaked a token"
                );
            }
        }
    }
}

/// The discard port on loopback: a connection refused immediately, with no
/// DNS lookup and no wait. Loopback is a permitted probe target here because
/// the local-runtime category exists, which is exactly what makes it usable
/// as a test endpoint.
const UNREACHABLE: &str = "http://127.0.0.1:9/v1";

// --- the connect flow ------------------------------------------------------

#[tokio::test]
async fn a_cloud_provider_takes_its_endpoint_from_the_catalogue() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        // A base URL is sent and must be ignored: the paths in that table
        // are too varied for an override to be anything but a mistake.
        Some(json!({ "kind": "groq", "baseUrl": "https://wrong.example/v1", "model": "acme/test-model" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let providers = resp["status"]["providers"].as_array().unwrap();
    let groq = providers.iter().find(|p| p["slug"] == "groq").unwrap();
    assert_eq!(groq["baseUrl"], "https://api.groq.com/openai/v1");
    assert_eq!(groq["label"], "Groq");
    assert_eq!(groq["enabled"], true, "a new provider arrives on");
}

#[tokio::test]
async fn a_rename_that_posts_back_the_served_endpoint_keeps_the_stored_one() {
    // Codex review on #2281. The row is served through `redact_endpoint`,
    // which masks a path segment that only looks like userinfo. A console
    // that posts that value back on a rename must not overwrite a working
    // endpoint with the mask.
    const GATEWAY: &str = "http://127.0.0.1:9/proxy/http:user@example.com/v1";
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": "Acme gateway",
            "baseUrl": GATEWAY,
            "key": "sk-not-a-real-key",
            "model": "acme-model",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a path is not a credential: {raw}");

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let served = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme-gateway")
        .expect("the row was created")["baseUrl"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(served, GATEWAY, "the row is served redacted: {served}");

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/acme-gateway",
        Some(json!({ "label": "Acme renamed", "baseUrl": served })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    use crate::company::inference::store;
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("registered");
    let stored = store::list_providers(runtime.id(), runtime.secrets().as_ref())
        .await
        .unwrap();
    let acme = stored
        .iter()
        .find(|p| p.slug == "acme-gateway")
        .expect("still listed");
    assert_eq!(
        acme.base_url, GATEWAY,
        "the stored endpoint survived the rename"
    );
    assert_eq!(acme.label, "Acme renamed");
}

#[tokio::test]
async fn a_second_provider_holds_its_own_credential() {
    // The first moment two keys exist at once, which is the whole point of
    // the list: today one slot per company means switching provider strands
    // a credential for the wrong vendor in the only slot there is.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    for (label, key) in [
        ("First", "sk-not-a-real-key-1"),
        ("Second", "sk-not-a-real-key-2"),
    ] {
        let (status, _, raw) = send(
            &state,
            "POST",
            "/api/v1/company/inference/providers",
            Some(
                json!({ "kind": "custom", "label": label, "baseUrl": UNREACHABLE, "key": key, "model": "acme-model" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{raw}");
    }

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let providers = dto["providers"].as_array().unwrap();
    assert_eq!(providers.len(), 2);
    assert!(
        providers.iter().all(|p| p["keyConfigured"] == true),
        "each provider holds its own credential: {providers:?}"
    );
}

#[tokio::test]
async fn an_unreachable_endpoint_keeps_the_key_and_creates_the_row() {
    // The non-destructive path, which is the one the naive implementation
    // gets wrong: a proxy, a WAF, a rate limit or a mistyped model id all
    // fail a probe while the key is perfectly good.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": "Acme gateway",
            "baseUrl": UNREACHABLE,
            "key": "sk-not-a-real-key",
            "model": "acme-model",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the save succeeded: {raw}");
    assert_eq!(resp["probe"]["ok"], false);
    assert_eq!(resp["probe"]["class"], "endpoint");
    let acme = resp["status"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme-gateway")
        .expect("the row was created");
    assert_eq!(acme["keyConfigured"], true, "the key was kept");
    assert_eq!(acme["health"]["state"], "endpoint");
}

#[tokio::test]
async fn a_slug_that_shadows_a_builtin_is_refused_before_anything_is_written() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, err, _) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Groq", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    // And nothing landed: not the record, and not the credential.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(
        dto["providers"].as_array().unwrap().is_empty(),
        "a refused add writes nothing: {dto}"
    );
}

#[tokio::test]
async fn a_custom_provider_with_no_name_has_no_slug_to_write_under() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _, _) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "   ", "baseUrl": UNREACHABLE })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deleting_a_provider_clears_its_credential_and_scrubs_its_routes() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "acme-model" })),
    )
    .await;
    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": { "reasoning-v1": "acme:gpt-5" } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    // The company's only provider auto-became its default (X1), so the
    // delete needs confirmation like any other in-use row.
    let (status, resp, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme?confirmInUse=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(
        resp["affectedTiers"],
        json!(["reasoning-v1"]),
        "the operator is told which rows moved"
    );

    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert!(
        routes["routes"].as_object().unwrap().is_empty(),
        "the orphaned route was reset: {routes}"
    );

    // Re-adding the slug must not inherit the old credential. The store has
    // no delete, so this only holds because the clear was actually issued.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-model" })),
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    assert_eq!(
        acme["keyConfigured"], false,
        "a re-added slug must not silently reuse the removed key"
    );
}

/// Bug KR-L1-01 (live E2E, orchestrator-reported): a provider whose
/// `/models` catalog is real, valid JSON but too large for the probe's
/// success-body cap used to be silently read as zero models, reporting a
/// healthy `ok` add — the operator's key was never at fault, and nothing
/// said so. Now the add still saves the row (this failure is
/// non-destructive: `ProbeClass::Unknown` never rolls a credential back),
/// but the probe result is an explicit failure naming the model list
/// itself, never a bare `ok: true` over zero models.
#[tokio::test]
async fn an_add_whose_catalog_is_too_large_to_read_never_reports_ok() {
    use axum::routing::get;

    // One entry whose filler alone exceeds the probe's cap — a cheap way
    // to produce a real over-the-wire body larger than
    // `probe::CATALOG_BODY_CAP` without generating (and comparing) many
    // megabytes of meaningful content.
    let oversized = "x".repeat(17 * 1024 * 1024);
    let body = format!(r#"{{"data":[{{"id":"acme/test-model","description":"{oversized}"#);
    let app = axum::Router::new().route(
        "/v1/models",
        get(move || {
            let body = body.clone();
            async move {
                (
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    body,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({
            "kind": "custom",
            "label": "Acme",
            "baseUrl": format!("http://{address}/v1"),
            "key": "sk-not-a-real-key",
            "model": "acme/test-model",
        })),
    )
    .await;
    server.abort();
    assert_eq!(
        status,
        StatusCode::OK,
        "a non-destructive probe failure still saves: {raw}"
    );
    assert_eq!(resp["probe"]["ok"], false, "{resp}");
    assert_eq!(resp["probe"]["modelCount"], 0, "{resp}");
    let message = resp["probe"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("could not be read"),
        "must say the model list itself could not be read, not a generic failure: {message}"
    );

    // And the row's own recorded health must not be "ok" either — the
    // whole point being that the console's health column must not read
    // as a working, connected provider.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    assert_ne!(acme["health"]["state"], "ok", "{acme}");
}

#[tokio::test]
async fn disabling_keeps_the_route_and_names_the_tiers_it_parks() {
    // The departure from the plan, pinned so it is a decision rather than an
    // omission: disabling does NOT scrub. A disabled provider keeps its
    // endpoint, its label and its credential so that "stop billing this
    // account this week" is expressible, and scrubbing would make
    // re-enabling a re-configuration.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "acme-model" })),
    )
    .await;
    send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": { "reasoning-v1": "acme:gpt-5" } })),
    )
    .await;

    // `acme` auto-became the company default (X1) on that first add, so
    // disabling it needs confirmation like any other in-use row.
    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/enabled",
        Some(json!({ "enabled": false, "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    // The explicitly routed tier **and** the three unset ones. `acme` is
    // this company's only provider, so it is also what every unrouted
    // workload was going through — switching it off moves those to managed,
    // and a response naming only the explicit route would have said nothing
    // about a change of who pays for the other three.
    let mut named: Vec<String> = resp["affectedTiers"]
        .as_array()
        .expect("affectedTiers is a list")
        .iter()
        .map(|t| t.as_str().unwrap_or_default().to_string())
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            "agentic-v1".to_string(),
            "chat-v1".to_string(),
            "reasoning-v1".to_string(),
            "vision-v1".to_string(),
        ],
        "{raw}"
    );

    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert_eq!(
        routes["routes"]["reasoning-v1"], "acme:gpt-5",
        "the route survives so switching back on restores it: {routes}"
    );
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    assert_eq!(acme["enabled"], false);
    assert_eq!(
        acme["keyConfigured"], true,
        "a disabled provider keeps its credential"
    );
}

#[tokio::test]
async fn removing_the_managed_key_does_not_take_another_row_s_key_with_it() {
    // `inference/key` is ONE address that two rows can read through their
    // own legacy fallback: entry zero's, and managed's. Which of them owns
    // it depends on what entry zero's kind normalises to.
    //
    // Clearing it unconditionally while writing a *different* slug's slot
    // destroyed whatever else was reading it — on a company configured for
    // OpenRouter, removing the managed key silently took the OpenRouter key
    // with it and the row went from "•••• configured" to a bare host. Found
    // in a browser; pinned here.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Entry zero is OpenRouter, with its credential at the legacy address.
    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/managed/key",
        Some(json!({ "key": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let zero = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "openrouter")
        .expect("entry zero is still listed");
    assert_eq!(
        zero["keyConfigured"], true,
        "removing MANAGED's key must not clear a credential another row reads"
    );
}

#[tokio::test]
async fn a_managed_company_does_converge_off_the_legacy_address() {
    // The other half: when entry zero IS managed, the legacy slot is its
    // own, and writing the new address must retire the old one — otherwise
    // a secret is orphaned at an address nothing will ever clear.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "managed", "key": TOKEN })),
    )
    .await;
    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/managed/key",
        Some(json!({ "key": "sk-not-a-real-key" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(
        dto["managed"]["source"], "provider_key",
        "the new address is what answers now"
    );
}

#[tokio::test]
async fn the_default_is_explicit_and_survives_a_delete_that_is_not_it() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    for label in ["First", "Second"] {
        send(
            &state,
            "POST",
            "/api/v1/company/inference/providers",
            Some(json!({ "kind": "custom", "label": label, "baseUrl": UNREACHABLE, "model": "acme-model" })),
        )
        .await;
    }

    // X1 (2026-09-15): the first provider added auto-becomes the default,
    // with no marker the operator set explicitly — but the observable
    // answer is the same one "list order" used to give.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(default_slug(&dto).as_deref(), Some("first"));

    // Marked, it is a thing the operator said rather than a thing that
    // happened.
    let (status, _, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/default",
        Some(json!({ "model": "second-model" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(default_slug(&dto).as_deref(), Some("second"));

    // Deleting the one that is NOT the default leaves the marker alone —
    // the failure the marker exists to prevent is the default moving with
    // list order, silently.
    send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/first",
        None,
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(default_slug(&dto).as_deref(), Some("second"));
}

#[tokio::test]
async fn disabling_or_deleting_the_default_never_leaves_it_marked() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    for label in ["First", "Second"] {
        send(
            &state,
            "POST",
            "/api/v1/company/inference/providers",
            Some(json!({ "kind": "custom", "label": label, "baseUrl": UNREACHABLE, "model": "acme-model" })),
        )
        .await;
    }
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/default",
        Some(json!({ "model": "second-model" })),
    )
    .await;

    // Switched off: the stored marker itself is left exactly as it was
    // (X14) — see `a_delete_disable_or_key_clear_never_rewrites_the_stored_default_marker`
    // below for the direct assertion on the raw value — but the
    // **derived** `isDefault`/`default_slug` view reports no row at all
    // (round-3a review P2-3), not the first enabled provider. F6 means a
    // turn never falls back to `first` once the full default is broken —
    // it fails closed — so no row may claim to be serving in its place.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/enabled",
        Some(json!({ "enabled": false, "confirmInUse": true })),
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(
        default_slug(&dto),
        None,
        "a broken full default must not be reported as served by a different row"
    );
    assert_eq!(dto["defaultChoice"]["provider"], "second");
    assert_eq!(dto["defaultChoice"]["broken"], true);

    // And a delete leaves the same derived view unchanged.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/enabled",
        Some(json!({ "enabled": true })),
    )
    .await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/default",
        Some(json!({ "model": "second-model" })),
    )
    .await;
    send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/second?confirmInUse=true",
        None,
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(
        default_slug(&dto),
        None,
        "a deleted full default's row is gone entirely — still no fallback claims it"
    );
    assert_eq!(dto["defaultChoice"]["broken"], true);
}

/// Keys rework (#2306), decision D-never-clear-default (X14, 2026-09-15):
/// disabling, clearing the key of, or deleting the provider
/// `inference/default` names never rewrites that **stored** value.
/// Round-3a review P2-3: the derived `isDefault`/`defaultChoice` view
/// reports the break honestly instead — no row falls back to claiming
/// `isDefault` in the broken default's place. This is the regression
/// `disabling_or_deleting_the_default_never_leaves_it_marked` above
/// cannot catch, because it only reads that derived view (which
/// already looked the same whether or not the raw marker was cleared).
#[tokio::test]
async fn a_delete_disable_or_key_clear_never_rewrites_the_stored_default_marker() {
    use crate::company::inference::store;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let id = CompanyId::new("acme");

    for label in ["First", "Second"] {
        send(
            &state,
            "POST",
            "/api/v1/company/inference/providers",
            Some(json!({ "kind": "custom", "label": label, "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "acme-model" })),
        )
        .await;
    }
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/default",
        Some(json!({ "model": "second-model" })),
    )
    .await;

    async fn raw_marker(state: &AppState, id: &CompanyId) -> Option<String> {
        let runtime = state.registry().get(id).expect("registered");
        let secrets = runtime.secrets();
        store::load_default_slug(id, secrets.as_ref())
            .await
            .unwrap()
    }
    assert_eq!(raw_marker(&state, &id).await.as_deref(), Some("second"));

    // Disabling it: the stored marker is untouched. `second` is in use
    // (it is the default), so the guard needs confirmation.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/second/enabled",
        Some(json!({ "enabled": false, "confirmInUse": true })),
    )
    .await;
    assert_eq!(
        raw_marker(&state, &id).await.as_deref(),
        Some("second"),
        "a disable must not rewrite inference/default"
    );

    // Clearing its key (edit with an empty key): still untouched. Also
    // guarded, for the same reason.
    send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/second",
        Some(json!({ "key": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(
        raw_marker(&state, &id).await.as_deref(),
        Some("second"),
        "a key clear must not rewrite inference/default"
    );

    // Deleting it: still untouched, even though no row now answers to it.
    send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/second?confirmInUse=true",
        None,
    )
    .await;
    assert_eq!(
        raw_marker(&state, &id).await.as_deref(),
        Some("second"),
        "a delete must not rewrite inference/default"
    );

    // The derived view still degrades gracefully — this is what the
    // console's status read and banner are for. No row claims to be the
    // default in `second`'s place (round-3a review P2-3), and the status
    // still names `second` as the (broken) stored choice rather than
    // hiding it.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(default_slug(&dto), None);
    assert_eq!(dto["defaultChoice"]["provider"], "second");
    assert_eq!(dto["defaultChoice"]["broken"], true);
}

#[tokio::test]
async fn a_provider_that_is_switched_off_cannot_be_made_the_default() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-model" })),
    )
    .await;
    // `acme` auto-became the default on that add (X1), so disabling it
    // needs confirmation.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/enabled",
        Some(json!({ "enabled": false, "confirmInUse": true })),
    )
    .await;

    let (status, _, _) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/default",
        Some(json!({ "model": "acme-model" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- 2c: a model is required everywhere ---------------------------------

#[tokio::test]
async fn setting_a_default_requires_a_model() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;

    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/default",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{raw}");
    assert!(
        err["error"].as_str().unwrap().contains("Choose a model"),
        "{err}"
    );
}

#[tokio::test]
async fn a_default_model_may_not_be_a_tier_name_or_contain_spaces() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;

    for bad in ["chat-v1", "test model", &"x".repeat(257)] {
        let (status, _, raw) = send(
            &state,
            "POST",
            "/api/v1/company/inference/providers/acme/default",
            Some(json!({ "model": bad })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {raw}");
    }
    let (status, _, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/default",
        Some(json!({ "model": "x".repeat(256) })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "256 chars is exactly the bound: {raw}"
    );
}

#[tokio::test]
async fn an_add_without_a_model_is_refused_before_anything_is_written() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!raw.contains("sk-not-a-real-key"), "{raw}");
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(dto["providers"].as_array().unwrap().is_empty());

    let (status, _, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "acme-1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
}

#[tokio::test]
async fn editing_a_providers_model_is_no_longer_silently_dropped() {
    // Round-2 console review: `EditProvider` used to take only a `models`
    // map, so the console's `model` field was silently ignored while a
    // success toast showed.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/acme",
        Some(json!({ "model": "acme-2" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    let acme = resp["status"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    assert_eq!(acme["model"], "acme-2");
    assert_eq!(acme["models"]["chat-v1"], "acme-2");

    // `acme` auto-became the default (X1), so its model moved with the
    // row (2c: editing the default row's model moves the default too).
    assert_eq!(resp["status"]["defaultChoice"]["model"], "acme-2");
}

#[tokio::test]
async fn status_reports_the_default_choice_and_each_rows_model() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(dto["defaultChoice"].is_null());

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    assert_eq!(acme["model"], "acme-1");
    assert_eq!(acme["modelAmbiguous"], false);
    assert_eq!(dto["defaultChoice"]["provider"], "acme");
    assert_eq!(dto["defaultChoice"]["model"], "acme-1");
}

/// Pure: the bare-slug/unset/full mappings, independent of a store.
#[test]
fn the_status_maps_a_bare_slug_default_to_a_null_model() {
    use crate::company::inference::store::{DefaultChoice, ModelChoice};

    assert!(default_choice_dto(&DefaultChoice::Unset, false).is_none());

    let bare =
        default_choice_dto(&DefaultChoice::ProviderOnly("acme".to_string()), false).unwrap();
    assert_eq!(bare.provider, "acme");
    assert!(bare.model.is_none());
    assert!(!bare.broken, "a bare-slug default is never reported broken");

    let full = default_choice_dto(
        &DefaultChoice::Full(ModelChoice {
            provider: "acme".to_string(),
            model: "acme/other-model".to_string(),
        }),
        false,
    )
    .unwrap();
    assert_eq!(full.provider, "acme");
    assert_eq!(full.model.as_deref(), Some("acme/other-model"));
    assert!(!full.broken);

    let full_broken = default_choice_dto(
        &DefaultChoice::Full(ModelChoice {
            provider: "acme".to_string(),
            model: "acme/other-model".to_string(),
        }),
        true,
    )
    .unwrap();
    assert!(full_broken.broken);
}

/// Round-3a review P2-3: only a *full* default can be reported broken —
/// see `default_full_broken`'s own doc for why a bare slug never is.
#[test]
fn default_full_broken_only_ever_fires_for_a_full_default() {
    use crate::company::inference::store::{DefaultChoice, ModelChoice};

    let full = |slug: &str| {
        DefaultChoice::Full(ModelChoice {
            provider: slug.to_string(),
            model: "m".to_string(),
        })
    };

    // The named provider is gone entirely.
    assert!(default_full_broken(
        &full("gone"),
        [("acme", true)].into_iter()
    ));
    // The named provider exists but is switched off.
    assert!(default_full_broken(
        &full("acme"),
        [("acme", false)].into_iter()
    ));
    // The named provider exists and is on: not broken.
    assert!(!default_full_broken(
        &full("acme"),
        [("acme", true)].into_iter()
    ));
    // A bare slug is never "broken" by this predicate, however stale.
    assert!(!default_full_broken(
        &DefaultChoice::ProviderOnly("gone".to_string()),
        std::iter::empty()
    ));
    // Unset is never broken.
    assert!(!default_full_broken(
        &DefaultChoice::Unset,
        std::iter::empty()
    ));
}

/// Pure: `ModelOnRow` collapses to the DTO's `(model, modelAmbiguous)`.
#[test]
fn an_ambiguous_row_reports_no_model_and_the_flag() {
    use crate::company::inference::store::ModelOnRow;

    assert_eq!(
        model_on_row_dto(ModelOnRow::Ambiguous(vec![
            "a".to_string(),
            "b".to_string()
        ])),
        (None, true)
    );
    assert_eq!(model_on_row_dto(ModelOnRow::None), (None, false));
    assert_eq!(
        model_on_row_dto(ModelOnRow::One("a".to_string())),
        (Some("a".to_string()), false)
    );
}

// ---- the in-use guard (docs/key-reworks/in-use-guards.md) ---------------

#[tokio::test]
async fn the_first_provider_and_model_added_becomes_the_default_with_no_opt_out() {
    // Decision D-first-default (X1, 2026-09-15): no `makeDefault` sent.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "First", "baseUrl": UNREACHABLE, "model": "first-model" })),
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["defaultChoice"]["provider"], "first");
    assert_eq!(dto["defaultChoice"]["model"], "first-model");

    // A second provider never touches an existing default (X1's other half).
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Second", "baseUrl": UNREACHABLE, "model": "second-model" })),
    )
    .await;
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["defaultChoice"]["provider"], "first");
}

/// Round-3a review, P0: "no stored default" is not "the first provider
/// ever" — every company that predates this rework has no stored default,
/// so testing only `Unset` would silently reroute an existing company's
/// traffic onto the next thing an operator "tried out". A company with an
/// existing row must not auto-default a second one.
#[tokio::test]
async fn adding_a_second_provider_to_a_non_empty_company_never_auto_defaults() {
    use crate::company::inference::store;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    // Planted straight into the store — a row from before this feature
    // existed, with no default marker of any kind, which is exactly the
    // pre-rework shape the P0 bug mishandled.
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    store::put_provider(
        runtime.id(),
        runtime.secrets().as_ref(),
        store::ProviderDraft {
            slug: "already-here".into(),
            label: "Already here".into(),
            kind: "custom".into(),
            base_url: UNREACHABLE.into(),
            models: BTreeMap::from([("chat".to_string(), "existing-model".to_string())]),
            enabled: true,
        },
    )
    .await
    .unwrap();

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Second", "baseUrl": UNREACHABLE, "model": "second-model" })),
    )
    .await;
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(
        dto["defaultChoice"].is_null(),
        "a company that already had a provider row must not auto-default its next add: {raw}"
    );
}

/// Round-3a review, P0: a legacy entry-zero company (the flat
/// `inference/config` slot every pre-rework company already resolves
/// through) adding its first *console* provider must not auto-default —
/// entry zero is already an effective default, so this is not that
/// company's first provider.
#[tokio::test]
async fn adding_a_provider_to_a_legacy_entry_zero_company_never_auto_defaults() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openai_compatible", "baseUrl": UNREACHABLE })),
    )
    .await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Second", "baseUrl": UNREACHABLE, "model": "second-model" })),
    )
    .await;
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(
        dto["defaultChoice"].is_null(),
        "a legacy entry-zero company's next add must not auto-default: {raw}"
    );
}

/// Round-3a review, P0: same guard, for a company whose inference comes
/// from a manifest `[inference]` section rather than a console row.
#[tokio::test]
async fn adding_a_provider_to_a_manifest_inference_company_never_auto_defaults() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(
        &home,
        "acme",
        r#"[company]
name = "Acme"
[policy]
mode = "full"

[inference]
provider = "openai_compatible"
base_url = "http://127.0.0.1:9/v1"
"#,
    )
    .await;

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Second", "baseUrl": UNREACHABLE, "model": "second-model" })),
    )
    .await;
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(
        dto["defaultChoice"].is_null(),
        "a manifest-inference company's first console add must not auto-default: {raw}"
    );
}

#[tokio::test]
async fn deleting_the_default_provider_is_refused_without_confirmation() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;

    let (status, err, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "in_use");
    assert!(err["error"].as_str().unwrap().contains("Acme"), "{err}");
    assert_eq!(err["usedBy"]["default"], true);

    // The row must still be there: a refusal writes nothing.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert!(
        dto["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["slug"] == "acme"),
        "{dto}"
    );

    // Confirmed, it proceeds and echoes what it broke.
    let (status, resp, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme?confirmInUse=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["usedBy"]["default"], true);
}

/// Keys rework, issue #2306, slice 3a: a provider named by an agent's own
/// pin is `usedBy` on every status read and refused on delete without
/// confirmation — the counterpart of the `default` guard above, now that
/// `Agent.provider` exists to name.
#[tokio::test]
async fn a_provider_pinned_by_an_agent_is_used_by_that_agent() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let pinned_manifest: CompanyManifest = toml::from_str(
        r#"[company]
name = "Acme"
[policy]
mode = "full"

[[agent]]
id = "researcher"
role = "Researcher"
provider = "acme"
model = "test-model-large"

[[agent]]
id = "writer"
role = "Writer"
"#,
    )
    .unwrap();
    save_record(&home, &id, &pinned_manifest).await;
    let runtime = RuntimeBuilder::new(home.clone(), pinned_manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    // The provider does not exist yet — no row, no default, no pin can
    // resolve — so nothing is used yet.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(
            json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "test-model-large" }),
        ),
    )
    .await;

    // Status names the pinning agent on the row itself.
    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    let agent_ids: Vec<&str> = acme["usedBy"]["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("no usedBy.agents on {acme}: {raw}"))
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(agent_ids, vec!["researcher"], "{acme}");
    assert_eq!(
        acme["usedBy"]["agents"][0]["name"], "Researcher",
        "the display name, not the bare id: {acme}"
    );
    // The writer, which names no pair, must not appear.
    assert!(
        !agent_ids.contains(&"writer"),
        "an agent with no pair must not be counted: {acme}"
    );

    // Deleting the pinned provider is refused the same way the default is.
    let (status, err, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "in_use");
    assert_eq!(
        err["usedBy"]["agents"][0]["id"], "researcher",
        "the refusal must name the pinning agent: {err}"
    );

    // Confirmed, it proceeds and echoes the same agent.
    let (status, resp, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme?confirmInUse=true",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["usedBy"]["agents"][0]["id"], "researcher");
}

/// Bug KR-L2-01 (live E2E): the same guard, but the pin is set through the
/// real write path an operator actually uses — `PATCH …/team/{id}` on a
/// manifest teammate with no pair of its own yet, which stores an
/// `AgentOverride` rather than editing `company.toml`. `usedBy.agents`
/// must see it exactly as it sees a manifest-declared pair.
#[tokio::test]
async fn a_provider_pinned_through_the_team_patch_route_is_used_by_that_agent() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let manifest: CompanyManifest = toml::from_str(
        r#"[company]
name = "Acme"
[policy]
mode = "full"

[[agent]]
id = "researcher"
role = "Researcher"
"#,
    )
    .unwrap();
    save_record(&home, &id, &manifest).await;
    let runtime = RuntimeBuilder::new(home, manifest)
        .with_id(id)
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state
        .registry()
        .insert(CompanyId::new("acme"), std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(
            json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "test-model-large" }),
        ),
    )
    .await;

    // The pin is set through the team PATCH route, not the manifest.
    let (status, patched, raw) = send(
        &state,
        "PATCH",
        "/api/v1/company/team/researcher",
        Some(json!({ "provider": "acme", "model": "test-model-large" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(patched["provider"], "acme", "{patched}");

    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    let acme = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["slug"] == "acme")
        .unwrap();
    let agent_ids: Vec<&str> = acme["usedBy"]["agents"]
        .as_array()
        .unwrap_or_else(|| panic!("no usedBy.agents on {acme}: {raw}"))
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(agent_ids, vec!["researcher"], "{acme}");

    let (status, err, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/acme",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "in_use");
    assert_eq!(err["usedBy"]["agents"][0]["id"], "researcher", "{err}");
}

#[tokio::test]
async fn disabling_the_default_provider_is_refused_without_confirmation() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-1" })),
    )
    .await;

    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/enabled",
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "in_use");

    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers/acme/enabled",
        Some(json!({ "enabled": false, "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["usedBy"]["default"], true);
}

#[tokio::test]
async fn clearing_the_key_of_the_default_provider_is_refused_without_confirmation() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "key": "sk-not-a-real-key", "model": "acme-1" })),
    )
    .await;

    let (status, err, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/acme",
        Some(json!({ "key": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "in_use");

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/providers/acme",
        Some(json!({ "key": "", "confirmInUse": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["usedBy"]["default"], true);
    // Q8, generalized (X14): the confirmed clear never moves the default.
    assert_eq!(resp["status"]["defaultChoice"]["provider"], "acme");
}

#[tokio::test]
async fn a_provider_not_in_use_needs_no_confirmation() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "First", "baseUrl": UNREACHABLE, "model": "first-model" })),
    )
    .await;
    // Second is not the default (X1 never moved it there), so removing it
    // needs no confirmation and its `usedBy` is absent.
    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Second", "baseUrl": UNREACHABLE, "model": "second-model" })),
    )
    .await;

    let (status, resp, raw) = send(
        &state,
        "DELETE",
        "/api/v1/company/inference/providers/second",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert!(resp.get("usedBy").is_none(), "{resp}");
}

/// The slug the status reports as the default, if any.
fn default_slug(dto: &Value) -> Option<String> {
    dto["providers"]
        .as_array()?
        .iter()
        .find(|p| p["isDefault"] == true)
        .and_then(|p| p["slug"].as_str())
        .map(str::to_string)
}

#[tokio::test]
async fn a_route_naming_a_provider_nobody_holds_is_refused() {
    // Fail closed. Accepting it and letting the turn discover it would
    // attribute that workload's spend to whatever the fallback happened to
    // be — the same defect as resolving an unknown provider kind.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, err, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": { "chat-v1": "ghost:gpt-5" } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        err["error"].as_str().unwrap_or_default().contains("ghost"),
        "the error names the slug that resolved to nothing: {err}"
    );
}

#[tokio::test]
async fn a_tier_this_runtime_does_not_have_is_refused() {
    // The five-row trap: `coding` maps onto the same `agentic-v1` tier as
    // `agentic`, so a `coding-v1` route would write one tier's route under a
    // second name and setting one would silently change the other.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": { "coding-v1": "managed" } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_routing_mode_is_inferred_from_the_routes() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // **The reported defect, at the HTTP boundary.** An empty table used to
    // answer `managed` on a company whose managed chain resolves to nothing,
    // while every unset row resolved to `Resolution::Primary` — the first
    // enabled provider. The screen named one destination and the turn used
    // another.
    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert_eq!(
        routes["mode"], "unset",
        "nothing set is not a mode when Managed cannot answer"
    );

    // Give the chain something to resolve to, and the same empty table is
    // genuinely Managed — the inference is about what the company can use,
    // not about the table alone.
    send(
        &state,
        "PUT",
        "/api/v1/company/inference/managed/key",
        Some(json!({ "key": "th-not-a-real-key" })),
    )
    .await;
    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert_eq!(
        routes["mode"], "managed",
        "nothing set is managed once managed answers"
    );

    send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-model" })),
    )
    .await;
    let (_, routes, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": {
            "chat-v1": "acme:gpt-5",
            "reasoning-v1": "acme:gpt-5",
            "agentic-v1": "acme:gpt-5",
            "vision-v1": "acme:gpt-5",
        }})),
    )
    .await;
    assert_eq!(routes["mode"], "own", "every row the same is own");

    let (_, routes, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({ "routes": {
            "chat-v1": "acme:gpt-5",
            "reasoning-v1": "managed",
        }})),
    )
    .await;
    assert_eq!(routes["mode"], "advanced");
}

#[tokio::test]
async fn the_only_provider_a_company_can_use_is_routed_to() {
    // §4. Nothing authored, no managed credential, one provider added: there
    // is precisely one thing in this company that can serve a turn, so
    // routing to anything else is not a choice that exists. Without this the
    // operator adds a provider, every screen says Managed, and every turn
    // goes to the provider anyway — with no per-tier model, which is the
    // reported `404 model: agentic-v1`.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, added, _) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE, "model": "acme-model" })),
    )
    .await;
    assert_eq!(
        added["affectedTiers"].as_array().map(Vec::len),
        Some(4),
        "the write says which rows it wrote rather than leaving them to be noticed: {added}"
    );

    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert_eq!(
        routes["mode"], "own",
        "one provider on every row is own: {routes}"
    );
    assert_eq!(routes["routes"]["agentic-v1"], "acme");
}

#[tokio::test]
async fn a_provider_added_beside_managed_is_not_routed_to() {
    // Row B2, and the case the guard exists for: Managed resolves, so adding
    // a key may be for one workload, for vision only, or to compare. Writing
    // all four rows would bill the operator for everything, silently, from a
    // screen that still says Managed. The answer is to ask, which is what
    // leaving the table empty does.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    send(
        &state,
        "PUT",
        "/api/v1/company/inference/managed/key",
        Some(json!({ "key": "th-not-a-real-key" })),
    )
    .await;
    let (_, added, _) = send(
        &state,
        "POST",
        "/api/v1/company/inference/providers",
        Some(json!({ "kind": "custom", "label": "Acme", "baseUrl": UNREACHABLE })),
    )
    .await;
    assert!(
        added["affectedTiers"]
            .as_array()
            .is_none_or(|tiers| tiers.is_empty()),
        "nothing was routed on the operator's behalf: {added}"
    );

    let (_, routes, _) = send(&state, "GET", "/api/v1/company/inference/routes", None).await;
    assert_eq!(
        routes["mode"], "managed",
        "the table is still empty: {routes}"
    );
}

#[tokio::test]
async fn the_draft_probe_refuses_the_metadata_address() {
    // The SSRF answer, made explicitly rather than inherited. That range is
    // where a container's credentials live and a company's model endpoint is
    // never there.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, resp, raw) = send(
        &state,
        "POST",
        "/api/v1/company/inference/probe",
        Some(json!({ "baseUrl": "http://169.254.169.254/latest/meta-data", "key": "sk-not-a-real-key" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["ok"], false);
    assert_eq!(resp["class"], "endpoint");
}

// --- Issue #266: a save the running brain cannot honour --------------------

/// On a host with no harness reachable, a saved config is *never* "restart
/// pending" — the echo brain is where this build ends up no matter how many
/// times it is restarted, and telling the operator otherwise would send them
/// bouncing a process for nothing.
///
/// This is the default build, so it is also the guard that keeps the flag
/// from firing on every self-hosted instance that simply has no local
/// inference compiled in.
#[tokio::test]
async fn a_save_is_not_restart_pending_when_no_restart_would_help() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, resp, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The config landed...
    assert_eq!(resp["status"]["source"], "runtime");
    assert_eq!(resp["status"]["keyConfigured"], true);
    // ...and the runtime is on the echo brain, but no harness is reachable
    // here, so there is nothing a restart would change.
    assert_eq!(resp["status"]["cognition"], "echo");
    assert_eq!(resp["status"]["restartRequired"], false);
    assert!(
        resp["note"]
            .as_str()
            .unwrap_or_default()
            .contains("no restart needed"),
        "{}",
        resp["note"]
    );

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["restartRequired"], false);
}

/// A company with nothing configured has nothing stranded, so the flag is
/// off even before any save.
#[tokio::test]
async fn an_unconfigured_company_is_not_restart_pending() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["source"], "managed");
    assert_eq!(dto["restartRequired"], false);
}

/// Issue #266, reproduced at the route: a company built with a harness pool
/// but **no** inference source boots onto the echo brain with an unwired
/// workflow runner. Storing a credential afterwards updates the secret store
/// and nothing else — the brain is chosen in `RuntimeBuilder::build` and this
/// one already ran.
///
/// So the save must report `restartRequired`, the note must say restart
/// rather than "next turn", and `POST …/workflows/{id}/run` must stop
/// claiming the deployment lacks workflow execution when what it lacks is a
/// restart.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn configuring_inference_after_boot_reports_restart_required() {
    use crate::harness::HarnessPool;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();

    // Build the company the way the serve path does — with a harness pool
    // attached — but with no inference source of any kind. That is the boot
    // this issue is about.
    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .with_harness(std::sync::Arc::new(HarnessPool::new()))
        .build()
        .await
        .unwrap();
    // The bug's precondition, asserted rather than assumed: the harness was
    // available and the company still landed on the offline brain.
    assert_eq!(
        runtime.cognition().path,
        "echo",
        "expected the no-inference boot to select the echo brain"
    );
    assert!(
        runtime.workflow_runner().is_none(),
        "expected the no-inference boot to leave the workflow runner unwired"
    );

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    // Configure inference, exactly as the console does.
    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": "https://stub.invalid/v1",
            "key": TOKEN,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(resp["status"]["source"], "runtime");
    assert_eq!(resp["status"]["keyConfigured"], true);
    // The config resolves now; the running brain still does not know it.
    assert_eq!(resp["status"]["cognition"], "echo");
    assert_eq!(resp["status"]["restartRequired"], true, "{raw}");
    // A pool is attached on this build, so the design path is reachable —
    // the flag the setup dialog reads to keep the "set up a model" CTA.
    assert_eq!(resp["status"]["harnessReachable"], true, "{raw}");

    let note = resp["note"].as_str().unwrap_or_default();
    assert!(note.contains("restart"), "note must say restart: {note}");
    assert!(
        !note.contains("next turn"),
        "note must not promise the next turn: {note}"
    );
    assert!(!raw.contains(TOKEN), "PUT response leaked the token: {raw}");

    // The flag is a property of the runtime, not of the mutation, so a plain
    // read reports it too — this is what keeps the warning on screen after
    // the toast is gone.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["restartRequired"], true);

    // Second surface: the run route no longer blames the deployment.
    let (status, err, _) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(err["code"], "restart_required");
    assert!(
        err["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Restart"),
        "{err}"
    );

    // Reverting to managed un-strands the company: there is no longer a
    // saved config waiting on a restart, so the flag clears.
    let (_, resp, _) = send(&state, "DELETE", "/api/v1/company/inference", None).await;
    assert_eq!(resp["status"]["restartRequired"], false);

    // #514: ...and with nothing pending BUT the harness reachable here,
    // the run route no longer 404s "no workflow execution" — it names the
    // real dead end, that this company never configured inference. (Before
    // #514 this asserted 404 `not_wired`; that was the third, unhandled
    // cause the console read as a permanent gap and degraded to read-only.)
    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "inference_required");
}

/// The reported company at the HTTP boundary: every tier routed to
/// `managed`, a managed key stored, and nothing else configured.
///
/// The unit half of this lives in
/// [`crate::company::inference`] — this is the same defect seen from the two
/// routes an operator actually meets. Managed has no row in
/// `inference/providers` and writes neither the legacy runtime blob nor a
/// manifest block, so before the third branch landed in
/// `resolve_effective_scoped` this company resolved `None`, booted onto the
/// offline echo brain, and stayed there across a restart — while the console
/// showed Managed available on both tabs and the chat pane said "no model
/// configured".
///
/// `restartRequired` needs no widening of its own: it is `restart_pending`
/// over the same resolver, so fixing the resolver fixes the banner, and the
/// run route's `RunnerGap` inherits it for free. Both are asserted here,
/// because "the resolver is right but nothing downstream moved" is the
/// failure this whole family of bugs keeps taking.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn configuring_only_managed_after_boot_reports_restart_required() {
    use crate::harness::HarnessPool;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();

    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .with_harness(std::sync::Arc::new(HarnessPool::new()))
        .build()
        .await
        .unwrap();
    assert_eq!(
        runtime.cognition().path,
        "echo",
        "expected the no-inference boot to select the echo brain"
    );

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    // Nothing configured yet: no legacy config, no providers, no managed
    // credential. The flag must be off, or the assertion below proves
    // nothing.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["restartRequired"], false);
    assert_eq!(dto["managed"]["configured"], false);

    // Configure Managed the way the console does — its own key route, then
    // the routing table pointed at it. Neither writes `inference/config`.
    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/managed/key",
        Some(json!({ "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (status, _, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference/routes",
        Some(json!({
            "routes": {
                "chat-v1": "managed",
                "reasoning-v1": "managed",
                "agentic-v1": "managed",
                "vision-v1": "managed"
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");

    let (_, dto, raw) = send(&state, "GET", "/api/v1/company/inference", None).await;
    // Managed resolves, and the running brain is still the one boot chose —
    // which together are what `restartRequired` is supposed to mean.
    assert_eq!(dto["managed"]["configured"], true, "{raw}");
    assert_eq!(dto["managed"]["source"], "provider_key", "{raw}");
    assert_eq!(dto["cognition"], "echo", "{raw}");
    assert_eq!(dto["harnessReachable"], true, "{raw}");
    // The regression itself.
    assert_eq!(dto["restartRequired"], true, "{raw}");
    assert!(!raw.contains(TOKEN), "GET response leaked the token: {raw}");

    // Second surface, same widening: `runner_gap_for` classified this
    // company as `not_wired` for the identical reason.
    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "restart_required", "{raw}");
}

/// A brain standing in for the one a rebuild puts a configured company on,
/// so the route's behaviour is pinned without constructing a real harness
/// brain (which would resolve MCP servers, Composio and an agent roster
/// inside a unit test).
///
/// Only its [`Cognition`](crate::ports::Cognition) matters here: reporting
/// the harness path is exactly what makes `restart_pending` false, which is
/// the observable the console reads.
#[cfg(feature = "openhuman")]
struct RebuiltBrain;

#[cfg(feature = "openhuman")]
#[async_trait::async_trait]
impl crate::ports::brain::Brain for RebuiltBrain {
    async fn run_cycle(
        &self,
        _req: crate::ports::types::CycleRequest,
        _host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        Ok(crate::ports::types::CycleResult {
            channel_responses: Vec::new(),
            new_traces: Vec::new(),
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }

    fn cognition(&self) -> crate::ports::Cognition {
        crate::ports::Cognition {
            path: crate::ports::brain::HARNESS_PATH,
            // A stub brain meters per turn and reports zero usage, so
            // nothing is double-counted (see `Brain::cognition`).
            provider: "stub",
            model: None,
            metering: crate::ports::UsageMetering::PerTurn,
        }
    }
}

/// A rebuilder that runs the real builder over the real handover, and puts
/// the successor on a brain that reports the harness path — the shape a live
/// rebuild produces once inference resolves.
#[cfg(feature = "openhuman")]
struct StubRebuilder {
    home: std::path::PathBuf,
}

#[cfg(feature = "openhuman")]
#[async_trait::async_trait]
impl crate::runtime::RuntimeRebuilder for StubRebuilder {
    async fn rebuild(
        &self,
        _state: &AppState,
        request: crate::runtime::RebuildRequest,
    ) -> crate::Result<crate::company::runtime::CompanyRuntime> {
        RuntimeBuilder::new(self.home.clone(), request.manifest)
            .with_id(request.id)
            .with_harness(std::sync::Arc::new(crate::harness::HarnessPool::new()))
            .with_brain(std::sync::Arc::new(RebuiltBrain))
            .with_handover(request.handover)
            .build()
            .await
    }
}

/// Issue #290, the half #266 did not ship: with a rebuilder wired, the save
/// that *would* have set `restartRequired` rebuilds the company instead, and
/// the response reports the successor rather than the runtime that is being
/// replaced.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn configuring_inference_after_boot_rebuilds_instead_of_asking_for_a_restart() {
    use crate::harness::HarnessPool;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .with_harness(std::sync::Arc::new(HarnessPool::new()))
        .build()
        .await
        .unwrap();
    assert_eq!(runtime.cognition().path, "echo");

    let outgoing = std::sync::Arc::new(runtime);
    let state = AppState::new(AppConfig::default())
        .with_rebuilder(std::sync::Arc::new(StubRebuilder { home: home.clone() }));
    state.registry().insert(id.clone(), outgoing.clone());
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    let (status, resp, raw) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({
            "provider": "openai_compatible",
            "baseUrl": "https://stub.invalid/v1",
            "key": TOKEN,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    // The save landed, and the company is no longer stranded behind a boot
    // decision: it reports the live cognition path, not "restart me".
    assert_eq!(resp["status"]["source"], "runtime");
    assert_eq!(resp["status"]["keyConfigured"], true);
    assert_eq!(resp["status"]["cognition"], "harness", "{raw}");
    assert_eq!(resp["status"]["restartRequired"], false, "{raw}");
    let note = resp["note"].as_str().unwrap_or_default();
    assert!(!note.contains("restart the company"), "{note}");
    assert!(!raw.contains(TOKEN), "PUT response leaked the token: {raw}");

    // The registry really was swapped, and the replaced runtime is left
    // quiesced so nothing still holding it keeps driving a dead company.
    let registered = state.registry().get(&id).expect("still registered");
    assert!(!std::sync::Arc::ptr_eq(&registered, &outgoing));
    assert!(outgoing.is_quiesced());
    assert!(!registered.is_quiesced());

    // A plain read agrees: the banner clears itself rather than needing the
    // console to remember what the mutation said.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["restartRequired"], false);
    assert_eq!(dto["cognition"], "harness");
}

/// Issue #514, case (c): a company built with a harness pool that never
/// configured any inference source. `workflow_runner()` is `None` — but not
/// because the deployment lacks execution (the harness is right here) and
/// not because a saved config is stranded behind a boot decision (nothing
/// was ever saved). The run route must name the real dead end:
/// `inference_required` (409), pointing the operator at Settings — NOT the
/// `not_wired` 404 the console reads as a permanent gap and degrades to a
/// read-only view on.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn running_a_workflow_with_no_inference_configured_asks_to_configure_inference() {
    use crate::harness::HarnessPool;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();

    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .with_harness(std::sync::Arc::new(HarnessPool::new()))
        .build()
        .await
        .unwrap();
    // Precondition (case c): the harness was reachable, nothing is
    // configured, and the company still landed with no runner — the exact
    // shape that used to 404 `not_wired`.
    assert_eq!(runtime.cognition().path, "echo");
    assert!(runtime.workflow_runner().is_none());

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    // Nothing configured — the status read agrees.
    let (_, dto, _) = send(&state, "GET", "/api/v1/company/inference", None).await;
    assert_eq!(dto["source"], "managed");
    assert_eq!(dto["restartRequired"], false);

    // The run route names the fix instead of blaming the deployment.
    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{raw}");
    assert_eq!(err["code"], "inference_required");
    let msg = err["error"].as_str().unwrap_or_default();
    assert!(
        msg.contains("inference"),
        "message must name inference: {msg}"
    );
    assert!(
        msg.contains("Settings"),
        "message must point at Settings: {msg}"
    );
    assert!(
        !msg.contains("not wired"),
        "message must not say 'not wired': {msg}"
    );
    assert!(
        !msg.contains("deployment"),
        "message must not blame the deployment: {msg}"
    );

    // The dead end left nothing behind — run history is still empty.
    let (status, runs, raw) = send(&state, "GET", "/api/v1/company/workflows/runs", None).await;
    assert_eq!(status, StatusCode::OK, "{raw}");
    assert_eq!(runs["runs"].as_array().map(Vec::len), Some(0), "{runs}");
}

/// Issue #514 (review): a config that cannot be *read* is not evidence to
/// tell the operator to configure inference. When `resolve_effective`
/// returns `Err` — the secret store is unreachable — the classifier must
/// degrade to `NotWired` (404), never `inference_required` (409), even with
/// a harness right here. A 409 would promise a fix ("configure inference")
/// that a resolve *failure* gives no reason to expect; the #266 doctrine is
/// that an unreadable config is not evidence a save would help. Guards the
/// `Err(_) => (false, false)` arm of `runner_gap_for` against a regression
/// back to the old `is_ok_and`, which folded `Err` into "not configured".
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_resolve_error_stays_not_wired_even_with_a_harness() {
    use crate::harness::HarnessPool;
    use crate::ports::types::SecretValue;

    struct FailingSecrets;
    #[async_trait::async_trait]
    impl crate::ports::SecretStore for FailingSecrets {
        async fn get(&self, _c: &CompanyId, _key: &str) -> crate::Result<Option<SecretValue>> {
            Err(crate::error::OpenCompanyError::Store(
                "secret store unreachable".into(),
            ))
        }
        async fn set(&self, _c: &CompanyId, _key: &str, _v: SecretValue) -> crate::Result<()> {
            Ok(())
        }
    }

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .with_harness(std::sync::Arc::new(HarnessPool::new()))
        .with_secrets(std::sync::Arc::new(FailingSecrets))
        .build()
        .await
        .unwrap();
    assert!(runtime.workflow_runner().is_none());

    // The classifier degrades an unreadable config to NotWired...
    assert!(
        matches!(
            super::runner_gap_for(&runtime).await,
            super::RunnerGap::NotWired
        ),
        "a resolve error must classify as NotWired, not InferenceRequired",
    );

    // ...and the run route answers 404 `not_wired`, not 409 `inference_required`.
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{raw}");
    assert_eq!(err["code"], "not_wired");
}

/// Issue #514 negative control: on the default build (no `openhuman`
/// feature, so no harness is reachable), an unconfigured company still gets
/// the honest `not_wired` 404. The `inference_required` arm must NOT fire
/// where configuring inference would be a false promise — there is no
/// harness here to run it, so a restart or a save changes nothing.
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn running_a_workflow_on_the_default_build_stays_not_wired() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{raw}");
    assert_eq!(err["code"], "not_wired");
}

/// Issue #514 negative control: a company that *has* configured inference
/// but runs on a build with no harness reachable still gets `not_wired`.
/// Neither the `restart_pending` arm (no harness → a restart changes
/// nothing) nor the `inference_required` arm (a config is already present)
/// applies.
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn running_a_workflow_configured_without_a_harness_stays_not_wired() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _, _) = send(
        &state,
        "PUT",
        "/api/v1/company/inference",
        Some(json!({ "provider": "openrouter", "key": TOKEN })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, err, raw) = send(
        &state,
        "POST",
        "/api/v1/company/workflows/daily/run",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{raw}");
    assert_eq!(err["code"], "not_wired");
}
