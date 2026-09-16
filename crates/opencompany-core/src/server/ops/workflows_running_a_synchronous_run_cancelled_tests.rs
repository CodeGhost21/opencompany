use super::*;
// The globals-unaware readers: these tests assert the company's own two
// sources, so they call the form that resolves no baseline.
use crate::company::{list_workflows_union, load_workflow_union};

/// The listed rows this company itself has, with the global baseline
/// filtered out. Every company lists the baseline graphs; these tests are
/// about what this one created, deleted, or declared.
///
/// This is an **id heuristic**, not provenance: `WorkflowSummary` carries
/// no `global` flag, so a row is classified as "the baseline's" purely by
/// id membership in `crate::globals::workflows()`. A company definition of
/// the *same* id supersedes the global one and would be wrongly excluded
/// here — none of the fixtures below give a company workflow a colliding
/// id, so the gap does not fire in this suite; see
/// `write_test::workflow_create_of_an_id_matching_a_global_wins_by_content`
/// for that case asserted directly, without this helper.
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

const DEMO: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    summary = "Kicks it off."
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
    summary = "Does the thing."
    agent = "assistant"
    [[node]]
    id = "done"
    kind = "output"
    name = "Report"
    [[edge]]
    from = "start"
    to = "worker"
    [[edge]]
    from = "worker"
    to = "done"
    label = "ok"
"#;

/// Writes `DEMO` to `<dir>/workflows/demo.toml` and returns `dir`.
fn seed_demo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("demo.toml"), DEMO).unwrap();
    dir
}

/// Issue #383: the run route driven against a runner that can be held
/// mid-run, so detach, cancellation, and surviving a dropped client are all
/// observable through the real router.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord, EventSeq};
use crate::ports::{CompanyStore, WorkflowRun, WorkflowRunContext, WorkflowRunner};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

/// A runner that parks until released, and settles as cancelled if the
/// run's stop signal fires first.
///
/// It is the real `WorkflowRunner` port, so everything above it — the
/// route, the supervisor registration, the spawned task, the journal
/// write — is production code. Only the graph walk is stubbed, which is
/// what lets these tests be about the *entry point* rather than about
/// the engine (the engine's own cancel behaviour is pinned in
/// `workflows::runner`).
struct StalledRunner {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    /// Set only if the run was allowed to finish on its own terms —
    /// which is how a test tells "the run completed" from "the run was
    /// dropped with the connection".
    completed: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl WorkflowRunner for StalledRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &crate::company::WorkflowFile,
        _input: serde_json::Value,
        ctx: &WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        // `notify_one` on both, not `notify_waiters`: a permit is
        // stored, so neither side has to be already parked. The detached
        // run answers before its task has even been polled, so a test
        // that waits on `entered` afterwards would otherwise race the
        // notification and hang.
        self.entered.notify_one();
        let released = self.release.notified();
        tokio::select! {
            () = released => {}
            () = ctx.cancel.cancelled() => {
                return Ok(WorkflowRun {
                    output: serde_json::Value::Null,
                    pending_approvals: Vec::new(),
                    deliveries: Vec::new(),
                    cancelled: true,
                    nodes: Vec::new(),
                    notices: Vec::new(),
                    board: Vec::new(),
                    blocked_nodes: Vec::new(),
                    approvals: Vec::new(),
                });
            }
        }
        self.completed.store(true, Ordering::SeqCst);
        Ok(WorkflowRun {
            output: serde_json::json!({ "run": {}, "nodes": {} }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: false,
            nodes: Vec::new(),
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        })
    }
}

struct Stalled {
    app: axum::Router,
    runtime: Arc<crate::company::runtime::CompanyRuntime>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    completed: Arc<AtomicBool>,
}

const GRAPH: &str = r#"
id = "demo"
name = "Demo"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "done"
kind = "output"
name = "Report"
[[edge]]
from = "start"
to = "done"
label = "ok"
"#;

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-workflows-running-")
        .tempdir()
        .expect("tempdir")
}

/// A hosted company with one overlay workflow and a runner that stalls.
async fn stalled_company(home: &std::path::Path) -> Stalled {
    let manifest: CompanyManifest =
        toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
    let id = CompanyId::new("acme");
    FsCompanyStore::new(home.to_path_buf())
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                id: "demo".to_string(),
                toml: GRAPH.to_string(),
            }],
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            lifecycle: "running".to_string(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();

    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let completed = Arc::new(AtomicBool::new(false));
    let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    runtime.set_workflow_runner(Arc::new(StalledRunner {
        entered: entered.clone(),
        release: release.clone(),
        completed: completed.clone(),
    }));
    let runtime = Arc::new(runtime);

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id.clone(), runtime.clone());
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    Stalled {
        app: router(state),
        runtime,
        entered,
        release,
        completed,
    }
}

fn run_request(body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/company/workflows/demo/run")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn cancel_request(run_id: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/company/workflows/runs/{run_id}/cancel"))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap()
}

fn get_workflow_request() -> Request<Body> {
    Request::builder()
        .uri("/api/v1/company/workflows/demo")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap()
}

fn delete_workflow_request(version: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/v1/company/workflows/demo?expectedVersion={version}"
        ))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

/// Every event the company journaled, oldest first.
async fn journal(
    runtime: &Arc<crate::company::runtime::CompanyRuntime>,
) -> Vec<CompanyEvent> {
    runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), usize::MAX)
        .await
        .expect("read")
        .into_iter()
        .map(|s| s.event)
        .collect()
}

/// Waits (bounded) for a `WorkflowRunFinished` to appear.
async fn await_finished(
    runtime: &Arc<crate::company::runtime::CompanyRuntime>,
) -> Option<CompanyEvent> {
    for _ in 0..200 {
        if let Some(event) = journal(runtime)
            .await
            .into_iter()
            .find(|e| matches!(e, CompanyEvent::WorkflowRunFinished { .. }))
        {
            return Some(event);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    None
}

/// **A synchronous run can be cancelled mid-request, and its response
/// has to say so.**
///
/// Easy to miss, because "detached" and "cancellable" sound like the
/// same feature: the run id is registered the moment the task is
/// spawned, and the console learns it from the `workflow_run_started`
/// frame — so the cancel route is reachable well before the synchronous
/// response is written. The runner then resolves to a cancelled run
/// whose `output` is `null` with no approvals and no deliveries, which
/// is byte-identical to a run that legitimately produced nothing. This
/// caller was the last reader in the PR still left guessing.
#[tokio::test]
async fn a_synchronous_run_cancelled_mid_request_says_so_in_its_response() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let mut running = Box::pin(
        c.app
            .clone()
            .oneshot(run_request(serde_json::json!({ "input": {} }))),
    );
    // Wait until the runner is actually parked, then find the run the
    // way the console does — by its id — and stop it.
    tokio::select! {
        _ = &mut running => panic!("the run answered before the runner was under way"),
        () = c.entered.notified() => {}
    }
    // Off the supervisor rather than the journal: this stub is the
    // `WorkflowRunner` port, so it never reaches the harness runner that
    // writes `WorkflowRunStarted`. The supervisor is the registration
    // the cancel route itself consults, which makes it the more direct
    // assertion anyway — the id is addressable while the request is open.
    let live = c.runtime.run_supervisor().live();
    assert_eq!(live.len(), 1, "the open synchronous run is registered");
    let (run_id, workflow_id) = live.into_iter().next().unwrap();
    assert_eq!(workflow_id, "demo");
    let response = c
        .app
        .clone()
        .oneshot(cancel_request(&run_id))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a synchronous run is cancellable while its request is open"
    );

    // The still-open request now answers, and must not read as a clean
    // empty success.
    let response = running.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["cancelled"], true, "{body}");
    assert_eq!(body["runId"], run_id.as_str(), "{body}");
    assert!(
        !c.completed.load(Ordering::SeqCst),
        "the run must not have completed its work"
    );
}

/// …and the flag is omitted entirely on a run nobody stopped, so an
/// existing caller's body is byte-unchanged.
#[tokio::test]
async fn an_uncancelled_synchronous_response_omits_the_flag() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;
    c.release.notify_one();

    let response = c
        .app
        .clone()
        .oneshot(run_request(serde_json::json!({ "input": {} })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert!(
        body.get("cancelled").is_none(),
        "a run nobody stopped carries no flag at all: {body}"
    );
}

/// The history fold reports it, so the console can render a stopped run
/// as stopped rather than as a clean success.
#[tokio::test]
async fn a_cancelled_run_reads_back_as_cancelled_and_not_running() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let response = c
        .app
        .clone()
        .oneshot(run_request(serde_json::json!({ "detach": true })))
        .await
        .unwrap();
    let run_id = json_body(response).await["runId"]
        .as_str()
        .unwrap()
        .to_string();
    c.entered.notified().await;
    c.app
        .clone()
        .oneshot(cancel_request(&run_id))
        .await
        .unwrap();
    await_finished(&c.runtime).await.expect("settles");

    let response = c
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/company/workflows/runs")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let rows = json_body(response).await;
    let row = &rows["runs"].as_array().expect("array")[0];
    assert_eq!(row["runId"], run_id.as_str(), "{rows}");
    assert_eq!(row["cancelled"], true, "{rows}");
    assert!(
        row.get("running").is_none(),
        "a settled run is not running: {rows}"
    );
    assert!(
        row.get("error").is_none(),
        "a stopped run carries no error: {rows}"
    );
}

/// Unknown and already-settled are the same `404`: there is nothing to
/// stop. Keeping a tombstone to tell them apart would mean choosing an
/// expiry for it, and the run history already says what became of a
/// settled run.
#[tokio::test]
async fn cancelling_an_unknown_or_settled_run_is_not_found() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let response = c
        .app
        .clone()
        .oneshot(cancel_request("never-existed"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Now run one to completion and try again.
    let response = c
        .app
        .clone()
        .oneshot(run_request(serde_json::json!({ "detach": true })))
        .await
        .unwrap();
    let run_id = json_body(response).await["runId"]
        .as_str()
        .unwrap()
        .to_string();
    c.entered.notified().await;
    c.release.notify_one();
    await_finished(&c.runtime).await.expect("settles");

    let response = c
        .app
        .clone()
        .oneshot(cancel_request(&run_id))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "a settled run is no longer cancellable"
    );
}

/// The cancel route is behind the same `ScopedCompany` guard as every
/// other route in this module — an unauthenticated caller cannot stop a
/// company's work.
#[tokio::test]
async fn cancelling_without_a_session_is_rejected() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let response = c
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/workflows/runs/whatever/cancel")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "an unauthenticated cancel must not be accepted"
    );
    assert!(
        response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN,
        "expected an auth rejection, got {}",
        response.status()
    );
}

/// The cancel path is a static prefix under `/workflows`, and `runs` is
/// a syntactically valid workflow id — so this pins that it is not
/// shadowed by the dynamic `/workflows/{wid}` routes, the same guarantee
/// `run_history_is_not_shadowed_by_the_graph_read` makes for the GET.
#[tokio::test]
async fn the_cancel_route_is_not_shadowed_by_the_dynamic_workflow_routes() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let response = c
        .app
        .clone()
        .oneshot(cancel_request("anything"))
        .await
        .unwrap();
    // 404 from the *cancel handler* (nothing to stop), not a 405 or a
    // route miss — reaching the handler at all is the assertion.
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert!(
        body.to_string().contains("workflow run"),
        "the 404 should come from the cancel handler: {body}"
    );
}

// ── Issue #542: dry run through the real route ──────────────────────

/// A runner that completes immediately, returning one node row — enough
/// to prove the route maps `WorkflowRun.nodes` onto the response and
/// echoes the request's `dry_run` as the discriminator.
struct EchoRunner;

#[async_trait::async_trait]
impl WorkflowRunner for EchoRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &crate::company::WorkflowFile,
        _input: serde_json::Value,
        _ctx: &WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        Ok(WorkflowRun {
            output: serde_json::json!({ "run": {}, "nodes": {} }),
            pending_approvals: Vec::new(),
            deliveries: Vec::new(),
            cancelled: false,
            nodes: vec![crate::ports::WorkflowRunNodeRow {
                node_id: "done".to_string(),
                status: crate::ports::types::WorkflowNodeStatus::Ok,
                elapsed_ms: 3,
                diagnostics: Vec::new(),
            }],
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        })
    }
}

/// A runner that settles cleanly and hands back one delivery row —
/// every node `ok`, no error, nothing cancelled, and a report that did
/// not go out. The exact shape issue #981 caught reading green.
struct DroppedReportRunner;

/// A runner whose only delivery row is a **dry run**'s (issue #542): the
/// report was routed as far as its destination and deliberately not
/// dispatched. The row shape a real `deliver_outputs_dry` writes.
struct DryRunRunner;

#[async_trait::async_trait]
impl WorkflowRunner for DryRunRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &crate::company::WorkflowFile,
        _input: serde_json::Value,
        _ctx: &WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        Ok(WorkflowRun {
            output: serde_json::json!({ "run": {}, "nodes": {} }),
            pending_approvals: Vec::new(),
            deliveries: vec![crate::ports::DeliveryReport {
                node: "done".to_string(),
                kind: "channel".to_string(),
                target: Some("engineering".to_string()),
                status: crate::ports::DeliveryStatus::Skipped,
                detail: "this was a test run — nothing was sent".to_string(),
                reason: crate::ports::DeliveryReason::DryRun,
            }],
            cancelled: false,
            nodes: vec![crate::ports::WorkflowRunNodeRow {
                node_id: "done".to_string(),
                status: crate::ports::types::WorkflowNodeStatus::Ok,
                elapsed_ms: 3,
                diagnostics: Vec::new(),
            }],
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl WorkflowRunner for DroppedReportRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &crate::company::WorkflowFile,
        _input: serde_json::Value,
        _ctx: &WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        Ok(WorkflowRun {
            output: serde_json::json!({ "run": {}, "nodes": {} }),
            pending_approvals: Vec::new(),
            deliveries: vec![crate::ports::DeliveryReport {
                node: "done".to_string(),
                kind: "channel".to_string(),
                target: Some("operator".to_string()),
                status: crate::ports::DeliveryStatus::Failed,
                detail: "`operator` is not an automation delivery channel".to_string(),
                reason: crate::ports::DeliveryReason::ChannelNotWired,
            }],
            cancelled: false,
            nodes: vec![crate::ports::WorkflowRunNodeRow {
                node_id: "done".to_string(),
                status: crate::ports::types::WorkflowNodeStatus::Ok,
                elapsed_ms: 3,
                diagnostics: Vec::new(),
            }],
            notices: Vec::new(),
            board: Vec::new(),
            blocked_nodes: Vec::new(),
            approvals: Vec::new(),
        })
    }
}

/// A hosted company whose runner echoes immediately.
async fn echo_company(home: &std::path::Path) -> axum::Router {
    company_with_runner(home, Arc::new(EchoRunner)).await
}

/// A hosted company with one overlay graph and the given runner behind
/// the port, so the route, the supervisor and the journal write are all
/// production code and only the graph walk is stubbed.
async fn company_with_runner(
    home: &std::path::Path,
    runner: Arc<dyn WorkflowRunner>,
) -> axum::Router {
    let manifest: CompanyManifest =
        toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
    let id = CompanyId::new("acme");
    FsCompanyStore::new(home.to_path_buf())
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: vec![crate::ports::types::OverlayWorkflow {
                id: "demo".to_string(),
                toml: GRAPH.to_string(),
            }],
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            lifecycle: "running".to_string(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let mut runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    runtime.set_workflow_runner(runner);
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id.clone(), Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    router(state)
}

