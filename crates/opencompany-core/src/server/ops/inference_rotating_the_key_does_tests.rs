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
async fn state_with_manifest(home: &std::path::Path, name: &str, manifest_toml: &str) -> AppState {
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

    let (status, body, raw) = send(&state, "GET", "/api/v1/company/inference/models", None).await;

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
    let state = state_with_manifest(home_dir.path(), "credurl-models", CREDENTIALED_MANIFEST).await;

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
    let state = state_with_manifest(home_dir.path(), "credurl-status", CREDENTIALED_MANIFEST).await;

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
