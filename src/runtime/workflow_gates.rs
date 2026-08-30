//! One workflow run, one continuation (issue #978).
//!
//! # The amplification this closes
//!
//! A run that fans out to N gated nodes parks N cards. Before this, approving
//! one of them re-dispatched the **whole run** on the spot — the spawn lived in
//! `perform_effect`, which fires once per approved effect — and the replay
//! carried an `approvals` array naming only that one node, so the other N-1
//! paused again and parked again. Three approvals produced three runs and six
//! new cards; the reported staging tenant went 3 → 6 → 12 → 24 and accumulated
//! 77 runs of one disabled workflow.
//!
//! [`ContinuationQueue`](crate::runtime::continuation::ContinuationQueue) already
//! solves the shape of this problem for an agent turn: count the outstanding
//! decisions, release once when the last lands. Issue #978 makes a workflow run
//! a turn in exactly that sense, keyed by
//! [`workflow_turn_key`](crate::runtime::workflow_resume::workflow_turn_key).
//! This module is the half that queue cannot supply.
//!
//! # Why a second structure, and not a field on the batch
//!
//! `ContinuationQueue::decide` hands back `ApprovalResolved` events: an id, a
//! verdict, an actor. To re-dispatch a run the host needs two things that are
//! not in there — **which graph node** each id was gating, and the paused run's
//! **trigger input** with its delivery / outward-call ledgers.
//!
//! Neither is recoverable from the journal at release time, and that is not an
//! oversight to route around. `RuntimeJournal` drops the parked entry on resolve
//! (`parked.remove`), and the record it *does* retain past resolution —
//! `approval_effects` — is deliberately **payload-scrubbed** (issue #351), so a
//! resolved gate's id, input and ledgers are all gone by construction. Widening
//! that scrub to keep them would reopen a privacy rule for the benefit of one
//! caller.
//!
//! So the facts are stashed here instead, at **park** time, from the one place
//! gates are parked.
//!
//! # Why park time, and not approve time
//!
//! Because **deny** never reaches the approve path. A denied effect resolves to
//! [`ResolveOutcome`](crate::policy::ResolveOutcome)`::Denied` and never touches
//! `perform_effect`, so a stash populated when an approval is *performed* would
//! hold no node id for a refusal — and the continuation, not knowing the node
//! was refused, would replay into it, pause, and park a fresh card. An approval
//! round that cleared three and created one is the same defect in miniature.
//!
//! Arming at park time covers approve, deny and TTL expiry with one mechanism,
//! which is what makes the denial ledger
//! ([`PAYLOAD_DENIED`](crate::runtime::workflow_resume::PAYLOAD_DENIED))
//! expressible at all.
//!
//! # One representative effect per run, not one per gate
//!
//! Every gate of a single run is built by one `park_pending_gates` loop from one
//! `trigger_input`, one run-level `deliveries` slice and one run-level
//! `performed` slice, and `gate_effect` derives the two ledgers purely from
//! those. So `input` / `delivered` / `performed` are byte-identical across
//! siblings **by construction** — including the case where an earlier node
//! already delivered before the fan-out, because that delivery is in the
//! run-level list every sibling is handed, not in a per-branch subset. Only
//! `node_id`, the call description and the per-gate upstream `content` vary, and
//! a continuation needs none of those. Keeping one effect per run rather than
//! one per gate is what stops this from holding N copies of a trigger input.
//!
//! # Durability, stated plainly
//!
//! In-memory, on exactly [`ContinuationQueue`]'s terms, and rehydrated at
//! recovery from the journal's still-parked gates
//! ([`rearm`](WorkflowGateQueue::rearm)) the way that queue is rehydrated from
//! `parked_turns`. The limit is inherited rather than added: a restart in the
//! middle of a partly-decided run comes back knowing only the gates still
//! parked, so a batch released after it carries the last decision and not the
//! ones banked before it. Those un-carried siblings re-park. That is pre-#469
//! behaviour for agent turns; #978 is where a workflow run starts feeling it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ports::types::{ApprovalId, Effect, Verdict};
use crate::runtime::workflow_resume::gate_node_id;

/// What one workflow run's gate batch has settled into, handed to the caller
/// that released it.
#[derive(Clone, Debug)]
pub struct ReleasedGates {
    /// Any one of the run's parked gate effects — they agree on everything a
    /// continuation reads. See the module docs for why one is enough.
    pub effect: Effect,
    /// The gate nodes the operator approved, in decision order.
    pub approved: Vec<String>,
    /// The gate nodes the operator refused, or that expired to a default-deny,
    /// in decision order. A continuation neither runs these nor re-asks.
    pub denied: Vec<String>,
}

/// One run's parked gates, and the verdicts landed on them so far.
#[derive(Clone, Debug)]
struct Batch {
    effect: Effect,
    /// Gate nodes still awaiting a verdict, by the approval deciding them.
    undecided: HashMap<ApprovalId, String>,
    approved: Vec<String>,
    denied: Vec<String>,
}

/// Per-run gate state: which node each parked approval is gating, and what the
/// run's trigger input was (issue #978).
///
/// Cheap to [`Clone`] — a shared handle like every other queue in the runtime —
/// so the parking side (the workflow runner, through `DeliveryParking`) and the
/// resolving side (the runtime) see one set of batches.
#[derive(Clone, Default)]
pub struct WorkflowGateQueue {
    inner: Arc<Mutex<HashMap<String, Batch>>>,
}

impl WorkflowGateQueue {
    /// Records that `turn` just parked one more gate, `id` deciding it.
    ///
    /// Called from the one place gates are parked and on its **successful-park
    /// path only**, immediately beside
    /// [`ContinuationQueue::arm`](crate::runtime::continuation::ContinuationQueue::arm),
    /// so the two cannot disagree about how many decisions a run is blocked on.
    /// Arming on a dedupe-skip or a failed park would leave the run waiting for
    /// a decision no card can ever deliver.
    ///
    /// A non-gate effect is ignored rather than stored: `gate_node_id` checks the
    /// kind, so nothing but a `workflow.approve` card can enter a batch.
    ///
    /// The first gate through sets the batch's representative effect; later ones
    /// do not replace it, because they agree with it on everything read back.
    pub fn arm(&self, turn: &str, id: &ApprovalId, effect: &Effect) {
        let Some(node) = gate_node_id(effect) else {
            return;
        };
        let node = node.to_string();
        let mut guard = self.inner.lock().expect("workflow gate queue poisoned");
        guard
            .entry(turn.to_string())
            .or_insert_with(|| Batch {
                effect: effect.clone(),
                undecided: HashMap::new(),
                approved: Vec::new(),
                denied: Vec::new(),
            })
            .undecided
            .insert(id.clone(), node);
    }

    /// Banks one verdict on `turn`.
    ///
    /// Recorded here rather than derived from the released
    /// [`ContinuationQueue`](crate::runtime::continuation::ContinuationQueue)
    /// batch because that batch cannot describe every decision: a TTL expiry
    /// contributes no event at all (the sweep appends its own), so a run whose
    /// last gate timed out would otherwise release with the expired node in
    /// neither ledger — and the continuation would replay into it and park a
    /// fresh card.
    ///
    /// A decision on a turn this queue is not tracking, or on an id it never
    /// armed, is a no-op: the caller is deciding something that is not a
    /// run-scoped workflow gate.
    pub fn decide(&self, turn: &str, id: &ApprovalId, verdict: Verdict) {
        let mut guard = self.inner.lock().expect("workflow gate queue poisoned");
        let Some(batch) = guard.get_mut(turn) else {
            return;
        };
        let Some(node) = batch.undecided.remove(id) else {
            return;
        };
        let ledger = match verdict {
            Verdict::Approve => &mut batch.approved,
            Verdict::Deny => &mut batch.denied,
        };
        if !ledger.contains(&node) {
            ledger.push(node);
        }
    }

    /// Takes `turn`'s whole batch, dropping it from the queue.
    ///
    /// Called once, by whichever caller the continuation queue handed the
    /// release to — that queue's counting decides who, under one lock, so this
    /// cannot be entered twice for one run.
    pub fn release(&self, turn: &str) -> Option<ReleasedGates> {
        self.inner
            .lock()
            .expect("workflow gate queue poisoned")
            .remove(turn)
            .map(|batch| ReleasedGates {
                effect: batch.effect,
                approved: batch.approved,
                denied: batch.denied,
            })
    }

    /// Whether `turn` is a run-scoped batch this queue is holding.
    ///
    /// What the approve path forks on: a gate whose run is armed defers to the
    /// batch release, and one that is not — a card parked by a build from before
    /// this issue, whose journal line carries no turn key — re-dispatches
    /// immediately, exactly as it always did.
    pub fn is_armed(&self, turn: &str) -> bool {
        self.inner
            .lock()
            .expect("workflow gate queue poisoned")
            .contains_key(turn)
    }

    /// How many of `turn`'s gates are still undecided. `0` for a turn this queue
    /// is not tracking.
    pub fn undecided(&self, turn: &str) -> usize {
        self.inner
            .lock()
            .expect("workflow gate queue poisoned")
            .get(turn)
            .map(|batch| batch.undecided.len())
            .unwrap_or(0)
    }

    /// Rebuilds the batches from every gate the journal still has parked, one
    /// entry per approval.
    ///
    /// **Idempotent**, on
    /// [`ContinuationQueue::rearm`](crate::runtime::continuation::ContinuationQueue::rearm)'s
    /// terms: it *replaces* each turn it sees rather than adding to it, so
    /// replaying twice (boot loads the journal, and `recover` can be driven
    /// again) leaves a run blocked on the gates it is actually blocked on.
    ///
    /// Verdicts banked by **this** process are dropped along with the rest of
    /// the turn, which is the honest reading: a rehydrate is only reached from a
    /// journal replay, and the journal records that an approval resolved without
    /// recording what it was gating. See the module docs on durability.
    pub fn rearm<'e>(&self, gates: impl IntoIterator<Item = (String, ApprovalId, &'e Effect)>) {
        let mut rebuilt: HashMap<String, Batch> = HashMap::new();
        for (turn, id, effect) in gates {
            let Some(node) = gate_node_id(effect) else {
                continue;
            };
            rebuilt
                .entry(turn)
                .or_insert_with(|| Batch {
                    effect: effect.clone(),
                    undecided: HashMap::new(),
                    approved: Vec::new(),
                    denied: Vec::new(),
                })
                .undecided
                .insert(id, node.to_string());
        }
        let mut guard = self.inner.lock().expect("workflow gate queue poisoned");
        for (turn, batch) in rebuilt {
            guard.insert(turn, batch);
        }
    }
}
