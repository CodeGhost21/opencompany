//! Issue #460 — end-to-end proof that a `tool_call` node the company's policy
//! stops does not execute, and leaves a card the operator can decide.
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
    PAYLOAD_NODE_ID, PAYLOAD_REASON, PAYLOAD_TOOL, PAYLOAD_WORKFLOW_ID, WORKFLOW_APPROVE_KIND,
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

    let input = crate::runtime::workflow_resume::continuation_input(&card.effect)
        .expect("a policy-gated card is a well-formed continuation");
    let approvals = input["approvals"]
        .as_array()
        .expect("the continuation carries an approvals array");
    assert!(
        approvals.iter().any(|id| id == "work"),
        "the continuation must approve the node that paused: {input}"
    );
}
