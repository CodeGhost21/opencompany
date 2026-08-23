//! Issues #460 and #614 — end-to-end proof that a node the company's policy
//! stops does not execute, and leaves a card the operator can decide. Covers
//! both gated node kinds: `tool_call` (#460) and `http_request` (#614).
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
use crate::runtime::workflow_resume::{
    PAYLOAD_ARGS, PAYLOAD_NODE_ID, PAYLOAD_REASON, PAYLOAD_TARGET, PAYLOAD_TOOL,
    PAYLOAD_WORKFLOW_ID, WORKFLOW_APPROVE_KIND,
};

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

/// The parent for the #617 regression. Its child carries the effectful node,
/// which means only the resolver can apply the policy gate before tinyflows
/// runs it.
const SUB_WORKFLOW_PARENT: &str = r#"
id = "parent"
name = "Parent"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "sub"
kind = "sub_workflow"
name = "Child workflow"
[node.config]
workflow_id = "child"
[[edge]]
from = "start"
to = "sub"
"#;

const SUB_WORKFLOW_CHILD: &str = r#"
id = "child"
name = "Child"
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
[[edge]]
from = "start"
to = "work"
"#;

/// A child whose gate is preceded by an ungated `http_request` POST — the
/// #617 continuation hazard: approving restarts the child, and a restart
/// re-calls the POST. `on_error = "continue"` keeps the SSRF guard's loopback
/// refusal from halting the child before it reaches the gated `work` node.
const SUB_WORKFLOW_CHILD_WITH_UPSTREAM: &str = r#"
id = "child"
name = "Child"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "fetch"
kind = "http_request"
name = "Fetch"
# `on_error` is a first-class node field, not a `config` key; the validator
# rejects reserved keys inside `[node.config]`.
on_error = "continue"
[node.config]
method = "POST"
url = "http://127.0.0.1:9/notify"
[[node]]
id = "work"
kind = "tool_call"
name = "Work"
[node.config]
slug = "shell"
[node.config.args]
command = "echo ran > marker.txt"
[[edge]]
from = "start"
to = "fetch"
[[edge]]
from = "fetch"
to = "work"
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

/// The headline. The call the operator would have been asked about on the agent
/// path must now be asked about on the `tool_call` path — and must not have
/// happened.
#[tokio::test]
async fn a_policy_gated_tool_call_node_parks_instead_of_running() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, run, run_id) = run_tool_graph(dir.path(), "\"shell\"").await;

    // 1. The engine stopped at the node rather than running it.
    assert_eq!(
        run.pending_approvals,
        vec!["work".to_string()],
        "the gated tool_call node must pause the run"
    );

    // 2. The side effect did NOT happen. This is the defect itself: before this
    //    change the shell ran and nobody was asked.
    assert!(
        !marker_written(dir.path()),
        "the gated shell call must not have executed"
    );

    // 3. There is a card, and it is decidable.
    let pending = journal.pending();
    let card = pending
        .iter()
        .find(|p| p.effect.kind == WORKFLOW_APPROVE_KIND)
        .unwrap_or_else(|| panic!("the paused node should be on the Approvals page: {pending:?}"));

    assert_eq!(card.effect.payload[PAYLOAD_WORKFLOW_ID], "gated-tool");
    assert_eq!(card.effect.payload[PAYLOAD_NODE_ID], "work");
    assert_eq!(
        card.effect.run_id.as_deref(),
        Some(run_id.as_str()),
        "the card must name the run waiting on it"
    );
    // 4. Issue #460's own addition: the card says which call, and why. A bare
    //    node id is the complaint #468 makes about this surface.
    assert_eq!(
        card.effect.payload[PAYLOAD_TOOL], "shell",
        "the card must name the tool, not just the node"
    );
    assert!(
        card.effect.payload[PAYLOAD_REASON]
            .as_str()
            .is_some_and(|reason| reason.contains("shell")),
        "the card must carry the policy's own reason: {:?}",
        card.effect.payload[PAYLOAD_REASON]
    );
}

/// Issue #617. A policy gate inside a resolved child must surface at the parent
/// boundary, where it is visible and resumable, rather than silently running.
#[tokio::test]
async fn a_policy_gated_child_tool_call_parks_and_resumes_through_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("company");
    let workflows = source.join("workflows");
    std::fs::create_dir_all(&workflows).expect("create child workflow directory");
    std::fs::write(workflows.join("child.toml"), SUB_WORKFLOW_CHILD).expect("write child workflow");

    let (mut deps, journal) =
        super::gated_tool_turn_test::deps("http://127.0.0.1:1/unused".to_string(), dir.path());
    deps.workflow_source_dir = Some(source);
    let record = record("\"shell\"");
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");
    let file = parse_workflow(SUB_WORKFLOW_PARENT).expect("parent parses");

    let first = super::runner::run_workflow(
        pool.clone(),
        deps.clone(),
        &record,
        &file,
        json!({}),
        &WorkflowRunContext::new(false),
    )
    .await
    .expect("the parent pauses cleanly");
    assert_eq!(first.pending_approvals, vec!["sub::work".to_string()]);
    assert!(
        !marker_written(dir.path()),
        "the child shell call must not execute before approval"
    );

    let card = journal
        .pending()
        .into_iter()
        .find(|pending| pending.effect.kind == WORKFLOW_APPROVE_KIND)
        .expect("the child gate is parked for the operator")
        .effect;
    assert_eq!(card.payload[PAYLOAD_WORKFLOW_ID], "parent");
    assert_eq!(card.payload[PAYLOAD_NODE_ID], "sub::work");
    // Issue #617: the card must name the child's call — the same tool, reason
    // and arguments a top-level policy gate carries — not just the namespaced
    // node id the parent graph cannot resolve.
    assert_eq!(
        card.payload[PAYLOAD_TOOL], "shell",
        "the card must name the child's tool, not just the node id"
    );
    assert!(
        card.payload[PAYLOAD_REASON]
            .as_str()
            .is_some_and(|reason| reason.contains("shell")),
        "the card must carry the policy's own reason: {:?}",
        card.payload[PAYLOAD_REASON]
    );
    assert_eq!(
        card.payload[PAYLOAD_ARGS]["command"], "echo ran > marker.txt",
        "the card must carry the child call's arguments: {:?}",
        card.payload[PAYLOAD_ARGS]
    );

    let continuation =
        crate::runtime::workflow_resume::continuation_input(&card, &["sub::work".to_string()], &[])
            .expect("the namespaced child gate is a valid continuation");
    let second = super::runner::run_workflow(
        pool,
        deps,
        &record,
        &file,
        continuation,
        &WorkflowRunContext::new(false),
    )
    .await
    .expect("approval resumes the child through its parent");
    assert!(second.pending_approvals.is_empty(), "{second:?}");
    assert!(
        marker_written(dir.path()),
        "the approved child shell call must execute"
    );
}

/// Issue #617, the continuation half. A child that parks namespaced gates
/// restarts from the trigger when its gate is approved, and a restart re-runs
/// the child's ungated outward calls — whose results were never carried up
/// with the pause. The run must tell the operator, the same way the top-level
/// path does for its own unreplayable calls, so approving is a decision made
/// with that cost in view.
#[tokio::test]
async fn an_ungated_outward_call_before_a_child_gate_is_reported_unreplayable() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("company");
    let workflows = source.join("workflows");
    std::fs::create_dir_all(&workflows).expect("create child workflow directory");
    std::fs::write(
        workflows.join("child.toml"),
        SUB_WORKFLOW_CHILD_WITH_UPSTREAM,
    )
    .expect("write child workflow");

    let (mut deps, _journal) =
        super::gated_tool_turn_test::deps("http://127.0.0.1:1/unused".to_string(), dir.path());
    deps.workflow_source_dir = Some(source);
    // `shell` gated, `http_request` not — so the child runs the POST and then
    // parks at the shell node, exactly the shape the hazard describes.
    let record = record("\"shell\"");
    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record, &deps).await.expect("roster builds");
    let file = parse_workflow(SUB_WORKFLOW_PARENT).expect("parent parses");

    let first = super::runner::run_workflow(
        pool,
        deps.clone(),
        &record,
        &file,
        json!({}),
        &WorkflowRunContext::new(false),
    )
    .await
    .expect("the parent pauses cleanly");
    assert_eq!(first.pending_approvals, vec!["sub::work".to_string()]);

    let notices = first
        .notices
        .iter()
        .filter(|n| n.contains("fetch"))
        .collect::<Vec<_>>();
    assert!(
        !notices.is_empty(),
        "approving restarts the child, so its ungated http_request must be reported: {:?}",
        first.notices
    );
    assert!(
        notices
            .iter()
            .any(|n| n.contains("http_request") && n.contains("restarts")),
        "the notice must name the call and why it would repeat: {:?}",
        notices
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
async fn a_policy_gated_http_request_node_parks_instead_of_requesting() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, run) = run_http_graph(dir.path(), "\"http_request\"").await;

    let run = run.expect("a gated node pauses the run, it does not error");
    assert_eq!(
        run.pending_approvals,
        vec!["call".to_string()],
        "the gated http_request node must pause the run"
    );

    let pending = journal.pending();
    let card = pending
        .iter()
        .find(|p| p.effect.kind == WORKFLOW_APPROVE_KIND)
        .unwrap_or_else(|| panic!("the paused node should be on the Approvals page: {pending:?}"));
    assert_eq!(card.effect.payload[PAYLOAD_NODE_ID], "call");
    assert_eq!(card.effect.payload[PAYLOAD_TOOL], "http_request");
    assert_eq!(
        card.effect.payload[PAYLOAD_TARGET], "POST 127.0.0.1:9",
        "the card must name the destination the operator is deciding about"
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
async fn the_parked_card_produces_a_continuation_that_approves_the_node() {
    let dir = tempfile::tempdir().unwrap();
    let (journal, _run, _) = run_tool_graph(dir.path(), "\"shell\"").await;

    let pending = journal.pending();
    let card = pending
        .iter()
        .find(|p| p.effect.kind == WORKFLOW_APPROVE_KIND)
        .expect("a gate card");

    let input = crate::runtime::workflow_resume::continuation_input(
        &card.effect,
        &["work".to_string()],
        &[],
    )
    .expect("a policy-gated card is a well-formed continuation");
    let approvals = input["approvals"]
        .as_array()
        .expect("the continuation carries an approvals array");
    assert!(
        approvals.iter().any(|id| id == "work"),
        "the continuation must approve the node that paused: {input}"
    );
}
