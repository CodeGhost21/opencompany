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
use crate::error::OpenCompanyError;
use crate::harness::HarnessPool;
use crate::harness::policy::ApprovalRequestQueue;
use crate::ports::journal::MemoryJournalStore;
use crate::ports::types::{
    Actor, ActorKind, ApprovalId, CompanyId, CompanyRecord, CompanySummary, LedgerEntry, StartedBy,
    Verdict,
};
use crate::ports::{
    CompanyStore, Durability, JournalStore, WorkflowRun, WorkflowRunContext, WorkflowRunner,
};
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
/// Wires a built [`CompanyRuntime`] up to a [`RecordingRunner`] — the block
/// every `runtime*` fixture below shares byte-for-byte (CodeRabbit nitpick,
/// review 5038258829): scripted-model deps, delivery parking, the harness
/// pool/turn, and the runner itself, in that order. Each fixture differs only
/// in how it builds `rt` (a plain builder, a pinned run-supervisor limit, or a
/// swapped-in journal/company store) before calling this; extracting it means
/// a future change to `DeliveryParking` (or anything else this wires) lands
/// once instead of once per fixture, where a missed site would otherwise
/// silently change what a test proves.
async fn wire_recording_runner(
    rt: &mut crate::company::runtime::CompanyRuntime,
    home: &std::path::Path,
    turns: Vec<Turn>,
) -> Arc<RecordingRunner> {
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
    runner
}

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
    let runner = wire_recording_runner(&mut rt, home, turns).await;
    (Arc::new(rt), runner)
}

/// [`runtime`]'s twin, with the run supervisor's concurrency ceiling pinned
/// to `limit` instead of [`DEFAULT_MAX_IN_FLIGHT_RUNS`](crate::company::DEFAULT_MAX_IN_FLIGHT_RUNS)'s
/// 8, for issue #1825's P2 capacity-refusal regression.
///
/// Set directly via [`CompanyRuntime::set_run_supervisor`] rather than
/// through `[workflows].max_in_flight_runs` in the manifest: the builder only
/// derives the supervisor from that field on the branch where it resolves a
/// **real** inference config (`RuntimeBuilder::build`'s `configured` gate),
/// and this fixture's model is a scripted loopback server wired in by hand
/// afterwards, same as [`runtime`] — so the manifest route silently keeps the
/// default-8 supervisor `CompanyRuntime::new` already carries, and a `limit`
/// this low would never actually bind.
async fn runtime_with_run_limit(
    home: &std::path::Path,
    limit: usize,
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
    rt.set_run_supervisor(crate::runtime::RunSupervisor::with_limit(limit));
    let runner = wire_recording_runner(&mut rt, home, turns).await;
    (Arc::new(rt), runner)
}

/// A [`JournalStore`] that suspends `append_journal` on a gate the test
/// controls whenever the line it is about to write contains `match_substr`,
/// wrapping an in-memory backend so every other append — and every append
/// before the gate is armed — passes straight through.
///
/// Issue #1825 (P1 follow-up): this fix's whole point is that the durable
/// `BlockedNodeDispatched` write now lands *before* the run it marks is
/// handed to `tokio::spawn`, not after — closing the window where a crash
/// between "the run was launched" and "the marker landed" could leave a
/// finished (or still-running) continuation with nothing durable saying so,
/// so a restart's `reconcile_stranded_blocked_nodes` would dispatch it a
/// second time. Freezing exactly the marker's own append — nothing else — is
/// what lets a test observe, deterministically rather than by racing real
/// wall-clock timing, whether the run has been launched yet at the instant
/// the marker write is attempted: on the fix, it cannot have been (the write
/// happens first), so `RecordingRunner::run` must not have been entered; on
/// the ordering this fix replaces, the run was already launched before this
/// write was ever attempted — so freezing this task here starves nothing
/// else the single-threaded test runtime needs, and the already-launched
/// detached run keeps running to completion while this one waits.
struct GatedJournalStore {
    inner: MemoryJournalStore,
    match_substr: &'static str,
    armed: AtomicBool,
    reached: Notify,
    release: Notify,
}

impl GatedJournalStore {
    fn new(match_substr: &'static str) -> Self {
        Self {
            inner: MemoryJournalStore::default(),
            match_substr,
            armed: AtomicBool::new(false),
            reached: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait]
impl JournalStore for GatedJournalStore {
    async fn append_journal(
        &self,
        id: &CompanyId,
        line: &str,
        durability: Durability,
    ) -> crate::Result<()> {
        if self.armed.load(Ordering::SeqCst) && line.contains(self.match_substr) {
            self.reached.notify_one();
            self.release.notified().await;
        }
        self.inner.append_journal(id, line, durability).await
    }

    async fn read_journal(&self, id: &CompanyId) -> crate::Result<Vec<String>> {
        self.inner.read_journal(id).await
    }

    async fn journal_imported(&self, id: &CompanyId) -> crate::Result<bool> {
        self.inner.journal_imported(id).await
    }

    async fn complete_import(&self, id: &CompanyId, lines: Vec<String>) -> crate::Result<()> {
        self.inner.complete_import(id, lines).await
    }
}

/// [`runtime`]'s twin, wired identically except the journal's durable sink is
/// a [`GatedJournalStore`] the caller can later arm to freeze mid-append.
async fn runtime_with_gated_journal_store(
    home: &std::path::Path,
    turns: Vec<Turn>,
    match_substr: &'static str,
) -> (
    Arc<crate::company::runtime::CompanyRuntime>,
    Arc<GatedJournalStore>,
    Arc<RecordingRunner>,
) {
    let store = Arc::new(GatedJournalStore::new(match_substr));
    let mut rt = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_seed_dir(home.to_path_buf())
        .with_journal_store(store.clone())
        .build()
        .await
        .expect("runtime builds");
    let runner = wire_recording_runner(&mut rt, home, turns).await;
    (Arc::new(rt), store, runner)
}

/// A [`JournalStore`] that fails `append_journal` with
/// [`OpenCompanyError::Store`] for exactly the first `fail_count` appends
/// whose line contains `match_substr`, then passes every append — including
/// later ones matching the same substring — straight through to an in-memory
/// backend.
///
/// Issue #1825 (P1 follow-ups, found by chatgpt-codex-connector): two
/// separate durable writes on this path — `record_blocked_node_dispatched`
/// and `record_blocked_node_approved` — used to swallow their own failure
/// (`tracing::warn!` and move on) rather than treat it as the load-bearing
/// fact it is. This is the harness that reproduces a *genuine* write failure
/// (as opposed to [`GatedJournalStore`]'s crash-race simulation) so a test
/// can assert on what each call site actually does with it: abort without
/// launching (the dispatch marker) or retry before giving up (the approval
/// bank).
struct FailNJournalStore {
    inner: MemoryJournalStore,
    match_substr: &'static str,
    remaining_failures: std::sync::atomic::AtomicUsize,
}

impl FailNJournalStore {
    fn new(match_substr: &'static str, fail_count: usize) -> Self {
        Self {
            inner: MemoryJournalStore::default(),
            match_substr,
            remaining_failures: std::sync::atomic::AtomicUsize::new(fail_count),
        }
    }
}

#[async_trait]
impl JournalStore for FailNJournalStore {
    async fn append_journal(
        &self,
        id: &CompanyId,
        line: &str,
        durability: Durability,
    ) -> crate::Result<()> {
        if line.contains(self.match_substr) {
            let prev =
                self.remaining_failures
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                        if n == 0 { None } else { Some(n - 1) }
                    });
            if prev.is_ok() {
                return Err(OpenCompanyError::Store(format!(
                    "FailNJournalStore: forced failure on a line matching {:?}",
                    self.match_substr
                )));
            }
        }
        self.inner.append_journal(id, line, durability).await
    }

    async fn read_journal(&self, id: &CompanyId) -> crate::Result<Vec<String>> {
        self.inner.read_journal(id).await
    }

    async fn journal_imported(&self, id: &CompanyId) -> crate::Result<bool> {
        self.inner.journal_imported(id).await
    }

    async fn complete_import(&self, id: &CompanyId, lines: Vec<String>) -> crate::Result<()> {
        self.inner.complete_import(id, lines).await
    }
}

/// [`runtime`]'s twin, wired identically except the journal's durable sink is
/// a [`FailNJournalStore`] that fails the first `fail_count` appends matching
/// `match_substr`.
async fn runtime_with_failing_journal_store(
    home: &std::path::Path,
    turns: Vec<Turn>,
    match_substr: &'static str,
    fail_count: usize,
) -> (
    Arc<crate::company::runtime::CompanyRuntime>,
    Arc<FailNJournalStore>,
    Arc<RecordingRunner>,
) {
    let store = Arc::new(FailNJournalStore::new(match_substr, fail_count));
    let mut rt = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_seed_dir(home.to_path_buf())
        .with_journal_store(store.clone())
        .build()
        .await
        .expect("runtime builds");
    let runner = wire_recording_runner(&mut rt, home, turns).await;
    (Arc::new(rt), store, runner)
}

/// Whether the store's own backing log — not `RuntimeJournal`'s in-memory
/// mirror, which a live `record_*` call updates optimistically before the
/// fallible durable append it guards even runs — actually holds a line
/// naming `turn` under `record_kind` (e.g. `"BlockedNodeApproved"`).
///
/// The in-memory mirror is what `blocked_node_approvals()`/
/// `blocked_node_dispatched()` read, and it is correct for their one real
/// caller — boot-time rearm, which constructs a fresh `RuntimeJournal` from a
/// replay of what is genuinely on disk, never from a live optimistic write.
/// Mid-session, though, it is not a reliable oracle for "did this attempt's
/// write actually land", which is exactly what these tests need to tell a
/// retried success apart from a failure the retry never actually recovered.
async fn store_durably_has(
    store: &FailNJournalStore,
    company: &CompanyId,
    record_kind: &str,
    turn: &str,
) -> bool {
    store
        .inner
        .read_journal(company)
        .await
        .expect("the in-memory backend never fails to read")
        .iter()
        .any(|line| line.contains(record_kind) && line.contains(turn))
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
    let runner = wire_recording_runner(&mut rt, home, turns).await;
    (Arc::new(rt), store, runner)
}

/// Starts the graph through the runtime's own runner — the console run path — and
/// returns the run id. The agent node parks its gated call and the run settles
/// Blocked rather than erroring.
async fn cold_run(rt: &Arc<crate::company::runtime::CompanyRuntime>) -> String {
    cold_run_with_started_by(rt, StartedBy::Operator).await
}

/// Same as [`cold_run`], but names the triggering `started_by` explicitly —
/// for tests that need an agent- or schedule-started run rather than the
/// operator default (issue #1862 prerequisite).
async fn cold_run_with_started_by(
    rt: &Arc<crate::company::runtime::CompanyRuntime>,
    started_by: StartedBy,
) -> String {
    let file = parse_workflow(SOLO_TOML).expect("graph parses");
    let ctx = WorkflowRunContext::new(false).with_started_by(started_by);
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

/// Issue #1862 prerequisite (`CodeRabbit`, comment `3879554180`; also raised by
/// `chatgpt-codex-connector`, comment `3879402310`): a restart between park and
/// approve must not silently reattribute an agent-started run to `Operator`.
///
/// [`BlockedNodeQueue::rearm`](crate::runtime::blocked_nodes::BlockedNodeQueue::rearm)
/// used to hardcode `StartedBy::Operator` for every stash it rehydrated,
/// because the durable [`BlockedNodeStashed`](crate::runtime::journal::JournalRecord::BlockedNodeStashed)
/// record only ever carried `(turn, workflow_id, input)` — the run's real
/// attribution never made it onto that record in the first place, so `rearm`
/// had nothing but the coarse default to fall back on. An agent-started run
/// that blocked and outlived a process/host replacement would come back
/// attributed to `Operator`, so a later blocker DM would address the wrong
/// sender — defeating the whole point of issue #1862's prerequisite for any
/// run a restart landed on. This is otherwise the identical scenario
/// `a_restart_between_park_and_approve_still_continues_the_run` drives; that
/// test's cold run is operator-started, so it could not have caught a
/// mis-attribution to the very value it never left.
#[tokio::test]
async fn a_restart_between_park_and_approve_preserves_agent_attribution() {
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

    let run_id = cold_run_with_started_by(&rt, StartedBy::Agent("ceo".to_string())).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1, "the cold run parks exactly one card");

    let turn = workflow_node_turn_key(&run_id, NODE);

    // Simulate the process/host replacement: the in-memory queue is gone.
    let lost = rt.blocked_nodes().release(&turn);
    assert!(
        lost.is_some(),
        "precondition: the block was in the in-memory queue before the drop"
    );

    // Boot rehydrate — byte-for-byte the call `RuntimeBuilder` makes from the
    // journal's still-live stashes.
    rt.blocked_nodes().rearm(rt.journal().blocked_stashes());
    let rehydrated = rt
        .blocked_nodes()
        .peek(&turn)
        .expect("the durable record re-armed the queue after the restart");
    assert_eq!(
        rehydrated.started_by,
        StartedBy::Agent("ceo".to_string()),
        "a restart between park and approve must rehydrate the run's real \
         attribution, not silently degrade an agent-started run to Operator"
    );

    // The operator approves after the restart; the rehydrated stash's
    // attribution rides the continuation exactly as it would have with no
    // restart at all.
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(
        runner.started(),
        2,
        "an approval after a restart re-dispatches the run from the durable stash"
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

/// Issue #1825 (P2 follow-up, found by chatgpt-codex-connector): a restart
/// landing after a blocked node's last decision resolves as a denial (or
/// expiry) — but before the retirement that resolution owes — must not leave
/// the stash rehydrated forever.
///
/// Mirrors `a_restart_after_the_last_approval_banks_but_before_release_still_continues_the_run`
/// exactly, but denies the node's only call instead of approving it. On
/// `main`, `reconcile_stranded_blocked_nodes` only ever scanned
/// `approved_turns()` — a stash that is durably unparked (every decision on
/// it landed) but never approved never appears there, so it rehydrates on
/// every boot's `rearm` and is never retired: the leak the finding names,
/// growing by one turn per restart that races this exact window, or whose
/// `retire_blocked_stash` write itself fails.
#[tokio::test]
async fn reconciliation_retires_an_unapproved_stash_stranded_after_its_last_denial() {
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

    // Durably settle the denial — removes it from `parked`, appends
    // `ApprovalResolved` — exactly as `resolve_approval` does, then abort the
    // detached follow-up before it can run `continue_turn` (and therefore
    // `retire_blocked_stash`) at all.
    let (_, follow_up) = rt
        .resolve_approval_spawned(&cards[0], Verdict::Deny, operator(), GrantScope::Once)
        .await
        .expect("the verdict settles durably");
    follow_up.abort();
    let _ = follow_up.await;

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
        !rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "precondition: nothing on this turn was ever approved"
    );
    assert!(
        rt.journal().parked_turns().iter().all(|t| t != &turn),
        "precondition: the denial is already durably resolved, so a boot \
         rearm would find nothing left parked for this turn"
    );

    // On `main` nothing drives a retirement here: `reconcile_stranded_blocked_nodes`
    // only ever scanned `approved_turns()`, and this turn — durably unparked,
    // never approved — never appeared there, so it survives every subsequent
    // boot's rearm unretired.
    rt.reconcile_stranded_blocked_nodes().await;

    assert_eq!(
        runner.started(),
        1,
        "an unapproved, resolved stash must not be dispatched — there is no approval to \
         redeem"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the stash is retired instead of surviving indefinitely in memory"
    );
    assert!(
        rt.journal()
            .blocked_stashes()
            .iter()
            .all(|(t, ..)| t != &turn),
        "the durable stash is retired too, or the next boot's rearm would rehydrate the \
         same leak"
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
/// graph run is handed off to a task `WorkflowSpawn::spawn` does not await, so
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

/// Issue #1825: a restart landing between a blocked node's continuation
/// actually being spawned and its `BlockedNodeReleased` write durably landing
/// must not cause `reconcile_stranded_blocked_nodes` to dispatch it a second
/// time.
///
/// `retire_blocked_stash`'s durable clear is best-effort by design (matching
/// the park's own stance), so a transient failure there is expected to
/// happen occasionally. Before this fix, the only durable facts left behind
/// by that failure — a still-armed `BlockedNodeStashed` paired with a
/// `BlockedNodeApproved` — were *indistinguishable* from a stash that was
/// never dispatched at all, so reconciliation would spawn the continuation
/// again: a duplicated agent turn over the same approved call, potentially
/// repeating token spend or an unprotected upstream side effect.
///
/// The failure is reproduced by hand, the same technique the sibling
/// release-race test above uses: run the real approve-and-dispatch path
/// once (so `spawn_blocked_node_continuation` genuinely succeeds and
/// `record_blocked_node_dispatched` genuinely lands), then re-create by hand
/// the exact durable state a failed `BlockedNodeReleased` write would have
/// left behind — the stash and its approval both still present, using the
/// **real** `workflow_id`/`input` this run stashed, so a broken guard would
/// actually be able to re-dispatch it rather than failing for an unrelated
/// reason (a fake workflow id would error out of `spawn_blocked_node_continuation`
/// before ever reaching the guard this test exists to prove).
#[tokio::test]
async fn reconciliation_does_not_redispatch_a_node_whose_dispatch_already_landed() {
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
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    // Capture the real stash contents before it is ever taken, so the
    // hand-rebuilt stash below is dispatchable for real.
    let original = rt
        .blocked_nodes()
        .peek(&turn)
        .expect("the cold run's block is stashed");

    // The real path: approve, let the real continuation actually spawn and
    // retire the stash. This is the one dispatch the guard must not repeat.
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(
        runner.started(),
        2,
        "the approval dispatched exactly one continuation"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the real release succeeded here — this test now rebuilds by hand \
         exactly what a *failed* release would have left behind instead"
    );

    // Re-create, by hand, the durable state a `record_blocked_node_released`
    // write failure would leave after that real dispatch: the stash and its
    // approval both restored (as release never ran), plus the
    // `BlockedNodeDispatched` marker the dispatch that actually happened did
    // manage to write.
    rt.blocked_nodes().arm(
        &turn,
        &original.workflow_id,
        &original.input,
        &original.started_by,
    );
    rt.blocked_nodes().mark_approved(&turn);
    rt.journal()
        .record_blocked_node_stashed(
            &turn,
            &original.workflow_id,
            &original.input,
            &original.started_by,
        )
        .await
        .expect("the durable re-stash succeeds");
    rt.journal()
        .record_blocked_node_approved(&turn)
        .await
        .expect("the durable re-approve succeeds");
    rt.journal()
        .record_blocked_node_dispatched(&turn)
        .await
        .expect("the durable dispatch marker succeeds");

    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "precondition: the rebuilt stash looks exactly like the stranded case"
    );
    assert!(
        rt.journal().parked_turns().iter().all(|t| t != &turn),
        "precondition: nothing is left parked for this turn"
    );

    rt.reconcile_stranded_blocked_nodes().await;

    assert_eq!(
        runner.started(),
        2,
        "a turn already durably marked dispatched must not be dispatched a \
         second time by reconciliation"
    );
}

/// Issue #1825 (finding `3877718169`, chatgpt-codex-connector): a ghost
/// decision reaching the **live** resolve path — not the boot reconciler —
/// must not repeat a dispatch that already landed.
///
/// `ApprovalResolved` is `Durability::Process`, deliberately: the journal's
/// own doc on it argues a ghost approval "cannot duplicate the effect,
/// because the effect's own commit is host-durable and `is_executed` skips
/// it." That covers a gated call's own effect replayed through the same
/// park. It says nothing about this node's *continuation dispatch*, which
/// `resume_blocked_agent_node` used to launch on nothing but
/// `stashed.is_some() && approved` — no check against
/// `blocked_node_dispatched` at all, unlike
/// `reconciliation_does_not_redispatch_a_node_whose_dispatch_already_landed`
/// above.
///
/// The twin of that test, but for the path it does not cover: instead of
/// calling `reconcile_stranded_blocked_nodes` directly (which never reaches
/// this turn once `already_dispatched` matches, and never reaches it at all
/// while the card is still parked — see `reconciliation_does_not_fire_early_
/// on_a_node_still_awaiting_a_sibling_decision`), this drives the exact
/// public API a second operator click takes: `resolve_approval` on a card
/// parked, by hand, for the same turn — reproducing what a host crash that
/// loses only the `Process`-tier resolution leaves behind: the original
/// card's own `ApprovalParked` line (`Durability::Host`) survives right
/// alongside the `BlockedNodeApproved`/`BlockedNodeDispatched` facts that
/// already redeemed it, so the turn reads as "still parked" rather than
/// "stranded" and the reconciler leaves it alone.
#[tokio::test]
async fn a_ghost_decision_on_the_live_path_does_not_redispatch_an_already_dispatched_node() {
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
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    // Capture the stash contents and the card's own effect before either is
    // ever taken — both are needed below to rebuild what a host crash leaves
    // behind, and `resolve_and_settle` removes the card from `pending()`.
    let original = rt
        .blocked_nodes()
        .peek(&turn)
        .expect("the cold run's block is stashed");
    let original_effect = rt
        .journal()
        .pending()
        .into_iter()
        .find(|p| p.id == cards[0])
        .expect("the cold run's card is still parked")
        .effect;

    // The real path: approve, let the real continuation actually spawn and
    // retire the stash. This is the one dispatch the ghost must not repeat.
    resolve_and_settle(&rt, &cards[0], Verdict::Approve).await;
    assert_eq!(
        runner.started(),
        2,
        "the approval dispatched exactly one continuation"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the real release succeeded here — the crash below is reconstructed \
         by hand on top of this, same as the reconciliation test above"
    );

    // Rebuild, by hand, exactly what a host crash losing only the
    // `Process`-tier `ApprovalResolved` record leaves behind: the stash and
    // its approval both restored, and the dispatch marker that already fired
    // — all three `Durability::Host`, so none of them were lost.
    rt.blocked_nodes().arm(
        &turn,
        &original.workflow_id,
        &original.input,
        &original.started_by,
    );
    rt.blocked_nodes().mark_approved(&turn);
    rt.journal()
        .record_blocked_node_stashed(
            &turn,
            &original.workflow_id,
            &original.input,
            &original.started_by,
        )
        .await
        .expect("the durable re-stash succeeds");
    rt.journal()
        .record_blocked_node_approved(&turn)
        .await
        .expect("the durable re-approve succeeds");
    rt.journal()
        .record_blocked_node_dispatched(&turn)
        .await
        .expect("the durable dispatch marker succeeds");

    // The piece the reconciliation test does not need: the card itself is
    // also `Durability::Host` (issue #1825's own earlier fix,
    // `a_blocked_node_tool_call_park_is_host_durable_too` in `journal.rs`),
    // so it survives the same crash right alongside the facts above — a
    // fresh id here stands in for "the original card, still parked",
    // functionally identical from the runtime's side: a live, decidable card
    // whose `approval_cycle` maps to an already-dispatched turn. Routed
    // through the real production entry point (`DeliveryParking::
    // park_and_journal`, the same call `park_gated_calls` makes), reusing
    // `rt`'s own live handles so the resolve below is indistinguishable from
    // a genuine operator click.
    let parking = super::delivery::DeliveryParking {
        approvals: rt.approvals.clone(),
        journal: rt.journal().clone(),
        continuations: rt.continuations.clone(),
        gates: rt.workflow_gates().clone(),
        blocked_nodes: rt.blocked_nodes().clone(),
    };
    let ghost_id = parking
        .park_and_journal(
            rt.id(),
            original_effect,
            crate::runtime::journal::TaskLink::Unlinked,
            None,
            Some(turn.clone()),
        )
        .await
        .expect("the ghost card parks");

    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "precondition: the rebuilt stash looks exactly like the stranded case"
    );
    assert!(
        rt.journal().parked_turns().iter().any(|t| t == &turn),
        "precondition: unlike the reconciliation case, this turn still has a \
         card parked — the ghost — so a boot's reconciler would read this as \
         mid-turn and leave it alone"
    );
    assert!(
        rt.journal().is_blocked_node_dispatched(&turn),
        "precondition: the dispatch marker is durably set"
    );

    // The second click: resolved through the same public API a real operator
    // action uses, not a direct call into `resume_blocked_agent_node`.
    resolve_and_settle(&rt, &ghost_id, Verdict::Approve).await;

    assert_eq!(
        runner.started(),
        2,
        "a ghost decision on an already-dispatched turn must not launch a \
         second continuation — that would duplicate model spend and any \
         external work the first continuation already did"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the stash is retired once the ghost decision is recorded, so it \
         does not linger for a third click to find"
    );
}

/// Issue #1825 (P1 follow-up, found by chatgpt-codex-connector): the durable
/// `BlockedNodeDispatched` marker must land *before* the continuation it
/// marks is handed to `tokio::spawn`, not after.
///
/// Before this fix, `resume_blocked_agent_node` wrote the marker only once
/// `spawn_blocked_node_continuation` had already returned — and that
/// function's own doc is explicit that the run it starts is detached, never
/// awaited by its caller. So the marker raced the *entire* run rather than a
/// moment's gap: a crash any time between the run being launched and the
/// write landing — however long the graph took — left the same durable
/// signature as no dispatch at all, and `reconcile_stranded_blocked_nodes`
/// would launch a second one over an approval whose first continuation may
/// already have finished (or still be mid-flight, having already taken real
/// action).
///
/// Proven by freezing the marker's own journal append (via
/// [`GatedJournalStore`]) and checking, at the instant the write is
/// attempted, whether the continuation's run has been launched yet. On `main`
/// the run was launched (and — nothing else competing for the single-threaded
/// test runtime's one thread — has already run to completion) *before* the
/// write was ever attempted, so `runner.started()` already reads 2 at that
/// point. With the fix, admission happens, the write is attempted (and freezes
/// here), and only once it lands does `spawn_admitted` exist to launch
/// anything — so `runner.started()` must still read 1.
#[tokio::test]
async fn the_dispatch_marker_lands_before_the_run_is_launched_not_after() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_gated_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeDispatched",
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    // Arm the gate only now — the cold run's own park/stash appends (and the
    // upcoming `ApprovalResolved`/`BlockedNodeApproved` writes) must pass
    // through untouched; only the dispatch marker itself is frozen.
    store.armed.store(true, Ordering::SeqCst);

    let card = cards[0].clone();
    let rt_task = Arc::clone(&rt);
    let resolve = tokio::spawn(async move {
        rt_task
            .resolve_approval(&card, Verdict::Approve, operator())
            .await
    });

    // Deterministic rendezvous: block here until the dispatch marker's own
    // append has actually been attempted, rather than racing a fixed sleep
    // against it.
    store.reached.notified().await;

    assert_eq!(
        runner.started(),
        1,
        "the continuation's run must not exist yet the instant the dispatch marker's own \
         write is attempted — a run already launched (and, with nothing else for this \
         single-threaded runtime to do while the marker write is frozen, already finished) \
         before that write was ever attempted is exactly the ordering this fix closes"
    );

    // Let the frozen write land, and the run it now precedes actually launch.
    store.release.notify_one();
    let result = resolve
        .await
        .expect("the spawned resolve task itself does not panic");
    assert!(result.is_ok(), "the continuation dispatches: {result:?}");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert_eq!(
        runner.started(),
        2,
        "the continuation actually ran once the marker write released"
    );
}

/// Issue #1825 (P2 follow-up, found by chatgpt-codex-connector): a blocked
/// node's continuation refused at the concurrency ceiling must not have its
/// recovery record thrown away.
///
/// `spawn_blocked_node_continuation` admits through the same
/// `RunSupervisor::begin` every other entry point uses, so a company already
/// at `[workflows].max_in_flight_runs` refuses a resume exactly as it refuses
/// a fresh run — `Err(WorkflowRunLimit)`. Before this fix,
/// `resume_blocked_agent_node`'s `Err` arm retired the stash unconditionally,
/// on every error alike — so a capacity refusal discarded the only durable
/// record the operator's already-given approval had, even though nothing
/// about the approval or the graph was wrong, only that a slot was not free
/// yet. Once the ceiling freed there was nothing left to retry from: not this
/// decision (already resolved), not a future one (none coming), not the
/// durable stash (retired). The approval was simply gone.
///
/// The refusal is reproduced by hand-occupying the company's sole
/// concurrency slot through the same `RunSupervisor::begin` a real run would
/// use, held open by not dropping the guard, so the continuation's own
/// `begin` genuinely refuses rather than the test asserting on a mocked
/// error.
#[tokio::test]
async fn a_capacity_refusal_keeps_the_stash_for_a_later_retry() {
    let home = seed_home();
    let (rt, runner) = runtime_with_run_limit(
        home.path(),
        1,
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

    // Occupy the company's sole slot — the same bookkeeping `WorkflowSpawn`
    // performs on any other entry point's behalf — so the continuation's own
    // admission attempt genuinely refuses.
    let (_ctx, guard) = rt
        .run_supervisor()
        .begin("occupant", false)
        .expect("the sole slot is free before this test occupies it");

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;

    assert!(
        matches!(result, Err(OpenCompanyError::WorkflowRunLimit { .. })),
        "resolve_approval must propagate the continuation's capacity refusal: {result:?}"
    );
    assert_eq!(
        runner.started(),
        1,
        "the refused continuation must not have run"
    );
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "a capacity refusal must not retire the stash — it is the only durable record able \
         to resume this approval once a slot frees"
    );
    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "the durable approval bank must also survive the refusal"
    );
    assert!(
        rt.journal()
            .blocked_node_dispatched()
            .iter()
            .all(|t| t != &turn),
        "nothing was actually dispatched, so no dispatch marker must exist either"
    );

    // Free the slot and let a later boot's reconciliation pick the stash back
    // up — the retry this fix exists to make possible.
    drop(guard);
    rt.reconcile_stranded_blocked_nodes().await;

    assert_eq!(
        runner.started(),
        2,
        "once capacity frees, reconciliation redeems the approval that survived the refusal"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "the stash is retired once the retried continuation actually runs"
    );
}

/// Issue #1825 (P1 follow-up, found by chatgpt-codex-connector): a
/// `record_blocked_node_dispatched` write that genuinely fails must abort the
/// launch, not warn and proceed unmarked.
///
/// Before this fix, a failed dispatch-marker write only logged a warning and
/// still called `spawn_admitted` — so a crash between that unmarked launch
/// and `BlockedNodeReleased` landing left a run genuinely in flight with
/// nothing durable saying so, and `reconcile_stranded_blocked_nodes` would
/// dispatch it a second time. [`FailNJournalStore`] forces the marker's own
/// append to fail for real (as opposed to [`GatedJournalStore`]'s crash-race
/// simulation), so this proves the call site's actual reaction: the run must
/// not launch, and the stash and its approval must stay exactly as durably
/// retryable as they were before the attempt — the same shape
/// `a_capacity_refusal_keeps_the_stash_for_a_later_retry` already proves for
/// the concurrency-ceiling refusal.
///
/// # Why this stops at "durably retryable" rather than driving a retry
///
/// `record_blocked_node_dispatched` (like every sibling `record_*` on this
/// journal — `record_blocked_node_stashed`, `record_blocked_node_approved`)
/// inserts into its in-memory mirror **before** attempting the durable
/// append, and does not rebuild that state on `main` from anything but a
/// process-inheriting rebuild or a fresh boot's replay. So this specific
/// process's `blocked_node_dispatched()` mirror is left saying `turn` is
/// dispatched even though the append that would have made it true just
/// failed — calling `reconcile_stranded_blocked_nodes` again in the *same*
/// process would read that stale `true`, treat this exactly like the
/// crash-race case `reconciliation_does_not_redispatch_a_node_whose_dispatch_already_landed`
/// proves correct, and retire the stash without ever having dispatched it —
/// reintroducing the loss this whole fix exists to close, through a path
/// none of these three P1s named. It is not reachable on `main` today
/// because `reconcile_stranded_blocked_nodes` has exactly one call site,
/// at boot, against a `RuntimeJournal` built fresh from replaying what is
/// genuinely on disk — which correctly excludes this failed write. This test
/// stops short of driving a real second boot (which would mean sharing a
/// `FailNJournalStore` across two full `RuntimeBuilder::build()` calls and
/// re-wiring a second harness pool) and instead asserts the boundary that
/// finding actually sits on: the durable, on-disk facts a genuine boot's
/// replay would see are correct. A same-process `reconcile_stranded_blocked_nodes`
/// call staying safe requires that in-memory mirror to be exactly as
/// truthful as the store — which is a pre-existing, documented property of
/// this journal (see `record_blocked_node_released`'s own doc comment) that
/// these three findings did not ask this round to change, so it is flagged
/// here rather than silently patched.
#[tokio::test]
async fn a_failed_dispatch_marker_write_aborts_the_launch_instead_of_launching_unmarked() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_failing_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeDispatched",
        1,
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;

    assert!(
        matches!(result, Err(OpenCompanyError::Store(_))),
        "resolve_approval must propagate the dispatch marker's write failure: {result:?}"
    );
    assert_eq!(
        runner.started(),
        1,
        "a failed marker write must not launch the continuation unmarked"
    );
    assert!(
        rt.blocked_nodes().is_armed(&turn),
        "the failed attempt must not retire the stash — it is still the only durable record \
         able to resume this approval"
    );
    assert!(
        rt.blocked_nodes()
            .approved_turns()
            .contains(&turn.to_string()),
        "the durable approval bank must also survive the failed dispatch attempt"
    );
    assert!(
        !store_durably_has(&store, rt.id(), "BlockedNodeDispatched", &turn).await,
        "nothing was actually dispatched, so no dispatch marker must be durably recorded — a \
         real reboot's replay, which is the only thing that reads this journal's in-memory \
         mirror back out at a point where it can matter, sees exactly this and would retry"
    );
    assert!(
        store_durably_has(&store, rt.id(), "BlockedNodeApproved", &turn).await,
        "the durable approval fact must have landed before the failed dispatch attempt, so a \
         real reboot's replay still knows this turn was approved"
    );
}

/// Issue #1825 (P1 follow-up, found by chatgpt-codex-connector): a
/// transient `record_blocked_node_approved` failure must be retried inline,
/// before `bank_blocked_node_approval` returns, rather than warned past once.
///
/// Unlike the dispatch marker above, there is no external caller who can
/// retry this write: `settle_approval`/`settle_approval_amended` reach it
/// only after `approval_gate.resolve_outcome` has already popped the id from
/// the parked set, so a re-click of "approve" on the same id short-circuits
/// to `AlreadyResolved` and never reaches this call again. Forcing exactly
/// two failures — one fewer than the bounded retry allows — proves the write
/// still lands, and the operator-visible outcome (the receipt, and the
/// continuation it dispatches) is unaffected by the transient blip.
#[tokio::test]
async fn a_transient_approval_bank_failure_is_retried_before_giving_up() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_failing_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeApproved",
        2,
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);
    assert_eq!(runner.started(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;

    assert!(
        result.is_ok(),
        "two transient failures are within the bounded retry — the resolve must still \
         succeed: {result:?}"
    );
    assert_eq!(
        runner.started(),
        2,
        "the continuation still dispatches once the retried bank succeeds"
    );
    assert!(
        store_durably_has(&store, rt.id(), "BlockedNodeApproved", &turn).await,
        "the durable approval fact must have landed despite the two failed attempts that \
         preceded it — this is exactly what a boot's `reconcile_stranded_blocked_nodes` reads \
         to find a stash stranded on its last decision"
    );
}

/// Issue #1825 (P1 follow-up): the bound on
/// `bank_blocked_node_approval`'s retry is real, not merely nominal — a
/// failure that outlasts every attempt still leaves the operator's click
/// succeeding (there is nothing else useful to tell them; the verdict and
/// the grant are already durable) but the durable approval fact genuinely
/// absent, exactly the gap this same fix's doc comment names rather than
/// pretending is closed.
#[tokio::test]
async fn a_persistent_approval_bank_failure_is_not_hidden_by_the_retry() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_failing_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeApproved",
        usize::MAX,
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;

    assert!(
        result.is_ok(),
        "a persistently failing durable bank must not fail the resolve itself — the verdict \
         and the grant are already durable by this point: {result:?}"
    );
    assert_eq!(
        runner.started(),
        2,
        "the live process still dispatches the continuation from its in-memory state \
         regardless of whether the durable bank landed"
    );
    assert!(
        !store_durably_has(&store, rt.id(), "BlockedNodeApproved", &turn).await,
        "a genuinely persistent failure is not something the bounded retry can paper over — \
         the durable fact is honestly absent, not silently assumed present"
    );
}

/// Issue #1825 (P1, found by chatgpt-codex-connector): the fact absent at the
/// end of the test above must not be *permanently* absent for an outage that
/// clears — `bank_blocked_node_approval`'s bounded inline loop still gives up
/// after three quick attempts (max ~150ms of backoff), and on `main` that was
/// the end of it: the caller sees `Ok` regardless (the grant and the resolved
/// journal line are already committed before this call runs), nothing
/// downstream re-attempts the write, and a restart landing anywhere after
/// this point rehydrates the stash from `blocked_stashes` with
/// `approved: false` — indistinguishable from a stash nobody ever decided —
/// so `reconcile_stranded_blocked_nodes` retires it and the operator's real
/// approval is gone for good, even though the outage that caused it had
/// already cleared by the time the process crashed.
///
/// `bank_blocked_node_approval` is actually called **twice** per resolve —
/// once inline from `settle_approval` before the follow-up cycle is even
/// spawned, and again as deliberate defense-in-depth from `continue_turn`
/// inside that follow-up cycle (see the comment at that call site). Each call
/// runs its own independent 3-attempt bounded loop, so a fixture that only
/// fails 5 appends never actually exercises the background retry this test
/// means to prove: the first call's loop burns 3 failures, the second call's
/// loop burns 2 more and then succeeds on its own third (inline) attempt —
/// recovered by the pre-existing redundant call, not by anything this P2
/// follow-up added. Fails **six** appends — exactly both call sites' combined
/// 3+3 inline attempts — so neither loop's own retries can recover it and
/// only a background retry (spawned independently by whichever call site
/// exhausts first) can land the seventh append. This test polls until the
/// fact appears rather than asserting an intermediate absence: with two
/// independent bank calls in play, the moment each one's background task gets
/// its first real poll is not something this test should hardcode a timing
/// assumption about. Pre-fix this test times out — nothing on `main` ever
/// retries past either call's third attempt, so the fact never appears no
/// matter how long the poll waits.
#[tokio::test]
async fn a_recovered_approval_bank_failure_lands_via_the_background_retry() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_failing_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeApproved",
        6,
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;
    assert!(
        result.is_ok(),
        "the resolve itself is unaffected by the bank's durable write failing — the same \
         guarantee the test above already establishes: {result:?}"
    );
    assert_eq!(
        runner.started(),
        2,
        "the live process still dispatches the continuation from its in-memory state, \
         regardless of the durable bank"
    );
    // Poll rather than asserting an immediate absence or a single fixed
    // sleep: two independent bank calls each spawn their own background
    // retry on exhaustion (see the doc above), so which one's first backoff
    // (200ms, 400ms, 800ms, ...) elapses first is not something this test
    // should hardcode a timing assumption about — only that the fact
    // eventually lands.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if store_durably_has(&store, rt.id(), "BlockedNodeApproved", &turn).await {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the background retry never landed the durable fact within 10s — on `main` this \
             is exactly the permanent loss the P2 follow-up closes: an outage that already \
             cleared still strands the approval forever because nothing ever retries past the \
             inline loop's third attempt"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Issue #1825 (P2, second follow-up — found by chatgpt-codex-connector): a
/// background approval-bank retry that lands *after* its own turn's stash was
/// already released must not leave `turn` resurrected in
/// `blocked_node_approvals` forever.
///
/// This reuses the fixture directly above unchanged, because the race it
/// closes is not an exotic timing edge — it is the *ordinary* shape of the
/// sibling test's own recovery. Dispatch never waits on either
/// `bank_blocked_node_approval` call site (`runner.started()` above already
/// proves the run launches "regardless of the durable bank"), and it reliably
/// releases the stash well inside the ~150ms both inline loops burn before a
/// background retry's first 200ms backoff even elapses. So by the time the
/// sibling test's poll observes the late write land, the stash is already
/// gone — precisely the moment `spawn_background_approval_bank_retry`'s
/// cleanup must fire.
///
/// Proven red by reverting only the `record_blocked_node_released` cleanup
/// call the P2 second follow-up adds (not the write itself, which the sibling
/// test already pins as required): pre-fix, `blocked_node_approvals()` keeps
/// `turn` forever once the late write lands, because nothing ever appends a
/// second `BlockedNodeReleased` to retire the resurrection — and, on a future
/// boot, replay reaches the identical stale state.
#[tokio::test]
async fn a_late_landing_approval_bank_write_retires_its_own_resurrection() {
    let home = seed_home();
    let (rt, store, runner) = runtime_with_failing_journal_store(
        home.path(),
        vec![
            Turn::Call {
                tool: "shell",
                args: json!({ "command": "echo hi" }),
            },
            Turn::Say("I was refused, so I stopped."),
            Turn::Say("Done."),
        ],
        "BlockedNodeApproved",
        6,
    )
    .await;

    let run_id = cold_run(&rt).await;
    let cards = cards_for(&rt, &run_id);
    assert_eq!(cards.len(), 1);

    let turn = workflow_node_turn_key(&run_id, NODE);

    let result = rt
        .resolve_approval(&cards[0], Verdict::Approve, operator())
        .await;
    assert!(
        result.is_ok(),
        "the resolve itself is unaffected by the bank's durable write failing: {result:?}"
    );
    assert_eq!(
        runner.started(),
        2,
        "the live process still dispatches the continuation from its in-memory state, \
         regardless of the durable bank"
    );
    assert!(
        !rt.blocked_nodes().is_armed(&turn),
        "precondition this test depends on: dispatch already released this turn's stash \
         before either background retry's own delayed write could have landed — the exact \
         ordering that makes a late-landing write a resurrection instead of a legitimate bank"
    );

    // Same poll shape as the sibling test: which retry's backoff lands the
    // seventh, recovering append is not something to hardcode a timing
    // assumption about.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if store_durably_has(&store, rt.id(), "BlockedNodeApproved", &turn).await {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the background retry never landed the durable fact within 10s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // The write landed after release (the precondition above already proved
    // it), so the retry's own cleanup must retire the resurrection. Poll
    // rather than assert immediately: the cleanup is a second awaited append,
    // made after the one the loop above just observed.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if !rt.journal().blocked_node_approvals().contains(&turn) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "a late-landing approval bank write left `turn` resurrected in \
             blocked_node_approvals with nothing left to release it — exactly the stale \
             durable key this fix exists to retire"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Issue #1825 (PR #1825 review) — a retried amend against an id **already**
/// resolved must not durably bank the *turn* as approved.
///
/// `settle_approval_amended` hardcodes `Verdict::Approve` for its inline bank
/// call, unlike the plain path (`settle_approval`), which passes the actual
/// operator verdict through and additionally short-circuits on
/// `ResolveOutcome::NotParked` before ever reaching the bank. Pre-fix, the
/// amend path had neither guard: a second amend call against an id already
/// resolved — here, resolved by a plain **deny** — reads
/// `ResolveOutcome::NotParked` (nothing left parked to overlay onto) but still
/// ran the bank unconditionally with the hardcoded `Approve`. `origins`
/// intentionally outlives resolution (see
/// `approval_origins_outlive_resolution_and_expiry_and_survive_reload` in
/// `runtime/journal.rs`), so `bank_blocked_node_approval` still resolves the
/// denied id back to its turn and durably marks that turn approved — even
/// though nothing this call did actually approved anything, and the node's
/// sibling card is still undecided. A restart before the sibling's own
/// (denial) decision would then rehydrate the turn as approved from nothing
/// but this spurious bank, and dispatch a continuation no real decision ever
/// authorized.
///
/// Proven red by re-adding the old unconditional
/// `self.rt.bank_blocked_node_approval(id, Verdict::Approve).await;` in place
/// of the `if executed.is_some() { .. }` gate this fix adds.
#[tokio::test]
async fn a_retried_amend_on_an_already_denied_card_does_not_bank_the_turn() {
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

    let turn = workflow_node_turn_key(&run_id, NODE);

    // A real decision on the first card: deny. `bank_blocked_node_approval`
    // correctly no-ops on a non-`Approve` verdict, so this alone must not
    // bank the turn.
    resolve_and_settle(&rt, &cards[0], Verdict::Deny).await;
    assert!(
        !rt.blocked_nodes()
            .peek(&turn)
            .expect("the node's stash is still live — one card is still undecided")
            .approved,
        "precondition: a denied card must not bank the turn as approved"
    );

    // A retried amend against the SAME, already-resolved id. The parked
    // effect is already gone (the deny above removed it), so this is the
    // `ResolveOutcome::NotParked` branch on the amend path — nothing new is
    // approved by this call.
    rt.resolve_approval_amended(
        &cards[0],
        json!({ "command": "echo one amended" }),
        operator(),
    )
    .await
    .expect("a resolve against an already-resolved id still returns Ok, not an error");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert!(
        !rt.blocked_nodes()
            .peek(&turn)
            .expect("the stash is still live — the sibling card is still undecided")
            .approved,
        "a retried amend on an already-denied id must not bank the turn as approved — \
         nothing about this call actually approved anything"
    );
    assert!(
        !rt.journal().blocked_node_approvals().contains(&turn),
        "no BlockedNodeApproved record for this turn — the retried amend approved nothing, \
         so nothing should have been durably banked"
    );

    // The node's actual last decision: the sibling card, also denied.
    resolve_and_settle(&rt, &cards[1], Verdict::Deny).await;

    assert_eq!(
        runner.started(),
        1,
        "both of this node's calls were denied — no continuation is owed, even though a \
         spurious retried amend once tried to bank the turn approved"
    );
}
