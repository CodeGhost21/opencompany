//! Policy-HITL migration coverage for authored `tool_call` and `http_request`
//! workflow nodes. Legacy classification remains tested in `gate`; production
//! runs do not add approval gates from policy.
//!
//! # Why a unit test could not have caught this
//!
//! The same reason #395's could not, one node kind over. Every part worked
//! alone: [`ApprovalPolicy`](crate::harness::policy::ApprovalPolicy) classified
//! the call, the engine paused a `requires_approval` node, `park_pending_gates`
//! wrote a card, and `resume_from_effect` re-ran the graph. What was missing was
//! that **nothing ever asked the policy about a `tool_call` node** — the
//! invoker resolved the grant namespace and executed. Green tests, and a `shell`
//! call an operator was never asked about.
//!
//! So this drives the real path: real graph, real `run_workflow`, real
//! translation, real [`WorkflowToolInvoker`](super::caps), real exec-security,
//! real gate and real on-disk journal. Nothing is stubbed — there is no model
//! in this graph to script, which is precisely what makes a `tool_call` node
//! different from #395's agent node.
//!
//! # The company gates under `full`, deliberately
//!
//! `always_approve = ["shell"]` with `mode = "full"` rather than
//! `mode = "supervised"`, and the choice is load-bearing twice over. It is the
//! stronger claim — the call is stopped whatever the tier, so no reader can
//! attribute the stop to the classifier. And it keeps **exec-security out of the
//! way**: `supervised` sets `require_approval_for_medium_risk`, so a `shell`
//! call could be refused by the toolbelt's own layer, and a test that cannot
//! tell those two refusals apart proves nothing about this change. Under `full`
//! autonomy the shell genuinely runs — as
//! [`the_same_call_executes_when_the_policy_does_not_gate_it`] shows — so the
//! only thing that can stop it is the gate this issue adds.

use std::sync::Arc;

use serde_json::json;

use crate::company::{CompanyManifest, parse_workflow};
use crate::harness::HarnessPool;
use crate::ports::WorkflowRunContext;
use crate::ports::types::CompanyRecord;
use crate::runtime::workflow_resume::WORKFLOW_APPROVE_KIND;

/// A graph whose only working node is a `tool_call` running `shell`. The
/// `marker` file it writes is how a test tells "the call was stopped" from "the
/// call ran and the run stopped afterwards" — a distinction no assertion on the
/// run outcome alone can make.
const TOOL_GRAPH: &str = r#"
id = "gated-tool"
name = "Gated tool"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "work"
kind = "tool_call"
name = "Work"
[node.config]
slug = "shell"
[node.config.args]
command = "echo ran > marker.txt"
[[node]]
id = "done"
kind = "output"
name = "Done"
[[edge]]
from = "start"
to = "work"
[[edge]]
from = "work"
to = "done"
"#;

/// A company that grants `shell` and gates it — under `full` autonomy, for the
/// reasons in this module's docs.
fn manifest(always_approve: &str) -> CompanyManifest {
    toml::from_str(&format!(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"
always_approve = [{always_approve}]

[tools]
allow = ["shell"]

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"
"#
    ))
    .expect("manifest parses")
}

fn record(always_approve: &str) -> CompanyRecord {
    CompanyRecord {
        manifest: manifest(always_approve),
        ..super::gated_tool_turn_test::record()
    }
}

/// Runs the tool graph once and hands back the journal, the run and the
/// workspace root the `shell` node wrote into.
async fn run_tool_graph(
    dir: &std::path::Path,
    always_approve: &str,
) -> (
    Arc<crate::runtime::journal::RuntimeJournal>,
    crate::ports::WorkflowRun,
    String,
) {
    // A base URL nothing calls: this graph has no agent node, so no model is
    // reached. Passing a dead address is the assertion — if a turn were
    // dispatched, the run would fail rather than quietly succeed.
    let (deps, journal) =
        super::gated_tool_turn_test::deps("http://127.0.0.1:1/unused".to_string(), dir);
    let record = record(always_approve);
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");

    let file = parse_workflow(TOOL_GRAPH).expect("graph parses");
    let ctx = WorkflowRunContext::new(false);
    let run_id = ctx.run_id.clone();
    let run = super::runner::run_workflow(pool, deps.clone(), &record, &file, json!({}), &ctx)
        .await
        .expect("the run settles — a gated node pauses it, it does not error");
    (journal, run, run_id)
}

/// Whether the `shell` node's side effect happened anywhere under the workspace
/// root. The per-run workflow workspace is a hashed path, so this looks for the
/// marker rather than reconstructing the directory name.
fn marker_written(root: &std::path::Path) -> bool {
    fn walk(dir: &std::path::Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path) {
                    return true;
                }
            } else if path.file_name().is_some_and(|n| n == "marker.txt") {
                return true;
            }
        }
        false
    }
    walk(root)
}

/// Policy settings no longer turn an authored workflow tool call into HITL.
#[tokio::test]
async fn always_approve_does_not_gate_a_workflow_tool_call() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, run, _run_id) = run_tool_graph(dir.path(), "\"shell\"").await;

    assert!(run.pending_approvals.is_empty(), "{run:?}");
    assert!(
        marker_written(dir.path()),
        "policy HITL is disabled, so the shell call should execute"
    );
    assert!(
        journal
            .pending()
            .iter()
            .all(|p| p.effect.kind != WORKFLOW_APPROVE_KIND),
        "policy HITL must not create a workflow approval card"
    );
}

/// The other half of the claim, and the one that keeps this change from being a
/// regression: when the policy does NOT gate the call, the node runs exactly as
/// it did before. Without this, every assertion above is also satisfied by a
/// change that simply broke `tool_call` nodes.
#[tokio::test]
async fn the_same_call_executes_when_the_policy_does_not_gate_it() {
    let dir = tempfile::tempdir().unwrap();
    // Same graph, same company, same `full` tier — the ONLY difference is that
    // `shell` is no longer on the always-approve list.
    let (journal, run, _) = run_tool_graph(dir.path(), "").await;

    assert!(
        run.pending_approvals.is_empty(),
        "an ungated tool_call must not pause the run: {:?}",
        run.pending_approvals
    );
    assert!(
        marker_written(dir.path()),
        "the ungated shell call must have executed"
    );
    assert!(
        journal
            .pending()
            .iter()
            .all(|p| p.effect.kind != WORKFLOW_APPROVE_KIND),
        "an ungated run must leave no gate card"
    );
}

/// A graph whose only working node is an `http_request` node (issue #614) — a
/// different capability from `tool_call`: `GuardedHttpClient`, never
/// `ToolInvoker`.
const HTTP_GRAPH: &str = r#"
id = "gated-http"
name = "Gated http"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "call"
kind = "http_request"
name = "Call"
[node.config]
method = "POST"
url = "http://127.0.0.1:9/notify"
[[edge]]
from = "start"
to = "call"
"#;

/// Runs the `http_request` graph once, returning the journal and the run result
/// (which may be an error — see [`an_ungated_http_node_reaches_the_capability`]).
async fn run_http_graph(
    dir: &std::path::Path,
    always_approve: &str,
) -> (
    Arc<crate::runtime::journal::RuntimeJournal>,
    crate::Result<crate::ports::WorkflowRun>,
) {
    let (deps, journal) =
        super::gated_tool_turn_test::deps("http://127.0.0.1:1/unused".to_string(), dir);
    let record = record(always_approve);
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");

    let file = parse_workflow(HTTP_GRAPH).expect("graph parses");
    let ctx = WorkflowRunContext::new(false);
    let run =
        super::runner::run_workflow(pool, deps.clone(), &record, &file, json!({}), &ctx).await;
    (journal, run)
}

/// Issue #614's defect: an `http_request` node reached an external address on a
/// `supervised` company with no card. It must now stop, and the card must name
/// the destination.
#[tokio::test]
async fn always_approve_does_not_gate_an_http_request_node() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, run) = run_http_graph(dir.path(), "\"http_request\"").await;

    let error = run.expect_err("the ungated call reaches the SSRF guard");
    assert!(error.to_string().contains("http_request"), "{error}");
    assert!(
        journal
            .pending()
            .iter()
            .all(|p| p.effect.kind != WORKFLOW_APPROVE_KIND)
    );
}

/// The control, and the reason the assertions above are not hollow: with the
/// policy not gating, the node genuinely reaches the HTTP capability.
///
/// It cannot be proved by a successful request — OpenHuman's `url_guard`
/// rejects loopback unconditionally, and that guard is one of the layers #614
/// is careful NOT to claim is missing. So the proof is the *shape of the
/// failure*: the run reaches the node and fails with the guard's own refusal,
/// which only happens if the request was attempted. A change that simply broke
/// `http_request` nodes would fail here with a different error, and a change
/// that gated them under `full` would not fail at all — it would pause.
#[tokio::test]
async fn an_ungated_http_node_reaches_the_capability() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, run) = run_http_graph(dir.path(), "").await;

    let err = run.expect_err("the SSRF guard refuses loopback, so the node fails");
    let message = err.to_string();
    assert!(
        message.contains("http_request"),
        "the failure must come from the http_request node: {message}"
    );
    assert!(
        journal
            .pending()
            .iter()
            .all(|p| p.effect.kind != WORKFLOW_APPROVE_KIND),
        "an ungated run must leave no gate card"
    );
}

/// A card is only decidable if approving it can actually continue the run.
///
/// The continuation itself is #395's machinery and is tested there; what is new
/// here is the *card this change parks*, which carries two payload keys #395
/// never wrote. This pins that the resume path still reads it — the integration
/// risk a unit test of either half alone would miss.
#[tokio::test]
async fn policy_hitl_disabled_produces_no_workflow_continuation_card() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, _run, _) = run_tool_graph(dir.path(), "\"shell\"").await;

    assert!(
        journal
            .pending()
            .iter()
            .all(|p| p.effect.kind != WORKFLOW_APPROVE_KIND)
    );
}
