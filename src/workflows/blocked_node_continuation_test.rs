//! Issue #899 (Stage 1) — approving a call gated **inside an agent node's own
//! tool loop** auto-continues the blocked run.
//!
//! # The hole this proves closed, and why the suite was blind to it
//!
//! A workflow agent node runs openhuman's tool loop. A policy-gated call mid-turn
//! is refused inside that loop, `park_gated_calls` opens a decidable card, and
//! the node returns `Err` so `WorkflowRun` reclassifies it Blocked (#881). Every
//! part worked — and approving the card still did **nothing to the run**. The
//! parked effect is tool-call-shaped (`agent: Some`), so approving it minted a
//! grant and the resolution ran a brain cycle; nothing re-dispatched the
//! settled run. The operator's only recourse was to re-run the whole workflow by
//! hand. `blocked_node_test` proves the block; `gated_tool_turn_test` proves the
//! card; neither drives the **resolve → continue** seam, because before this
//! there was nothing on the other side of it.
//!
//! # What this drives
//!
//! The real path, end to end: a real graph, the real engine through
//! [`HarnessWorkflowRunner`](super::runner::HarnessWorkflowRunner), the real
//! `ApprovalPolicy` gate, a real on-disk journal, and a real [`CompanyRuntime`]
//! resolving through `resolve_approval` — the same call the console's Approvals
//! route makes. The one thing scripted is the model's choices, over a loopback
//! OpenAI-compatible endpoint, exactly as its two ancestors script it. The grant
//! set is **shared** between the runtime that mints and the workflow policy that
//! redeems (the production wiring), so a continuation whose agent re-issues the
//! identical call passes rather than re-parking — the Stage-1 loop-safety.
//!
//! # Reading the count
//!
//! One number carries the issue, asserted as an **equality**: `runs_started`.
//! The cold run is 1. On `main` an approval adds nothing (the resolution is a
//! brain cycle, not a re-dispatch), so it stays 1 — this is the red. With the
//! fix an approval adds exactly one continuation, so it is 2; a refusal still
//! adds none; and a node blocked on two calls adds one, only after the second
//! decision. Two blocked runs never continue each other.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::company::{CompanyManifest, parse_workflow};
use crate::harness::HarnessPool;
use crate::harness::policy::ApprovalRequestQueue;
use crate::ports::types::{
    Actor, ActorKind, ApprovalId, CompanyId, CompanyRecord, CompanySummary, LedgerEntry, Verdict,
};
use crate::ports::{CompanyStore, WorkflowRun, WorkflowRunContext, WorkflowRunner};
use crate::runtime::RuntimeBuilder;
use crate::runtime::grants::GrantScope;
use crate::runtime::workflow_resume::workflow_node_turn_key;
use crate::store::FsCompanyStore;

use super::gated_tool_turn_test::{Turn, deps, spawn_script_recording};

/// `start -> work(agent, gates a shell call) -> done`. The minimal shape of the
/// reported case: one agent node whose turn is stopped by a policy gate.
const SOLO_TOML: &str = r#"
id = "solo"
name = "Solo"
[[node]]
id = "start"
kind = "trigger"
name = "Start"
[[node]]
id = "work"
kind = "agent"
name = "Work"
summary = "Do the gated thing."
agent = "ceo"
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

/// The agent node id every card in these tests is keyed under.
const NODE: &str = "work";

/// A company that gates `shell` under **full** autonomy — the strongest tier, so
/// the stop is the policy's explicit gate rather than a supervised classifier.
fn manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"
always_approve = ["shell"]

[tools]
allow = ["shell"]

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"
"#,
    )
    .expect("manifest parses")
}

fn record() -> CompanyRecord {
    CompanyRecord {
        manifest: manifest(),
        ..super::gated_tool_turn_test::record()
    }
}

fn operator() -> Actor {
    Actor {
        kind: ActorKind::Operator,
        id: "owner".into(),
    }
}

/// What the host actually asked the engine to run.
#[derive(Clone, Debug)]
struct StartedRun {
    #[allow(dead_code)]
    input: Value,
}

/// The real runner, wrapped so the test can count dispatches — the reported
/// symptom (approving started zero continuations) is a count the run history
/// cannot give back.
struct RecordingRunner {
    inner: super::runner::HarnessWorkflowRunner,
    started: Mutex<Vec<StartedRun>>,
}

impl RecordingRunner {
    fn started(&self) -> usize {
        self.started.lock().expect("recording runner").len()
    }
}

#[async_trait]
impl WorkflowRunner for RecordingRunner {
    async fn run(
        &self,
        company: &CompanyId,
        workflow: &crate::company::WorkflowFile,
        input: Value,
        ctx: &WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        self.started
            .lock()
            .expect("recording runner")
            .push(StartedRun {
                input: input.clone(),
            });
        self.inner.run(company, workflow, input, ctx).await
    }
}

/// A home whose `workflows/` directory holds the graph, so a continuation's
/// loader finds it by id exactly as the console run route would.
fn seed_home() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("opencompany-blocked-cont-")
        .tempdir()
        .expect("tempdir");
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).expect("workflows dir");
    std::fs::write(workflows.join("solo.toml"), SOLO_TOML).expect("seed graph");
    dir
}

/// A runtime wired to the **real** workflow runner, parking into the runtime's
/// own gate/journal/continuation/blocked-node queues, with the grant set
/// **shared** so a continuation redeems what the resolve minted.
///
/// The model is served by `turns` on loopback. Everything the resolve path
/// touches is the runtime's own handle, or every assertion here would be vacuous.
async fn runtime(
    home: &std::path::Path,
    turns: Vec<Turn>,
) -> (
    Arc<crate::company::runtime::CompanyRuntime>,
    Arc<RecordingRunner>,
) {
    let mut rt = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_seed_dir(home.to_path_buf())
        .build()
        .await
        .expect("runtime builds");

    let (base_url, _script) = spawn_script_recording(turns).await;
    let (mut deps, _unused) = deps(base_url, home);
    // Production wiring: the workflow policy redeems from the SAME grant set the
    // runtime mints into, so an approved continuation's identical call passes.
    deps.approval_requests = ApprovalRequestQueue::with_grants(rt.grants.clone());
    let delivery = deps.delivery.as_mut().expect("the fixture wires delivery");
    delivery.parking = Some(super::delivery::DeliveryParking {
        approvals: rt.approvals.clone(),
        journal: rt.journal().clone(),
        continuations: rt.continuations.clone(),
        gates: rt.workflow_gates().clone(),
        blocked_nodes: rt.blocked_nodes().clone(),
    });

    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record(), &deps).await.expect("roster builds");
    // Single-harness fixture: the default lane over the pool is the turn
    // (mirrors `run_workflow`'s single-pool entrypoint).
    let turn = Arc::new(crate::harness::built_in::run_turn::HarnessRunTurn::new(
        pool,
        Arc::new(deps.clone()),
    ));
    let runner = Arc::new(RecordingRunner {
        inner: super::runner::HarnessWorkflowRunner::new(turn, deps, record()),
        started: Mutex::new(Vec::new()),
    });
    rt.set_workflow_runner(runner.clone());
    (Arc::new(rt), runner)
}

/// A [`CompanyStore`] that suspends its `load` calls on a gate the test
/// controls, wrapping the real filesystem store so every other call — and
/// every `load` before the gate is armed — behaves exactly as it always did.
///
/// `spawn_blocked_node_continuation` awaits `runtime.store().load(...)`
/// before its synchronous run-admission step, and that is the one genuine
/// suspension point between "`resume_blocked_agent_node` has committed to
/// continuing" and the run actually being registered — everything after it
/// (the graph run itself) is handed to a task `WorkflowSpawn::spawn` does not
/// await, so nothing past this point is observably synchronous with the
/// caller. Holding this one open is therefore the only way to inspect state
/// from inside that window at all, and `reached` makes stopping there
/// deterministic rather than a hope that the test task and the load race the
/// way the test wants.
struct GatedStore {
    inner: FsCompanyStore,
    armed: AtomicBool,
    reached: Notify,
    release: Notify,
}

impl GatedStore {
    fn new(home: &std::path::Path) -> Self {
        Self {
            inner: FsCompanyStore::new(home.to_path_buf()),
            armed: AtomicBool::new(false),
            reached: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait]
impl CompanyStore for GatedStore {
    async fn load(&self, id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        if self.armed.load(Ordering::SeqCst) {
            self.reached.notify_one();
            self.release.notified().await;
        }
        self.inner.load(id).await
    }

    async fn save(&self, record: &CompanyRecord) -> crate::Result<()> {
        self.inner.save(record).await
    }

    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        self.inner.list().await
    }

    async fn append_ledger(&self, id: &CompanyId, entry: LedgerEntry) -> crate::Result<()> {
        self.inner.append_ledger(id, entry).await
    }
}

/// [`runtime`]'s twin, wired identically except the store is a [`GatedStore`]
/// the caller can later arm to freeze a resolve mid-continuation.
async fn runtime_with_gated_store(
    home: &std::path::Path,
    turns: Vec<Turn>,
) -> (
    Arc<crate::company::runtime::CompanyRuntime>,
    Arc<GatedStore>,
    Arc<RecordingRunner>,
) {
    let store = Arc::new(GatedStore::new(home));
    let mut rt = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_seed_dir(home.to_path_buf())
        .with_store(store.clone())
        .build()
        .await
        .expect("runtime builds");

    let (base_url, _script) = spawn_script_recording(turns).await;
    let (mut deps, _unused) = deps(base_url, home);
    deps.approval_requests = ApprovalRequestQueue::with_grants(rt.grants.clone());
    let delivery = deps.delivery.as_mut().expect("the fixture wires delivery");
    delivery.parking = Some(super::delivery::DeliveryParking {
        approvals: rt.approvals.clone(),
        journal: rt.journal().clone(),
        continuations: rt.continuations.clone(),
        gates: rt.workflow_gates().clone(),
        blocked_nodes: rt.blocked_nodes().clone(),
    });

    let pool = Arc::new(HarnessPool::new());
    pool.ensure(&record(), &deps).await.expect("roster builds");
    let turn = Arc::new(crate::harness::built_in::run_turn::HarnessRunTurn::new(
        pool,
        Arc::new(deps.clone()),
    ));
    let runner = Arc::new(RecordingRunner {
        inner: super::runner::HarnessWorkflowRunner::new(turn, deps, record()),
        started: Mutex::new(Vec::new()),
    });
    rt.set_workflow_runner(runner.clone());
    (Arc::new(rt), store, runner)
}

/// Starts the graph through the runtime's own runner — the console run path — and
/// returns the run id. The agent node parks its gated call and the run settles
/// Blocked rather than erroring.
async fn cold_run(rt: &Arc<crate::company::runtime::CompanyRuntime>) -> String {
    let file = parse_workflow(SOLO_TOML).expect("graph parses");
    let ctx = WorkflowRunContext::new(false);
    let run_id = ctx.run_id.clone();
    let runner = rt.workflow_runner().cloned().expect("a runner is wired");
    runner
        .run(rt.id(), &file, json!({ "request": "the thing" }), &ctx)
        .await
        .expect("a blocked run settles rather than failing");
    run_id
}

/// The agent-internal cards a given run parked, oldest first — the ones keyed
/// under this issue's `workflow-node:` turn key.
fn cards_for(rt: &Arc<crate::company::runtime::CompanyRuntime>, run_id: &str) -> Vec<ApprovalId> {
    let turn = workflow_node_turn_key(run_id, NODE);
    rt.journal()
        .pending()
        .into_iter()
        .filter(|entry| entry.batch.as_deref() == Some(turn.as_str()))
        .map(|entry| entry.id)
        .collect()
}

/// Every agent-internal card still parked, across all runs.
fn all_blocked_cards(rt: &Arc<crate::company::runtime::CompanyRuntime>) -> usize {
    rt.journal()
        .pending()
        .into_iter()
        .filter(|entry| {
            entry
                .batch
                .as_deref()
                .is_some_and(|b| b.starts_with("workflow-node:"))
        })
        .count()
}

/// A resolve, plus the settling window the detached continuation needs — the
/// continuation is spawned rather than awaited (issue #380's drop safety), so an
/// immediate assertion would race it. Bounded, so a genuine failure fails.
async fn resolve_and_settle(
    rt: &Arc<crate::company::runtime::CompanyRuntime>,
    id: &ApprovalId,
    verdict: Verdict,
) {
    rt.resolve_approval(id, verdict, operator())
        .await
        .expect("the verdict is recorded");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

/// The headline. On `main` this is red: approving an agent-internal card starts
/// no run, so `runs_started` stays 1. With the fix it is 2 — one continuation.
#[tokio::test]
async fn approving_a_blocked_agent_node_call_continues_the_run() {
    let home = seed_home();
    // Cold turn gates one call; the continuation redeems the grant and finishes.
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1, "the cold run parks exactly one card");
    assert_eq!(runner.started(), 1, "only the cold run has started");

    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;

    assert_eq!(
        runner.started(),
        2,
        "approving the card auto-continues the run — exactly one continuation"
    );
    assert_eq!(
        all_blocked_cards(&rt),
        0,
        "the continuation redeemed the grant and did not re-park (loop-safety)"
    );
}

/// Issue #1816 (Stage 2) — the headline durability regression. A process/host
/// replacement **between park and approve** (the ~90-min staging cron pod-roll)
/// drops the in-memory `BlockedNodeQueue`. On Stage 1 the approval then dead-ends
/// on "re-run the workflow", because nothing rehydrated the run. With the durable
/// stash record beneath the queue, the boot builder re-arms it from the journal
/// and the approval re-dispatches the run exactly as if no restart happened.
///
/// The restart is simulated the way the boot path works: the in-memory stash is
/// dropped, then the queue is re-armed from `journal.blocked_stashes()` — the
/// exact call `RuntimeBuilder` makes at boot. Deleting that single re-arm makes
/// this test red (`runs_started` stays 1), which is the Stage-1 / `main`
/// behaviour it locks against.
#[tokio::test]
async fn a_restart_between_park_and_approve_still_continues_the_run() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1, "the cold run parks exactly one card");
    assert_eq!(runner.started(), 1, "only the cold run has started");

    let turn = workflow_node_turn_key(&run_id, NODE);

    // The park wrote the continuation facts to the DURABLE journal, not only the
    // in-memory queue — the workflow id and the run's own trigger input.
    let stashes = rt.journal().blocked_stashes();
    let stash = stashes
        .iter()
        .find(|(t, ..)| t == &turn)
        .expect("the block's continuation facts are on the journal, survivable across a restart");
    assert_eq!(
        stash.1, "solo",
        "the stash carries the workflow id to re-load"
    );
    assert_eq!(
        stash.2,
        json!({ "request": "the thing" }),
        "the stash carries the paused run's own trigger input, replayed unchanged"
    );

    // Simulate the process/host replacement: the in-memory queue is gone.
    let lost = rt.blocked_nodes().release(&turn);
    assert!(
        lost.is_some(),
        "precondition: the block was in the in-memory queue before the drop"
    );
    assert_eq!(
        rt.blocked_nodes().waiting(),
        0,
        "the in-memory stash is now gone, as it would be after a restart"
    );

    // Boot rehydrate — byte-for-byte the call `RuntimeBuilder` makes from the
    // journal's still-live stashes. THIS is the line whose removal reproduces the
    // Stage-1 dead-end.
    rt.blocked_nodes().rearm(rt.journal().blocked_stashes());
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "the durable record re-armed the queue after the restart"
    );

    // The operator approves after the restart. The rehydrated stash lets the run
    // re-dispatch instead of stranding on "re-run the workflow".
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;

    assert_eq!(
        runner.started(),
        2,
        "an approval after a restart re-dispatches the run from the durable stash"
    );
    assert_eq!(
        all_blocked_cards(&rt),
        0,
        "the continuation redeemed the grant and did not re-park"
    );

    // The release retired the durable record, so a later boot will not rehydrate
    // a block this decision already continued.
    assert!(
        rt.journal()
            .blocked_stashes()
            .iter()
            .all(|(t, ..)| t != &turn),
        "the resolved block's durable stash is retired on release"
    );
}

/// A refused block starts no continuation — the run stays stopped, which is the
/// correct outcome for a denial.
#[tokio::test]
async fn a_denied_blocked_call_starts_no_continuation() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);

    resolve_and_settle(&rt, &cards[0], Verdict::Deny).await;

    assert_eq!(
        runner.started(),
        1,
        "a denied block starts nothing — no continuation run"
    );
}

/// A node blocked on **two** calls owes ONE continuation, after the LAST
/// decision — the #469 batch property, on the agent-node key. Approving the
/// first releases nothing.
#[tokio::test]
async fn two_calls_on_one_node_continue_once_after_the_last_decision() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            // Cold turn: two gated calls, then the model stops.
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo one" }),
            },
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo two" }),
            },
            Turn::Say("Both were refused, so I stopped."),
            // Continuation: both grants redeem, then it finishes.
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 2, "the node parked two cards under one batch");
    assert_eq!(runner.started(), 1);

    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(
        runner.started(),
        1,
        "the first of two decisions releases nothing"
    );

    resolve_and_settle(&rt, &cards[1], Verdict::Approve).await;
    assert_eq!(
        runner.started(),
        2,
        "the last decision releases exactly one continuation for the node"
    );
}

/// Issue #1816: a restart between the FIRST and SECOND decision on a two-call
/// node must not lose the first decision's approval.
///
/// `ContinuationQueue`'s released batch only carries the verdicts one process
/// held in memory (see that module's docs on `rearm`); a restart between two
/// decisions on the same node drops the earlier one from it. If the surviving
/// decision is a deny, the naive `approved = batch.iter().any(Approve)` this
/// module used to compute reads false even though the operator did approve
/// one of the node's two calls — and unlike a workflow gate, which re-parks
/// whatever its own batch forgets, a blocked node has no re-park fallback: it
/// either spawns the continuation once or never. On the pre-fix code this test
/// is red — `runner.started()` stays 1, the approved grant is minted and never
/// redeemed, and nothing tells the operator anything is wrong.
#[tokio::test]
async fn a_restart_between_two_decisions_does_not_lose_the_earlier_approval() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo one" }),
            },
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo two" }),
            },
            Turn::Say("Both were refused, so I stopped."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 2, "the node parked two cards under one batch");
    assert_eq!(runner.started(), 1);

    // Approve the first call. The turn is still blocked on the second, so
    // nothing continues yet — but the approve is durably banked the moment it
    // lands (issue #1816's fix), not only if it happens to survive to release.
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(runner.started(), 1, "one of two decisions releases nothing");

    let turn = workflow_node_turn_key(&run_id, NODE);
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "precondition: the node's stash is still live before the restart"
    );

    // Simulate the process/host replacement between the two decisions: a
    // fresh runtime built over the same home directory, so every in-memory
    // queue (`ContinuationQueue`, `BlockedNodeQueue`, `WorkflowGateQueue`)
    // starts empty and comes back only from what the on-disk journal replays
    // — exactly `a_restart_between_park_and_approve_still_continues_the_run`'s
    // scenario, but after one of two decisions rather than before either.
    let (rt2, runner2) = runtime(
        home.path(),
        vec![
            // The continuation replays the node's turn from its original
            // trigger input: the first call's grant redeems silently, and the
            // second — genuinely denied below, no grant minted — parks again.
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo one" }),
            },
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo two" }),
            },
            Turn::Say("The remaining call is still refused."),
        ],
    )
    .await;
    assert_eq!(
        rt2.blocked_nodes().waiting(),
        1,
        "the durable stash rehydrated across the simulated restart"
    );

    // The second (and last) decision, against the fresh runtime: denied. On
    // the pre-fix code the batch this releases carries only this deny, so
    // `approved` reads false and `runner2.started()` stays 0 — the first
    // call's grant is stranded, unredeemed, with nothing to tell the operator.
    resolve_and_settle(&rt2, &cards[1], Verdict::Deny).await;
    assert_eq!(
        runner2.started(),
        1,
        "the earlier approval must still release a continuation, even though \
         the deciding process never saw it get approved"
    );
}

/// Two runs blocked at once do not continue each other — the stash is per
/// (run, node), so approving one run's card leaves the other's untouched and
/// starts a continuation for the approved run only.
#[tokio::test]
async fn two_blocked_runs_do_not_cross_continue() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            // Run A cold turn.
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo a" }),
            },
            Turn::Say("A refused."),
            // Run B cold turn.
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo b" }),
            },
            Turn::Say("B refused."),
            // A's continuation.
            Turn::Say("A done."),
        ],
    )
    .await;

    // Sequential cold runs, so the shared script is consumed deterministically.
    let run_a = cold_run(&rt).await;
    let run_b = cold_run(&rt).await;
    let cards_a = cards_for(&rt, &run_a);
    let cards_b = cards_for(&rt, &run_b);
    assert_eq!(cards_a.len(), 1, "run A parked its own card");
    assert_eq!(cards_b.len(), 1, "run B parked its own card");
    assert_eq!(runner.started(), 2, "two cold runs, no continuations yet");

    resolve_and_settle(&rt, &cards_a[0], Verdict::Approve).await;

    assert_eq!(
        runner.started(),
        3,
        "approving A starts exactly one continuation (A's)"
    );
    assert_eq!(
        cards_for(&rt, &run_b).len(),
        1,
        "run B's card is untouched — no cross-continuation"
    );
}

/// Issue #1816 (Stage 3): a restart landing between the durable approval bank
/// (`record_blocked_node_approved`) and the in-memory decision that would
/// have released the block (`ContinuationQueue::decide`) must not strand the
/// run.
///
/// The approval here is the node's only (and therefore last) decision, so on
/// `main` a boot rehydrates the stash marked approved but `parked_turns()` no
/// longer names this turn at all — it already resolved before the crash — so
/// nothing scans for an approved, no-longer-waited-on stash, and the run sits
/// stranded forever: not a future decision away, because there is no future
/// decision coming.
///
/// The crash is reproduced by doing exactly what `resolve_approval` does up
/// to the point this issue is about — durably settling the approval, then
/// durably banking it — and then aborting before the in-memory decide, rather
/// than literally racing a process exit. `reconcile_stranded_blocked_nodes`
/// is called directly (the method the boot builder calls once, at the end of
/// every cold boot) so the assertion is on its exact logic without depending
/// on this fixture's own workflow-runner wiring order.
#[tokio::test]
async fn a_restart_after_the_last_approval_banks_but_before_release_still_continues_the_run() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1, "the cold run parks exactly one card");
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    // Durably settle the approval — removes it from `parked`, appends
    // `ApprovalResolved` — exactly as `resolve_approval` does, then abort the
    // detached follow-up before it can run `continue_turn` at all.
    let (_, follow_up) = rt
        .resolve_approval_spawned(&cards[0], Verdict::Approve, operator(), GrantScope::Once)
        .await
        .expect("the verdict settles durably");
    follow_up.abort();
    let _ = follow_up.await;

    // The bank `continue_turn` would have written next — both halves, exactly
    // as `blocked_nodes.mark_approved` + `journal.record_blocked_node_approved`
    // are called together there — done by hand, matching precisely what a
    // crash immediately after that awaited durable append succeeds would
    // leave behind: the in-memory flag set (nothing rebuilds this from the
    // durable record until a boot rearms it) and the durable record written.
    rt.blocked_nodes().mark_approved(&turn);
    rt.journal()
        .record_blocked_node_approved(&turn)
        .await
        .expect("the durable bank succeeds");

    assert_eq!(
        runner.started(),
        1,
        "precondition: the crash lands before any continuation runs"
    );
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "precondition: the stash is still live — release never ran"
    );
    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "precondition: the approval is durably banked"
    );
    assert!(
        rt.journal().parked_turns().iter().all(|t| t != &turn),
        "precondition: the approval is already durably resolved, so a boot \
         rearm would find nothing left parked for this turn"
    );

    // On `main` nothing drives this: the boot builder rearms `blocked_nodes`
    // (approved) and `continuations` (nothing outstanding for this turn,
    // since nothing is parked for it), and no future decision is coming to
    // trigger `resume_blocked_agent_node`. This is the reconciliation this
    // issue adds, run exactly as the boot builder runs it once per cold boot.
    rt.reconcile_stranded_blocked_nodes().await;

    assert_eq!(
        runner.started(),
        2,
        "the stranded approval is resumed exactly once, without waiting for a \
         decision that will never come"
    );
    assert_eq!(
        all_blocked_cards(&rt),
        0,
        "the continuation redeemed the grant and did not re-park"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the stash is retired once the continuation actually runs"
    );
}

/// A node still genuinely waiting on a sibling decision must not be resumed
/// by reconciliation just because one of its two calls is durably approved —
/// only a turn with nothing left parked is stranded; this one is mid-turn.
#[tokio::test]
async fn reconciliation_does_not_fire_early_on_a_node_still_awaiting_a_sibling_decision() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo one" }),
            },
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo two" }),
            },
            Turn::Say("Both were refused, so I stopped."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 2, "the node parked two cards under one batch");

    // Approve the first of two — durably banked, but the second call is still
    // parked, so this turn is genuinely mid-block, not stranded.
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(runner.started(), 1, "one of two decisions releases nothing");

    let turn = workflow_node_turn_key(&run_id, NODE);
    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "the first approval is durably banked"
    );
    assert!(
        rt.journal().parked_turns().iter().any(|t| t == &turn),
        "the second call is still parked — this turn is not stranded"
    );

    rt.reconcile_stranded_blocked_nodes().await;

    assert_eq!(
        runner.started(),
        1,
        "reconciliation must not resume a node still waiting on a sibling \
         decision — that would run the continuation before the operator has \
         finished deciding"
    );
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "the stash is untouched — still correctly waiting on the second call"
    );
}

/// Issue #1816 (Stage 4): a restart landing strictly *during* the awaited
/// `spawn_blocked_node_continuation` call — after `resume_blocked_agent_node`
/// has committed to continuing, but before the run is actually admitted —
/// must not lose the stash. On `main` the durable release (and the in-memory
/// drop beneath it) fires unconditionally before the spawn is even
/// attempted, so a crash in that window strands the run exactly like a spawn
/// that never happened: no pending decision (already resolved, per the
/// approval's own `ApprovalResolved`), no stash (already released), nothing
/// left to rehydrate.
///
/// Proven by freezing the resolve inside `spawn_blocked_node_continuation`'s
/// own `store().load(...)` await — the one genuine suspension point between
/// "committed to continuing" and the run being registered, since the actual
/// graph run is hand off to a task `WorkflowSpawn::spawn` does not await, so
/// nothing past that point is observably synchronous with the caller. On
/// `main` the stash is already gone by the time this point is reached —
/// release fired before `spawn_blocked_node_continuation` was ever called.
#[tokio::test]
async fn the_durable_stash_survives_until_the_spawn_is_admitted() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_gated_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);
    let card = cards[0].clone();

    // Arm the gate only now — the builder's own boot-time `store.load` (and
    // the cold run's own reads) must pass through untouched.
    store.armed.store(true, Ordering::SeqCst);

    // The resolve has to run concurrently: `resolve_approval` awaits its own
    // follow-up cycle to completion, and that cycle is what will be frozen
    // inside the gate.
    let rt_task = Arc::clone(&rt);
    let resolve = tokio::spawn(async move {
        rt_task
            .resolve_approval(&card, Verdict::Approve, operator())
            .await
    });

    // Deterministic rendezvous: block here until the continuation's own
    // `store().load(...)` has actually been entered, rather than racing a
    // fixed sleep against it.
    store.reached.notified().await;

    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "the durable stash must still be live the instant the spawn attempt \
         actually begins — releasing it beforehand is exactly the gap this \
         fix closes"
    );
    assert!(
        !rt.journal().blocked_stashes().is_empty(),
        "and the durable record beneath it must still be live too"
    );

    // Let the frozen resolve finish. `resolve_approval` returns once
    // `spawn_blocked_node_continuation` is admitted, not once the graph run
    // it detaches has actually executed — the same settling window
    // `resolve_and_settle` gives every other test in this module.
    store.release.notify_one();
    resolve
        .await
        .expect("the spawned resolve task itself does not panic")
        .expect("the verdict resolves");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert_eq!(
        runner.started(),
        2,
        "the continuation actually ran once the gate released"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "once the spawn is admitted, the stash is retired same as before"
    );
}

/// Issue #1825: a restart landing between `settle_approval` durably resolving
/// the verdict and the detached follow-up task (`spawn_follow_up` →
/// `continue_turn`) ever being polled must not lose the blocked-node bank.
///
/// Before this fix, `blocked_nodes.mark_approved` and
/// `journal.record_blocked_node_approved` were only ever called from inside
/// `continue_turn`, on the detached task `resolve_approval_spawned` hands off
/// to *after* the settle has already returned durably. Aborting that task
/// before it is polled — precisely what a process restart in that window
/// does — used to leave the approval durably resolved (`ApprovalResolved` is
/// in the journal) but the blocked-node bank never written, so a boot could
/// not tell this stash was ever approved and `reconcile_stranded_blocked_nodes`
/// would never revisit it. The fix moved the bank inline into
/// `settle_approval` itself, so it lands before the receipt is even returned
/// and before the follow-up task exists to be aborted.
///
/// Unlike the sibling regression test above
/// (`a_restart_after_the_last_approval_banks_but_before_release_still_continues_the_run`),
/// this test performs **no manual bank** — it aborts the follow-up and checks
/// the bank landed anyway, so it actually exercises the inline path rather
/// than asserting a hand-simulated substitute for it.
#[tokio::test]
async fn the_blocked_node_bank_survives_a_restart_before_the_detached_follow_up_runs() {
    let home = seed_home();
    let (rt, runner) = runtime(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1, "the cold run parks exactly one card");
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    // Durably settle the approval, then abort the detached follow-up before
    // it can be polled at all — simulating a restart in exactly the window
    // between the settle returning and that task's first poll. No manual
    // bank follows: everything asserted below must already be true.
    let (_, follow_up) = rt
        .resolve_approval_spawned(&cards[0], Verdict::Approve, operator(), GrantScope::Once)
        .await
        .expect("the verdict settles durably");
    follow_up.abort();
    let _ = follow_up.await;

    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "the in-memory bank must be set by the inline settle, with no \
         follow-up task ever having run"
    );
    assert!(
        rt.journal()
            .blocked_node_approvals()
            .iter()
            .any(|t| t == &turn),
        "and the durable journal record must exist too — this is the fact a \
         boot rearm reads, not the in-memory flag"
    );

    // Confirm the bank is load-bearing exactly the same way the sibling test
    // proves it: with nothing left parked for this turn, reconciliation is
    // the only thing that can still redeem it, and it must be able to.
    rt.reconcile_stranded_blocked_nodes().await;
    assert_eq!(
        runner.started(),
        2,
        "reconciliation redeems the bank the inline settle wrote, exactly as \
         it does the hand-simulated one in the sibling test"
    );
}
