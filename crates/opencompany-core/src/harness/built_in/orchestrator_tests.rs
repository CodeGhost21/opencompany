use super::*;
use crate::ports::tasks::TaskTitle;
use std::sync::Mutex as StdMutex;

use crate::ports::runs::RunStatus;
use crate::ports::types::{CompanyRecord, CompanySummary, LedgerEntry};

fn agent(id: &str, tier: Option<&str>) -> ManifestAgent {
    ManifestAgent {
        provider: None,
        global: false,
        id: id.to_string(),
        role: "Role".to_string(),
        name: None,
        description: None,
        tier: tier.map(str::to_string),
        harness: None,
        tools: None,
        delegates_to: Vec::new(),
        context: None,
        budget_usd_daily: None,
        prompt: None,
        prompt_files: Vec::new(),
        prompt_files_resolved: Vec::new(),
        classes: Vec::new(),
        ledgers: None,
        can_declare_ledgers: true,
        model: None,
    }
}

// ── Issue #419: the cap is announced, not silently applied ─────────────

// ── Issue #453: no claim, no delegation ────────────────────────────────

// ── Issue #186 part b: the lifecycle tools ─────────────────────────────

/// A company with a `strategy` desk led by a roster teammate, an
/// `archive` desk nobody on the roster sits on, and a `writer` teammate who
/// is *not* a desk — the exact shape issue #272 was observed on.
fn desks_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer"]

[[group_chat]]
id = "archive"
name = "Archive desk"
members = ["nobody"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

fn desk_tool(record: CompanyRecord, queue: &DelegationQueue) -> DelegateToDeskTool {
    let company = record.id.clone();
    DelegateToDeskTool::new(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
    )
}

// --- Recursive desk delegation (issue #176) -----------------------------

/// A three-desk record where two desks have roster leads, so a member of one
/// can be given an allowlist that admits one desk and not another.
fn nested_desks_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"
delegates_to = ["research"]

[[agent]]
id = "analyst"
role = "Analyst"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer"]

[[group_chat]]
id = "research"
name = "Research desk"
members = ["analyst"]

[[group_chat]]
id = "legal"
name = "Legal desk"
members = ["ceo"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

/// The `writer`'s copy of `delegate_to_desk`: allowed `research` only.
fn member_desk_tool(record: CompanyRecord, queue: &DelegationQueue) -> DelegateToDeskTool {
    let company = record.id.clone();
    DelegateToDeskTool::for_member(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["research".to_string()],
        },
    )
}

/// A store that cannot answer, so the grounding read has nothing to check
/// the target against.
struct BrokenStore;

#[async_trait::async_trait]
impl CompanyStore for BrokenStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Err(crate::OpenCompanyError::Store("store is down".to_string()))
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

// ── delegate_to_teammate at the tool boundary (issue #884) ──────────────

/// A company whose `strategy` desk has THREE members, so its lead has peers
/// to reach — the shape D1 was observed on — plus an `analyst` on a desk the
/// lead's `delegates_to` permits and a `legal_counsel` on one it does not.
fn peers_record(id: &CompanyId) -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"
tier = "orchestrator"

[[agent]]
id = "writer"
role = "Writer"
delegates_to = ["research"]

[[agent]]
id = "editor"
role = "Editor"

[[agent]]
id = "analyst"
role = "Analyst"

[[agent]]
id = "legal_counsel"
role = "Counsel"

[[group_chat]]
id = "strategy"
name = "Strategy desk"
members = ["writer", "editor"]

[[group_chat]]
id = "research"
name = "Research desk"
members = ["analyst"]

[[group_chat]]
id = "legal"
name = "Legal desk"
members = ["legal_counsel"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..seeded_record(id)
    }
}

/// `writer`'s copy of the teammate tool: a desk lead with one peer on its
/// own desk and a `research` allowlist.
fn member_teammate_tool(
    record: CompanyRecord,
    queue: &DelegationQueue,
) -> DelegateToTeammateTool {
    let company = record.id.clone();
    DelegateToTeammateTool::for_member(
        queue.clone(),
        company,
        Arc::new(MemStore::seeded(record)) as Arc<dyn CompanyStore>,
        MemberScope {
            member: "writer".to_string(),
            delegates_to: vec!["research".to_string()],
        },
    )
}

// --- add_agent (issue #71) ----------------------------------------------

/// An in-memory `CompanyStore` so `AddAgentTool` can be exercised without a
/// filesystem, mirroring `crate::server::ops::team`'s `add_member` write
/// path (load → push overlay → save).
#[derive(Default)]
struct MemStore {
    record: StdMutex<Option<CompanyRecord>>,
}

impl MemStore {
    fn seeded(record: CompanyRecord) -> Self {
        Self {
            record: StdMutex::new(Some(record)),
        }
    }
}

#[async_trait::async_trait]
impl CompanyStore for MemStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(self.record.lock().unwrap().clone())
    }
    async fn save(&self, record: &CompanyRecord) -> crate::Result<()> {
        *self.record.lock().unwrap() = Some(record.clone());
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

fn empty_manifest() -> crate::company::CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n").expect("valid manifest")
}

fn seeded_record(id: &CompanyId) -> CompanyRecord {
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: id.clone(),
        manifest: empty_manifest(),
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
    }
}

/// A minter scoped to part of the company grant, for the #619 tests below.
/// `minter_tools` is the line it declares; `minter_grants` is that line
/// already narrowed by the company `allow` — what `build_agent` hands the
/// tool.
fn scoped_add_agent(company: CompanyId, store: Arc<dyn CompanyStore>) -> AddAgentTool {
    AddAgentTool::new(
        company,
        store,
        "ceo".to_string(),
        Some(vec!["workspace".to_string()]),
        vec!["workspace".to_string()],
    )
}

// ---- run_workflow (issue #67) ----

/// A valid trigger → agent → output graph, mirroring the REST route's fixture.
const DEMO_WF: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
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
"#;

/// A [`WorkflowRunner`] test double: records the ids it was asked to run and
/// returns a canned [`WorkflowRun`].
struct StubRunner {
    calls: Arc<Mutex<Vec<String>>>,
    run: WorkflowRun,
}

impl StubRunner {
    fn new(run: WorkflowRun) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            run,
        }
    }

    fn empty() -> Self {
        Self::new(WorkflowRun {
            output: Value::Null,
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

#[async_trait::async_trait]
impl WorkflowRunner for StubRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        workflow: &WorkflowFile,
        _input: Value,
        _ctx: &crate::ports::WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        self.calls.lock().unwrap().push(workflow.id.clone());
        Ok(self.run.clone())
    }
}

/// A [`WorkflowRunner`] test double whose `run` always returns `Err` — the
/// engine-failed shape issue #1865's review comment 3877185396 flagged as
/// silent: `RunWorkflowTool`'s `Ok(Err(err))` arm journaled a finish but
/// filed no `workflow_run_failed` notification, unlike the console run
/// route, the cron scheduler, and the approval-resume path, which all
/// file one through `WorkflowSpawn`.
struct FailingRunner;

#[async_trait::async_trait]
impl WorkflowRunner for FailingRunner {
    async fn run(
        &self,
        _company: &CompanyId,
        _workflow: &WorkflowFile,
        _input: Value,
        _ctx: &crate::ports::WorkflowRunContext,
    ) -> crate::Result<WorkflowRun> {
        Err(crate::error::OpenCompanyError::Harness(
            "the engine blew up".to_string(),
        ))
    }
}

/// Writes `DEMO_WF` to `<dir>/workflows/demo.toml`.
fn seed_demo_workflow(dir: &std::path::Path) {
    let wf = dir.join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("demo.toml"), DEMO_WF).unwrap();
}

// ---- create_workflow (issue #112) ----

/// A record with an `assistant` roster agent so an `agent`-node graph passes
/// the roster cross-check inside the create core.
fn record_with_assistant(company: &CompanyId) -> CompanyRecord {
    let manifest: crate::company::CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n",
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: company.clone(),
        manifest,
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
    }
}

/// The canonical happy graph the create tool accepts (camelCase body).
fn greeter_body() -> Value {
    json!({
        "id": "greeter",
        "name": "Greeter",
        "description": "Says hi.",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "worker", "kind": "agent", "name": "Worker", "agent": "assistant" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [
            { "from": "start", "to": "worker" },
            { "from": "worker", "to": "done", "label": "ok" }
        ]
    })
}

/// Like [`record_with_assistant`], but the company `[tools].allow` grants the
/// `web` namespace so a `web_fetch` `tool_call` clears the author-time grant
/// gate under the `openhuman` build (issue #661).
fn record_granting_web(company: &CompanyId) -> CompanyRecord {
    let mut record = record_with_assistant(company);
    record.manifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [\"web\"]\n[[agent]]\nid = \"assistant\"\nrole = \"Assistant\"\n",
    )
    .expect("valid manifest");
    record
}

// ---- read_run_output (issue #418) ----

/// Builds a `RunWorkflowTool` over the demo graph in `dir`, a stub runner
/// returning `run`, and the given caches — the shared setup the round-trip
/// tests need.
/// Returns the tool **and** the runner `Arc` — the handle keeps only a weak
/// reference, so the caller must hold the returned runner alive for the
/// duration of the test or the run tool reports "no runner wired".
fn run_tool_over(
    dir: &std::path::Path,
    run: WorkflowRun,
    refs: WorkflowRefQueue,
    cache: RunOutputCache,
) -> (RunWorkflowTool, Arc<dyn WorkflowRunner>) {
    let runner: Arc<dyn WorkflowRunner> = Arc::new(StubRunner::new(run));
    let handle = WorkflowRunnerHandle::default();
    handle.set(&runner);
    let tool = RunWorkflowTool::new(
        CompanyId::new("acme"),
        Some(dir.to_path_buf()),
        Arc::new(MemStore::default()),
        handle,
        crate::runtime::RunSupervisor::default(),
        None,
        refs,
        cache,
        None,
    );
    (tool, runner)
}

// -----------------------------------------------------------------------
// Issue #661: the queue is scoped per claimant
// -----------------------------------------------------------------------

/// A card, titled so a drain can be identified by what it carried.
fn card(title: &str) -> Delegation {
    Delegation::SpawnTask {
        title: title.to_string(),
        note: None,
        assignee: None,
    }
}

fn hand_off() -> Delegation {
    Delegation::DelegateToDesk {
        desk: "design".to_string(),
        instruction: "have a look".to_string(),
    }
}

fn titles(drained: Vec<Delegation>) -> Vec<String> {
    drained
        .into_iter()
        .map(|d| match d {
            Delegation::SpawnTask { title, .. } => title,
            other => panic!("expected a card, got {other:?}"),
        })
        .collect()
}

fn stage(queue: &DelegationQueue, d: Delegation) -> Staged {
    queue.push_within_cap(d, MAX_DELEGATIONS_PER_TURN, NO_DEPTH_BOUND)
}

// -----------------------------------------------------------------------
// Issue #1859: the execution-state read trio (`list_tasks` / `read_task` /
// `read_run`) and `query_company`'s `## Board` section.
// -----------------------------------------------------------------------

/// A minimal board card, for fixtures below. Named `task_card` rather than
/// `card` — that name is already the `Delegation` fixture above.
fn task_card(id: &str, title: &str, column: &str, assignee: &str) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        title: TaskTitle::authored(title),
        note: None,
        column: column.to_string(),
        priority: "medium".to_string(),
        assignee: assignee.to_string(),
        updated_at_millis: 1,
        origin: crate::ports::TaskOrigin::new(None, None),
        parent_task_id: None,
        output: None,
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: crate::ports::tasks::TaskDeliverable::Once,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
        origin_message_seq: None,
        bounced: None,
    }
}

/// A task board that cannot answer, so a read failure never collapses
/// into an empty or missing board.
struct BrokenTaskStore;

#[async_trait]
impl TaskStore for BrokenTaskStore {
    async fn list(&self, _company: &CompanyId) -> crate::Result<Vec<TaskRecord>> {
        Err(OpenCompanyError::Store(
            "simulated board read failure".into(),
        ))
    }
    async fn upsert(&self, _company: &CompanyId, _task: &TaskRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn update_if_column(
        &self,
        _company: &CompanyId,
        _task: &TaskRecord,
        _observed: &TaskRecord,
        _expected_column: &str,
    ) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
}

/// A run store that cannot answer, so a run-history read failure never
/// collapses into "no attempts" or a missing run — the same distinction
/// `list_tasks`/`read_task`'s board read already makes for [`TaskStore`].
struct BrokenRunStore;

#[async_trait]
impl RunStore for BrokenRunStore {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by these tests")
    }
    async fn get_run(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-store read failure".into(),
        ))
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-history read failure".into(),
        ))
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by these tests")
    }
}

/// A run store that answers `get_run` but never `list_runs`, to isolate
/// [`ReadRunTool`]'s agent-attempt lookup from its journal fallback.
struct FailingGetRun;

#[async_trait]
impl RunStore for FailingGetRun {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by these tests")
    }
    async fn get_run(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<RunRecord>> {
        Err(OpenCompanyError::Store(
            "simulated run-store read failure".into(),
        ))
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        unimplemented!("not exercised by these tests")
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by these tests")
    }
}

/// An event log that always fails `read_from`, to prove
/// [`ReadRunTool`]'s workflow-run fallback distinguishes a journal read
/// failure from a genuinely absent run.
struct BrokenEventLog;

#[async_trait]
impl EventLog for BrokenEventLog {
    async fn append(&self, _id: &CompanyId, _event: CompanyEvent) -> crate::Result<EventSeq> {
        unreachable!("read_run only reads")
    }
    async fn read_from(
        &self,
        _id: &CompanyId,
        _seq: EventSeq,
        _limit: usize,
    ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
        Err(OpenCompanyError::Store(
            "simulated event-log read failure".into(),
        ))
    }
    fn subscribe(
        &self,
        _id: &CompanyId,
    ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem> {
        Box::pin(futures::stream::empty())
    }
}

/// An artifact store that cannot answer, so an output-surface read
/// failure never collapses into "nothing published".
struct BrokenArtifactStore;

#[async_trait]
impl ArtifactStore for BrokenArtifactStore {
    async fn list(
        &self,
        _company: &CompanyId,
        _task_id: Option<&str>,
    ) -> crate::Result<Vec<crate::ports::artifacts::ArtifactRecord>> {
        Err(OpenCompanyError::Store(
            "simulated artifact-store read failure".into(),
        ))
    }
    async fn get(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<crate::ports::artifacts::ArtifactRecord>> {
        unimplemented!("not exercised by these tests")
    }
    async fn upsert(
        &self,
        _company: &CompanyId,
        _artifact: &crate::ports::artifacts::ArtifactRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by these tests")
    }
    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        unimplemented!("not exercised by these tests")
    }
}

// -- FAIL-axis: cross-tenant reach, ungrounded targets, unbounded growth -

/// A `RunStore` that genuinely partitions by company — the shape every
/// real backend promises — so a lookup under one company can never answer
/// with a row filed under another.
struct TenantScopedRunStore {
    rows: std::sync::Mutex<Vec<RunRecord>>,
}

#[async_trait]
impl RunStore for TenantScopedRunStore {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<RunRecord> {
        unimplemented!("not exercised by this test")
    }
    async fn get_run(&self, company: &CompanyId, id: &str) -> crate::Result<Option<RunRecord>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| &r.company == company && r.id == id)
            .cloned())
    }
    async fn put_run(&self, _company: &CompanyId, _run: &RunRecord) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &RunFilter,
    ) -> crate::Result<Vec<RunRecord>> {
        unimplemented!("not exercised by this test")
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        unimplemented!("not exercised by this test")
    }
}

fn tenant_run(company: &str, id: &str) -> RunRecord {
    RunRecord {
        id: id.to_string(),
        company: CompanyId::new(company),
        task_id: None,
        chat_id: None,
        agent_id: "ceo".to_string(),
        attempt: 1,
        status: crate::ports::runs::RunStatus::Running,
        trigger_event_seq: None,
        thread_root: None,
        created_at_millis: 1_000,
        started_at_millis: None,
        finished_at_millis: None,
        error: None,
        usage: crate::ports::types::TokenUsage::default(),
        step_count: 0,
        workflow_run_id: None,
        node_id: None,
    }
}

/// A `CompanyStore` that genuinely partitions by company — unlike
/// `MemStore`, which ignores the `id` argument and answers for whichever
/// company it was seeded with regardless of who asks. Needed to prove
/// `spawn_task`'s grounding actually scopes its lookup to `self.company`
/// rather than happening to work because every test fixture only ever
/// holds one company's record.
struct TenantScopedCompanyStore {
    records: std::collections::HashMap<String, CompanyRecord>,
}

#[async_trait::async_trait]
impl CompanyStore for TenantScopedCompanyStore {
    async fn load(&self, id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(self.records.get(id.as_ref()).cloned())
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        unimplemented!("not exercised by this test")
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}

/// A store that yields between reading a record and writing it back, so two
/// concurrent `add_agent` calls genuinely interleave their load → push →
/// save cycle rather than each running to completion uncontended.
struct YieldingStore {
    record: StdMutex<Option<CompanyRecord>>,
}

#[async_trait::async_trait]
impl CompanyStore for YieldingStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        let snapshot = self.record.lock().expect("record").clone();
        tokio::task::yield_now().await;
        Ok(snapshot)
    }
    async fn save(&self, record: &CompanyRecord) -> crate::Result<()> {
        tokio::task::yield_now().await;
        *self.record.lock().expect("record") = Some(record.clone());
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Ok(())
    }
}


#[path = "orchestrator_tests_part1.rs"]
mod tests_part1;
#[path = "orchestrator_tests_part2.rs"]
mod tests_part2;
#[path = "orchestrator_tests_part3.rs"]
mod tests_part3;
#[path = "orchestrator_tests_part4.rs"]
mod tests_part4;
#[path = "orchestrator_tests_part5.rs"]
mod tests_part5;
#[path = "orchestrator_tests_part6.rs"]
mod tests_part6;
#[path = "orchestrator_tests_part7.rs"]
mod tests_part7;
#[path = "orchestrator_tests_part8.rs"]
mod tests_part8;
#[path = "orchestrator_tests_part9.rs"]
mod tests_part9;
#[path = "orchestrator_tests_part10.rs"]
mod tests_part10;
#[path = "orchestrator_tests_part11.rs"]
mod tests_part11;
