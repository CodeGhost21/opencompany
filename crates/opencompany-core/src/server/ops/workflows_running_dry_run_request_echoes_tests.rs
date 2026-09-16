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
async fn journal(runtime: &Arc<crate::company::runtime::CompanyRuntime>) -> Vec<CompanyEvent> {
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

struct EchoRunner;

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

struct DroppedReportRunner;

struct DryRunRunner;

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

async fn echo_company(home: &std::path::Path) -> axum::Router {
    company_with_runner(home, Arc::new(EchoRunner)).await
}

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

/// T8 — `{"dry_run":true}` answers 200 carrying `dryRun:true` and the
/// per-node `nodes`; a plain body carries neither `dryRun` (a real run's
/// shape an old host would produce) — the presence discriminator the
/// console reads instead of trusting what it asked for.
#[tokio::test]
async fn dry_run_request_echoes_the_marker_and_nodes_a_plain_body_omits_it() {
    let home_dir = home();
    let app = echo_company(home_dir.path()).await;

    let dry = app
        .clone()
        .oneshot(run_request(serde_json::json!({ "dry_run": true })))
        .await
        .unwrap();
    assert_eq!(dry.status(), StatusCode::OK);
    let body = json_body(dry).await;
    assert_eq!(body["dryRun"], serde_json::json!(true), "{body}");
    assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
    assert_eq!(body["nodes"][0]["status"], "ok", "{body}");

    let plain = app
        .oneshot(run_request(serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(plain.status(), StatusCode::OK);
    let body = json_body(plain).await;
    assert!(
        body.get("dryRun").is_none(),
        "a real run must carry no dryRun key: {body}"
    );
    // The node trail rides every settled run, dry or not.
    assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
}

// ── Issue #981 (part 2): the run's own verdict ───────────────────────

/// **The defect, at the HTTP boundary.** A run whose report was refused
/// answers `200` with every node `ok` and no error — and before this
/// there was nothing on the body that said otherwise, so a client
/// folding `nodes[].status` (the QA harness among them) scored it green.
#[tokio::test]
async fn a_run_whose_report_was_dropped_does_not_answer_as_a_clean_run() {
    let home_dir = home();
    let app = company_with_runner(home_dir.path(), Arc::new(DroppedReportRunner)).await;

    let response = app
        .oneshot(run_request(serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["verdict"], "undelivered", "{body}");

    // …and the three facts the verdict must NOT have disturbed. A
    // delivery failure is not a broken graph: the node ran, so its
    // status stays `ok`, no `error` appears, and the run is not
    // cancelled. Flipping any of them would send the copilot's
    // fix-from-run at a graph that was fine.
    assert_eq!(body["nodes"][0]["nodeId"], "done", "{body}");
    assert_eq!(body["nodes"][0]["status"], "ok", "{body}");
    assert!(body.get("error").is_none(), "{body}");
    assert!(body.get("cancelled").is_none(), "{body}");
    // The row is still where the *reason* lives; the verdict is the
    // reading.
    assert_eq!(
        body["deliveries"][0]["reason"], "channel-not-wired",
        "{body}"
    );
}

/// Issue #981, the second half: a **test run** is not a run that lost
/// its report.
///
/// `deliver_outputs_dry` writes one `skipped`/`dry-run` row per routed
/// `output` node, so before this every single test run of a graph with
/// a destination answered `undelivered` — the console badged the safest
/// thing an operator can do as a failure, every time. The rows stay on
/// the body: they are what say *where* the report would have gone.
///
/// Asked for as a **real dry run** (`dry_run: true`) and the `dryRun`
/// discriminator asserted alongside the verdict, so this cannot pass on
/// a run that was not one — a stub runner returning a `dry-run` row is
/// only half the claim.
#[tokio::test]
async fn a_test_run_is_not_a_run_that_lost_its_report() {
    let home_dir = home();
    let app = company_with_runner(home_dir.path(), Arc::new(DryRunRunner)).await;

    let response = app
        .oneshot(run_request(serde_json::json!({ "dry_run": true })))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;

    assert_eq!(body["verdict"], "ok", "{body}");
    assert_eq!(body["dryRun"], serde_json::json!(true), "{body}");
    assert_eq!(body["deliveries"][0]["reason"], "dry-run", "{body}");
    assert_eq!(body["deliveries"][0]["status"], "skipped", "{body}");
}

/// The other direction, which is the one that must not regress: a run
/// that delivered everything still reads `ok`.
#[tokio::test]
async fn a_run_that_delivered_fine_still_answers_ok() {
    let home_dir = home();
    let app = echo_company(home_dir.path()).await;

    let response = app
        .oneshot(run_request(serde_json::json!({})))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["verdict"], "ok", "{body}");
}
