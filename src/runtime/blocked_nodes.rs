//! One blocked agent node, one continuation (issue #899, Stage 1).
//!
//! # What this stashes, and why the [`ContinuationQueue`] cannot
//!
//! A policy-gated call inside an agent node's own tool loop parks a
//! tool-call-shaped effect (`agent: Some`). Approving it mints a grant, but
//! nothing re-dispatches the workflow run — the hole issue #899 closes. The fix
//! reuses the [`ContinuationQueue`](crate::runtime::continuation::ContinuationQueue)
//! to count a node's parked calls as one batch (armed at park time under a
//! [`workflow_node_turn_key`](crate::runtime::workflow_resume::workflow_node_turn_key)),
//! and releases once when the last decision lands. But to *spawn* the
//! continuation the host needs two facts that batch cannot carry — the
//! **workflow id** and the paused run's **trigger input** — for the same reason
//! [`WorkflowGateQueue`](crate::runtime::workflow_gates::WorkflowGateQueue)
//! exists beside that queue for gates: the released batch is only
//! `ApprovalResolved` events, and the parked tool-call effect carries no
//! workflow lineage of its own (it is minted by `ApprovalPolicy::effect_for`,
//! which knows nothing of the run).
//!
//! # Why armed at block-settle, not at park time
//!
//! The calls are parked mid-turn from `HarnessAgentRunner`, which does **not**
//! carry the trigger input (only the run request). The runner's block-settle
//! *does* — it is the one place with the workflow id and the trigger input
//! beside the list of blocked nodes — so the stash is populated there, exactly
//! as `park_pending_gates` populates the gate queue from the runner.
//!
//! # Durability, stated plainly
//!
//! In-memory, and — unlike [`WorkflowGateQueue`] — **not** rehydrated from the
//! journal at recovery, because the parked tool-call effect carries no
//! workflow id or trigger input to rebuild it from (widening the effect payload
//! to carry them is a larger change than Stage 1 takes on). A restart in the
//! middle of a blocked node therefore comes back with the
//! [`ContinuationQueue`] counter re-armed (from `parked_turns`) but the stash
//! empty: the batch releases and finds nothing to spawn, and the operator is
//! told to re-run the workflow. That is a strictly worse restart story than the
//! gate path's, and it is the Stage-1 boundary — the durable-card version is
//! Stage 2 territory.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::ports::types::StartedBy;

/// The three facts a blocked agent node's continuation needs, stashed at
/// block-settle and handed back on release.
#[derive(Clone, Debug)]
pub struct StashedBlock {
    /// The workflow whose run blocked, to load the graph for the re-run.
    pub workflow_id: String,
    /// The paused run's own trigger input, replayed unchanged — the minted grant
    /// is what lets the identical gated call pass on the re-run.
    pub input: Value,
    /// The blocked run's own attribution (issue #1862 prerequisite), carried
    /// into the continuation so approving the block does not silently reset it
    /// to `Operator` — see `spawn_blocked_node_continuation`.
    pub started_by: StartedBy,
}

/// Per-(run, node) continuation state for a blocked agent node: the workflow id
/// and trigger input its re-run needs (issue #899, Stage 1).
///
/// Cheap to [`Clone`] — a shared handle like every other queue in the runtime —
/// so the arming side (the workflow runner, through `DeliveryParking`) and the
/// releasing side (the runtime's `continue_turn`) see one set of stashes.
#[derive(Clone, Default)]
pub struct BlockedNodeQueue {
    inner: Arc<Mutex<HashMap<String, StashedBlock>>>,
}

impl BlockedNodeQueue {
    /// Stashes the facts a blocked node's continuation needs, keyed by its
    /// per-(run, node) turn key.
    ///
    /// **First write wins.** Every gated call one node parked shares one turn
    /// key and one trigger input, so a second arm for the same key would carry
    /// identical facts; keeping the first is simplest and cannot disagree.
    pub fn arm(&self, turn: &str, workflow_id: &str, input: &Value, started_by: &StartedBy) {
        self.inner
            .lock()
            .expect("blocked node queue poisoned")
            .entry(turn.to_string())
            .or_insert_with(|| StashedBlock {
                workflow_id: workflow_id.to_string(),
                input: input.clone(),
                started_by: started_by.clone(),
            });
    }

    /// Takes `turn`'s stash, dropping it from the queue.
    ///
    /// Called once, by whichever caller the [`ContinuationQueue`] handed the
    /// release to — that queue's counting decides who, under one lock, so this
    /// cannot be entered twice for one blocked node. `None` for a turn this
    /// queue is not holding: a card parked before this issue, or a stash lost to
    /// a restart, which the caller reports as "re-run the workflow".
    pub fn release(&self, turn: &str) -> Option<StashedBlock> {
        self.inner
            .lock()
            .expect("blocked node queue poisoned")
            .remove(turn)
    }

    /// Whether `turn` is a blocked node this queue is holding a stash for.
    pub fn is_armed(&self, turn: &str) -> bool {
        self.inner
            .lock()
            .expect("blocked node queue poisoned")
            .contains_key(turn)
    }

    /// How many blocked nodes are stashed. For tests and diagnostics.
    pub fn waiting(&self) -> usize {
        self.inner
            .lock()
            .expect("blocked node queue poisoned")
            .len()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    fn arm_then_release_hands_back_the_stashed_facts() {
        let q = BlockedNodeQueue::default();
        q.arm(
            "workflow-node:run-1:draft",
            "digest",
            &json!({ "topic": "x" }),
            &StartedBy::Agent("ceo".into()),
        );
        assert!(q.is_armed("workflow-node:run-1:draft"));

        let block = q.release("workflow-node:run-1:draft").expect("armed");
        assert_eq!(block.workflow_id, "digest");
        assert_eq!(block.input, json!({ "topic": "x" }));
        assert_eq!(
            block.started_by,
            StartedBy::Agent("ceo".into()),
            "the blocked run's own attribution must ride the stash, not reset on release"
        );
        assert_eq!(q.waiting(), 0, "release drops the stash");
        assert!(!q.is_armed("workflow-node:run-1:draft"));
    }

    #[test]
    fn release_of_an_unheld_turn_is_none() {
        let q = BlockedNodeQueue::default();
        assert!(q.release("workflow-node:run-9:ghost").is_none());
    }

    #[test]
    fn first_arm_wins_for_a_repeated_key() {
        let q = BlockedNodeQueue::default();
        q.arm(
            "workflow-node:run-1:draft",
            "digest",
            &json!({ "topic": "first" }),
            &StartedBy::Operator,
        );
        q.arm(
            "workflow-node:run-1:draft",
            "digest",
            &json!({ "topic": "second" }),
            &StartedBy::Agent("ceo".into()),
        );
        let block = q.release("workflow-node:run-1:draft").expect("armed");
        assert_eq!(block.input, json!({ "topic": "first" }));
        assert_eq!(
            block.started_by,
            StartedBy::Operator,
            "first write wins for started_by too, same as workflow_id/input"
        );
    }

    /// Two blocked nodes of two runs are independent stashes — a release of one
    /// leaves the other untouched (the scope-disjointness a cross-continuation
    /// would violate).
    #[test]
    fn two_blocked_nodes_do_not_share_a_stash() {
        let q = BlockedNodeQueue::default();
        q.arm(
            "workflow-node:run-1:draft",
            "digest",
            &json!({ "n": 1 }),
            &StartedBy::Operator,
        );
        q.arm(
            "workflow-node:run-2:draft",
            "digest",
            &json!({ "n": 2 }),
            &StartedBy::Operator,
        );

        let first = q.release("workflow-node:run-1:draft").expect("armed");
        assert_eq!(first.input, json!({ "n": 1 }));
        assert!(
            q.is_armed("workflow-node:run-2:draft"),
            "the other run stays"
        );
        assert_eq!(
            q.release("workflow-node:run-2:draft").unwrap().input,
            json!({ "n": 2 })
        );
    }
}
