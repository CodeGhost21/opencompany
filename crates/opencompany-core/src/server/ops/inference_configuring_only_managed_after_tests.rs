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
