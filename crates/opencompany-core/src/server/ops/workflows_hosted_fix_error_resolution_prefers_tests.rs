fn own_rows(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("array response")
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            !crate::globals::workflows().iter().any(|w| w.id == id)
        })
        .collect()
}

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{
    CompanyEvent, DEFAULT_RUN_LIMIT, MAX_RUN_ARTIFACTS, WorkflowNodeStatus, WorkflowRunOutcome,
    WorkflowRunVerdict, select_run_page,
};
use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-workflows-hosted-")
        .tempdir()
        .expect("tempdir")
}

/// A manifest declaring one enabled workflow — mirrors what a
/// platform tenant provisions with, minus any `workflows/` directory
/// on disk (there isn't one: hosted tenants have no source dir).
fn manifest_with_enabled() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[workflows]\nenabled = [\"demo\"]\n",
    )
    .unwrap()
}

/// Builds a running company whose runtime has **no source directory**
/// (built without `with_seed_dir`, matching how the platform builds a
/// provisioned tenant) but whose persisted record declares an enabled
/// workflow — the exact hosted-mode gap #70 reports.
async fn state_with_hosted_company(home: &std::path::Path) -> AppState {
    state_with_hosted_company_lifecycle(home, "running").await
}

/// The same fixture at a chosen lifecycle, so a paused company is
/// reachable without a second copy of the record literal.
async fn state_with_hosted_company_lifecycle(home: &std::path::Path, lifecycle: &str) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest_with_enabled(),
            ledger: Vec::new(),
            lifecycle: lifecycle.to_string(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest_with_enabled())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    assert!(
        runtime.source_dir().is_none(),
        "test setup must simulate hosted mode: no source dir"
    );
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// Issue #840 (PR-3): the fix route's error-resolution matrix — a journaled
/// error wins (carrying its failing node), a clean/absent run falls back to
/// the caller's hint, and a clean run with no usable hint is nothing to fix
/// from (a 400). Unit-tested on the pure helper so the whole matrix is
/// pinned without a running host.
#[cfg(feature = "openhuman")]
#[test]
fn fix_error_resolution_prefers_journal_then_hint_then_nothing() {
    use super::{JournaledFailure, resolve_fix_error};
    // A journaled error wins, carrying the failing node id.
    assert_eq!(
        resolve_fix_error(
            Some(JournaledFailure {
                error: Some("boom".to_string()),
                failed_node_id: Some("n1".to_string()),
            }),
            Some("hint".to_string()),
        ),
        Some(("boom".to_string(), Some("n1".to_string())))
    );
    // A run that finished CLEAN (no error) falls back to the hint.
    assert_eq!(
        resolve_fix_error(
            Some(JournaledFailure {
                error: None,
                failed_node_id: None,
            }),
            Some("hint".to_string()),
        ),
        Some(("hint".to_string(), None))
    );
    // No finish for this run id at all → the hint is the only source.
    assert_eq!(
        resolve_fix_error(None, Some("hint".to_string())),
        Some(("hint".to_string(), None))
    );
    // A clean run and no hint → nothing to fix from.
    assert_eq!(
        resolve_fix_error(
            Some(JournaledFailure {
                error: None,
                failed_node_id: None,
            }),
            None
        ),
        None
    );
    // No run and no hint → nothing to fix from.
    assert_eq!(resolve_fix_error(None, None), None);
    // A whitespace-only hint is not usable.
    assert_eq!(resolve_fix_error(None, Some("   ".to_string())), None);
}

/// Journals a `WorkflowRunFinished` naming a `run_id`, the shape
/// `journaled_run_failure` scans for — distinct from `journal_run` above,
/// which always journals `run_id: None` for the delivery-history tests.
#[cfg(feature = "openhuman")]
async fn journal_run_with_id(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    error: &str,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowRunFinished {
                workflow_id: workflow_id.to_string(),
                scheduled: false,
                run_id: Some(run_id.to_string()),
                deliveries: Vec::new(),
                pending_approvals: Vec::new(),
                error: Some(error.to_string()),
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                // Added by #881/#880 after this fixture was written. A
                // failed run parks nothing and blocks nothing, so both
                // are empty here — see `Settled::from`'s Err arm, which
                // makes the same choice for the same reason.
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            },
        )
        .await
        .expect("append");
}

/// Issue #840 (PR-3), tinysweeper finding: the only prior server test for
/// `fix_from_run` exercised the builder-gap path (404/409, above). This is
/// the core of the feature — a valid request with the `openhuman` feature
/// on, asserting a 200 with `automatable: true`, the corrected workflow,
/// and its readiness — proven at the HTTP boundary rather than only at the
/// `fix_workflow_from_failure` unit layer (`workflow_build::test` already
/// covers identity-preservation there).
///
/// Reuses `workflow_build::test`'s scripted-model + `HarnessDeps` fixture
/// (widened to `pub(crate)` for this) rather than hand-rolling a second
/// `HarnessModel`/`HarnessDeps` here — that struct has ~30 fields and
/// duplicating it would drift silently the next time one is added.
///
/// A copilot turn nests the provider/tool loop deep enough to overflow
/// tokio's default 2 MiB worker-thread stack — the same exposure
/// `openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES`'s doc comment
/// names for production hosts. Every other agent-turn test in this module
/// (and `workflow_build::test`) is plain `#[tokio::test]` and relies on
/// CI setting `RUST_MIN_STACK=16777216` for this job
/// (`.github/workflows/ci.yml`'s `Rust (openhuman, tinymemory)` lane) —
/// this one follows the same convention rather than wrapping itself in a
/// custom-stack thread, which no sibling test does. Run locally with
/// `RUST_MIN_STACK=16777216 cargo test …` if it overflows outside CI.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn fix_from_run_returns_the_corrected_graph_on_success() {
    use crate::harness::provider::HarnessModel;
    use crate::harness::workflow_build::WorkflowBuilder;
    use crate::harness::workflow_build::test::{
        NativeCopilotModel, NativeStep, agent_deps, propose_step,
    };

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;

    // Wire a builder AND the harness deps `run_copilot` builds its agent
    // from — the route's own capability gate only checks the former, but
    // the copilot needs both (issue #840, PR-2's `HarnessDeps` wiring).
    let model = NativeCopilotModel::scripting(vec![
        propose_step(
            "dropped the unwired step",
            serde_json::json!({
                "name": "Greeter",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done" } ]
            }),
        ),
        NativeStep::done("Corrected the workflow."),
    ]);
    {
        let mut runtime =
            std::sync::Arc::into_inner(state.registry().remove(&id).expect("registered"))
                .expect("uniquely held in this test");
        let deps = agent_deps(&runtime, model.clone() as std::sync::Arc<dyn HarnessModel>);
        runtime.set_builder(std::sync::Arc::new(WorkflowBuilder::new(
            model as std::sync::Arc<dyn HarnessModel>,
            "test-model",
        )));
        runtime.set_workflow_harness_deps(deps);
        state
            .registry()
            .insert(id.clone(), std::sync::Arc::new(runtime));
    }

    // Seed the workflow the run failed on (hosted mode has no source
    // dir, so it exists only as an overlay created via the API).
    let created = router(state.clone())
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows",
            Some(create_body()),
        ))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    journal_run_with_id(
        &state,
        &id,
        "greeter",
        "run-1",
        "the tool `web_search` is not wired on this deployment",
    )
    .await;

    let response = router(state)
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/greeter/fix-from-run",
            Some(serde_json::json!({ "runId": "run-1" })),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["automatable"], true, "body: {body}");
    assert_eq!(
        body["workflow"]["id"], "greeter",
        "the fix keeps the workflow's id"
    );
    assert!(
        body["workflow"]["nodes"]
            .as_array()
            .is_some_and(|n| !n.is_empty()),
        "body: {body}"
    );
    assert!(body["readiness"]["ok"].is_boolean(), "body: {body}");
}

/// A node declared `repeatable = false` is named in the correction's
/// notes, alongside `on_error`/`retry` (issue #850).
///
/// `WorkflowNodeSpec` — what the copilot's builder actually authors —
/// has no `repeatable` field, so a corrected graph silently drops the
/// declaration unless `fix_from_run` names it in `notes`. Without this,
/// an operator who saves a copilot correction over a workflow with a
/// `repeatable: false` node loses that guard with no warning at all.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn fix_from_run_notes_a_dropped_repeatable_declaration() {
    use crate::harness::provider::HarnessModel;
    use crate::harness::workflow_build::WorkflowBuilder;
    use crate::harness::workflow_build::test::{
        NativeCopilotModel, NativeStep, agent_deps, propose_step,
    };

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;

    let model = NativeCopilotModel::scripting(vec![
        propose_step(
            "dropped the unwired step",
            serde_json::json!({
                "name": "Greeter",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done" } ]
            }),
        ),
        NativeStep::done("Corrected the workflow."),
    ]);
    {
        let mut runtime =
            std::sync::Arc::into_inner(state.registry().remove(&id).expect("registered"))
                .expect("uniquely held in this test");
        let deps = agent_deps(&runtime, model.clone() as std::sync::Arc<dyn HarnessModel>);
        runtime.set_builder(std::sync::Arc::new(WorkflowBuilder::new(
            model as std::sync::Arc<dyn HarnessModel>,
            "test-model",
        )));
        runtime.set_workflow_harness_deps(deps);
        state
            .registry()
            .insert(id.clone(), std::sync::Arc::new(runtime));
    }

    // Seed a workflow whose middle node declares `repeatable: false` —
    // the exact declaration `fix_from_run`'s correction cannot carry
    // through the builder's `WorkflowNodeSpec`.
    let mut body = create_body();
    body["nodes"].as_array_mut().unwrap().insert(
        1,
        serde_json::json!({
            "id": "publish",
            "kind": "tool_call",
            "name": "Publish",
            "config": { "slug": "shell", "args": { "command": "./bin/announce" } },
            "repeatable": false
        }),
    );
    body["edges"] = serde_json::json!([
        { "from": "start", "to": "publish" },
        { "from": "publish", "to": "done" }
    ]);
    let created = router(state.clone())
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    journal_run_with_id(
        &state,
        &id,
        "greeter",
        "run-1",
        "the tool `web_search` is not wired on this deployment",
    )
    .await;

    let response = router(state)
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/greeter/fix-from-run",
            Some(serde_json::json!({ "runId": "run-1" })),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let notes = body["notes"].as_array().cloned().unwrap_or_default();
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .is_some_and(|s| s.contains("repeatable") && s.contains("Publish"))),
        "notes must name the dropped repeatable declaration on `Publish`: {body}"
    );
}

/// CodeRabbit review on #1937 (issue #1866): the same drop the sibling
/// test above pins for `repeatable` also applies to `postcondition` —
/// `WorkflowNodeSpec` has no field for it either, so a node's declared
/// run-safety gate is silently dropped by a fix-from-run correction
/// unless `fix_from_run` names it in `notes`. Without this, an operator
/// who saves a copilot correction over a workflow whose agent node
/// declared a `postcondition` loses that gate with no warning — the
/// SAME defect class as thread 1's GET -> PUT erasure, but reached
/// through the agent-authored correction path instead of a REST edit.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn fix_from_run_notes_a_dropped_postcondition_declaration() {
    use crate::harness::provider::HarnessModel;
    use crate::harness::workflow_build::WorkflowBuilder;
    use crate::harness::workflow_build::test::{
        NativeCopilotModel, NativeStep, agent_deps, propose_step,
    };

    let home_dir = home();
    let id = CompanyId::new("acme");
    let state = desk_state(home_dir.path()).await;

    let model = NativeCopilotModel::scripting(vec![
        propose_step(
            "dropped the unwired step",
            serde_json::json!({
                "name": "Greeter",
                "nodes": [
                    { "id": "start", "kind": "trigger", "name": "Start" },
                    { "id": "done", "kind": "output", "name": "Report" }
                ],
                "edges": [ { "from": "start", "to": "done" } ]
            }),
        ),
        NativeStep::done("Corrected the workflow."),
    ]);
    {
        let mut runtime =
            std::sync::Arc::into_inner(state.registry().remove(&id).expect("registered"))
                .expect("uniquely held in this test");
        let deps = agent_deps(&runtime, model.clone() as std::sync::Arc<dyn HarnessModel>);
        runtime.set_builder(std::sync::Arc::new(WorkflowBuilder::new(
            model as std::sync::Arc<dyn HarnessModel>,
            "test-model",
        )));
        runtime.set_workflow_harness_deps(deps);
        state
            .registry()
            .insert(id.clone(), std::sync::Arc::new(runtime));
    }

    // Seed a workflow whose middle node is an agent naming the roster
    // teammate `desk_manifest` declares (`ceo`) and carries a
    // `postcondition` — the exact declaration `fix_from_run`'s
    // correction cannot carry through the builder's `WorkflowNodeSpec`.
    let mut body = create_body();
    body["nodes"].as_array_mut().unwrap().insert(
        1,
        serde_json::json!({
            "id": "ask",
            "kind": "agent",
            "name": "Ask",
            "agent": "ceo",
            "postcondition": { "require": "non_empty" }
        }),
    );
    body["edges"] = serde_json::json!([
        { "from": "start", "to": "ask" },
        { "from": "ask", "to": "done" }
    ]);
    let created = router(state.clone())
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);

    journal_run_with_id(
        &state,
        &id,
        "greeter",
        "run-1",
        "the tool `web_search` is not wired on this deployment",
    )
    .await;

    let response = router(state)
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/greeter/fix-from-run",
            Some(serde_json::json!({ "runId": "run-1" })),
        ))
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let notes = body["notes"].as_array().cloned().unwrap_or_default();
    assert!(
        notes.iter().any(|n| n
            .as_str()
            .is_some_and(|s| s.contains("postcondition") && s.contains("Ask"))),
        "notes must name the dropped postcondition declaration on `Ask`: {body}"
    );
}

/// Issue #783: the per-workflow copilot's tool-grounding read answers
/// `200 {"slugs":[…],"unwired":[…]}` on **both** scope forms — which also
/// proves the static prefix is wired ahead of the dynamic
/// `/workflows/{wid}` (a route-miss, or a `tool-slugs` swallowed as a
/// `wid`, would not be this shape). The blank tenant grants no tools, so
/// both lists are empty here; the point pinned is the contract shape and
/// that the route exists.
///
/// Issue #874 added `unwired` and it is pinned here as **always present**,
/// because the console reads it unconditionally: a body that omitted the
/// key on a wired host would read as "nothing is unwired" and silently
/// restore the bug this route was narrowed to fix.
#[tokio::test]
async fn tool_slugs_answers_a_slug_array_on_both_scope_forms() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, _id) = hosted_state(&home).await;

    for uri in [
        "/api/v1/company/workflows/tool-slugs",
        "/api/v1/companies/acme/workflows/tool-slugs",
    ] {
        let response = router(state.clone())
            .oneshot(request("GET", uri, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "tool-slugs on {uri}");
        let body = json_body(response).await;
        assert!(
            body["slugs"].is_array(),
            "tool-slugs answers a `slugs` array on {uri}, got: {body}"
        );
        assert!(
            body["unwired"].is_array(),
            "tool-slugs answers an `unwired` array on {uri}, got: {body}"
        );
    }
}

/// Issue #874, the staging repro through the route itself: a company that
/// explicitly grants `search`, on a deployment with no managed search
/// backend, must NOT be handed `web_search` to ground a proposal on.
///
/// This is the regression that shipped. The route answered the
/// **grant-only** set, so `web_search` was advertised, the copilot
/// authored a `tool_call` on it, and the run died at the first node with
/// `tool_call 'web_search' is not available in company workflows`. What
/// pins the fix is the pair of assertions: the slug is gone from `slugs`
/// **and** present in `unwired` with a reason — dropping it silently would
/// leave an operator unable to tell "not allowed" from "not configured".
///
/// A granted-and-wired tool (`shell`) stays offered in the same answer, so
/// this cannot pass by narrowing the list to nothing.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn tool_slugs_omits_a_granted_but_unwired_tool_and_says_why() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let mut manifest = empty_manifest();
    manifest.tools.allow = vec!["search".to_string(), "shell".to_string()];

    let store = FsCompanyStore::new(home.clone());
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

    let mut runtime = RuntimeBuilder::new(home.clone(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    // `workflow_wiring_deps` pins `search: None` — the deployment half of
    // the repro. Everything else is allowed, so `shell` stays wired.
    runtime.set_workflow_harness_deps(crate::harness::workflow_wiring_deps(
        &runtime,
        None,
        crate::harness::toolbelt::CapabilityFilter::AllowAll,
        None,
    ));
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    let response = router(state)
        .oneshot(request("GET", "/api/v1/company/workflows/tool-slugs", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    let slugs: Vec<&str> = body["slugs"]
        .as_array()
        .expect("slugs")
        .iter()
        .map(|v| v.as_str().expect("slug"))
        .collect();
    assert!(
        !slugs.contains(&"web_search"),
        "a granted-but-unwired tool is not offered for grounding: {body}"
    );
    assert!(
        slugs.contains(&"shell"),
        "a granted AND wired tool is still offered: {body}"
    );

    let unwired = body["unwired"].as_array().expect("unwired");
    let entry = unwired
        .iter()
        .find(|e| e["slug"] == "web_search")
        .unwrap_or_else(|| panic!("web_search is reported as unwired: {body}"));
    assert_eq!(
        entry["reason"], "searchBackendNotConfigured",
        "the reason distinguishes an unconfigured provider from a filtered \
         capability tier: {body}"
    );
    assert!(
        entry["detail"]
            .as_str()
            .is_some_and(|d| d.contains("search backend")),
        "the prose reason is servable as-is: {body}"
    );
}

/// A create body whose trigger carries a cron.
fn scheduled_create_body() -> serde_json::Value {
    serde_json::json!({
        "id": "digest",
        "name": "Digest",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * *" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
    })
}
