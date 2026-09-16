use super::*;
use crate::ports::tasks::TaskTitle;

use tinyinference::Result as TaResult;
use tinyinference::message::Message;
use tinyinference::model::{ChatModel, ModelRequest, ModelResponse};

use crate::company::CompanyManifest;
use crate::harness::provider::{HarnessModel, MockProvider};
use crate::hivemind::episode::HiveTurnRunner;
use crate::hivemind::referral::HiveReferralRunner;
use crate::ports::brain::CycleHost;
// Issue #301: every lifecycle return now lands in To-do (the `backlog` pool
// is gone), so these assertions read the const rather than a literal.
use crate::ports::tasks::{COLUMN_IN_REVIEW, COLUMN_PAUSED, COLUMN_TODO};
use crate::ports::types::{
    ApprovalId, CompanyId, ContextOp, ContextOpResult, Effect, EffectDisposition, OverlayAgent,
    ToolCall, ToolResult,
};
use crate::store::{FsCompanyStore, FsContextStore, FsOps};

/// A `CycleHost` that auto-executes anything the brain asks for and swallows
/// anything it parks; used by every test that isn't about approvals.
#[derive(Default)]
struct NoopHost;

#[async_trait]
impl CycleHost for NoopHost {
    async fn call_tool(&self, _call: ToolCall) -> Result<ToolResult> {
        Ok(ToolResult {
            ok: true,
            output: serde_json::Value::Null,
        })
    }
    async fn context_op(&self, _op: ContextOp) -> Result<ContextOpResult> {
        Ok(ContextOpResult::Text(String::new()))
    }
    async fn emit_effect(&self, _effect: Effect) -> Result<EffectDisposition> {
        Ok(EffectDisposition::Executed)
    }
    async fn park_effect(&self, _effect: Effect) -> Result<ApprovalId> {
        Ok(ApprovalId::new("appr-parked"))
    }
}

/// A `CycleHost` that records every effect parked for approval, so the
/// approval drain can be asserted on (issue #172). Anything else it does is
/// inert.
#[derive(Default)]
struct ParkingHost {
    parked: std::sync::Mutex<Vec<Effect>>,
}

impl ParkingHost {
    /// The effects parked through `park_effect`, in order.
    fn parked(&self) -> Vec<Effect> {
        self.parked.lock().expect("parked").clone()
    }
}

#[async_trait]
impl CycleHost for ParkingHost {
    async fn call_tool(&self, _call: ToolCall) -> Result<ToolResult> {
        Ok(ToolResult {
            ok: true,
            output: serde_json::Value::Null,
        })
    }
    async fn context_op(&self, _op: ContextOp) -> Result<ContextOpResult> {
        Ok(ContextOpResult::Text(String::new()))
    }
    async fn emit_effect(&self, _effect: Effect) -> Result<EffectDisposition> {
        panic!("an approval request must be parked, never re-evaluated as an effect");
    }
    async fn park_effect(&self, effect: Effect) -> Result<ApprovalId> {
        let mut parked = self.parked.lock().expect("parked");
        parked.push(effect);
        Ok(ApprovalId::new(format!("appr-{}", parked.len())))
    }
}

fn record() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Runs Acme."
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

fn brain_over_mock(dir: &std::path::Path) -> HarnessBrain {
    brain_over_mock_with(dir, record())
}

/// [`brain_over_mock`] over a chosen record, so a test can vary the roster
/// (and its `[[harness]]` block) without restating the whole deps literal.
fn brain_over_mock_with(dir: &std::path::Path, record: CompanyRecord) -> HarnessBrain {
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: Some(Arc::new(FsOps::new(dir))),
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record)
}

fn request(events: Vec<CompanyEvent>) -> CycleRequest {
    CycleRequest {
        cycle_id: "cycle-1".to_string(),
        company_id: CompanyId::new("acme"),
        events,
        event_seqs: Vec::new(),
        policy: None,
    }
}

// --- Task dispatch ------------------------------------------------------

use crate::ports::TaskStore;

/// A two-agent record so assignee routing has somewhere to route.
fn record_two() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Runs Acme."

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds it."
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

/// A brain wired to a real task store (shared handle returned for seeding /
/// asserting), over the offline mock provider.
fn brain_with_tasks(dir: &std::path::Path) -> (HarnessBrain, Arc<FsOps>) {
    brain_with_tasks_notified(dir, false)
}

/// As [`brain_with_tasks`], but with the journal wired too — so a test can
/// seed a card, settle it, and read back the `DeskTaskCompleted` the settle
/// wrote (issue #1890 B). [`FsOps`] is not an [`EventLog`], so the log is a
/// second store over the same directory.
fn brain_with_tasks_and_events(
    dir: &std::path::Path,
) -> (HarnessBrain, Arc<FsOps>, Arc<dyn crate::ports::EventLog>) {
    let events: Arc<dyn crate::ports::EventLog> = Arc::new(crate::store::FsEventLog::new(dir));
    let (brain, tasks) = brain_with_tasks_notified_logging(dir, false, Some(events.clone()));
    (brain, tasks, events)
}

/// Same as [`brain_with_tasks`], but also wires the task store as the
/// notification store (issue #1865, PR #1883 review comment 3878668326):
/// [`FsOps`] implements both, so a test can seed a card, drive a cycle,
/// and then read back any `dispatch_failed` row a refusal filed.
fn brain_with_tasks_notified(dir: &std::path::Path, notify: bool) -> (HarnessBrain, Arc<FsOps>) {
    brain_with_tasks_notified_logging(dir, notify, None)
}

/// As [`brain_with_tasks_notified`], but with the journal optionally wired
/// — so a test can seed a card, settle it, and read back the
/// `DeskTaskCompleted` the settle wrote (issue #1890 B). `None` is the
/// shape every caller had before, and `HarnessBrain` holds its deps behind
/// an `Arc`, so this has to be a build-time choice rather than a mutation
/// after the fact.
fn brain_with_tasks_notified_logging(
    dir: &std::path::Path,
    notify: bool,
    events: Option<Arc<dyn crate::ports::EventLog>>,
) -> (HarnessBrain, Arc<FsOps>) {
    let tasks = Arc::new(FsOps::new(dir));
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: if notify { Some(tasks.clone()) } else { None },
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: Some(Arc::new(FsOps::new(dir))),
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(tasks.clone()),
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_two()),
        tasks,
    )
}

/// A model whose every call fails with the exact wire shape
/// `is_top_level_budget_exhausted` recognises (issue #1846 review, Codex
/// #3864988168) — the same body `a_top_level_budget_exhaustion_pauses_
/// gracefully_and_parks_a_reissue_marker` in `mod.rs` scripts, reused here
/// to prove the DISPATCHED-CARD path settles on the pause rather than
/// completing.
struct BudgetExhaustedProvider;

#[async_trait]
impl ChatModel<()> for BudgetExhaustedProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Err(tinyinference::Error::Model(
            "USER_INSUFFICIENT_CREDITS: insufficient budget for this account — add credits \
             to continue"
                .to_string(),
        ))
    }
}

impl HarnessModel for BudgetExhaustedProvider {
    fn telemetry_provider_id(&self) -> String {
        "scripted".to_string()
    }
}

/// As [`brain_with_tasks`], but every model call fails with a
/// budget-exhausted body (issue #1846 review, Codex #3864988168) —
/// otherwise byte-identical, so the only variable a test built on this
/// exercises is how the dispatch path reacts to that one failure shape.
fn brain_with_tasks_and_budget_exhausted_provider(
    dir: &std::path::Path,
) -> (HarnessBrain, Arc<FsOps>) {
    let tasks = Arc::new(FsOps::new(dir));
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(BudgetExhaustedProvider),
        provider_slug: "scripted".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: Some(Arc::new(FsOps::new(dir))),
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(tasks.clone()),
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_two()),
        tasks,
    )
}

/// As [`brain_with_tasks`], but the roster also carries an `eng` desk led by
/// the engineer — the shape `delegate_to_desk` writes into a card's
/// `assignee` (issue #205).
fn brain_with_desk_tasks(dir: &std::path::Path) -> (HarnessBrain, Arc<FsOps>) {
    let (brain, tasks) = brain_with_tasks(dir);
    let group_chats = toml::from_str::<CompanyManifest>(
        r#"
[company]
name = "Acme"

[[agent]]
id = "engineer"
role = "Engineer"

[[group_chat]]
id = "eng"
name = "Engineering desk"
members = ["engineer"]
"#,
    )
    .expect("valid manifest")
    .group_chats;
    brain.mutate_record(|r| r.manifest.group_chats = group_chats);
    (brain, tasks)
}

/// As [`brain_with_tasks`], but with the artifact store wired to the same
/// [`FsOps`] handle (it implements both), so a dispatch's versioned output
/// is observable.
fn brain_with_artifacts(dir: &std::path::Path) -> (HarnessBrain, Arc<FsOps>) {
    brain_with_stores(dir, false)
}

/// As [`brain_with_artifacts`], but with the workspace store wired to the
/// same [`FsOps`] handle too (it implements all three), so issue #552's
/// dual write into the shared tree is observable.
///
/// A separate constructor rather than a change to the one above: leaving
/// `brain_with_artifacts` workspace-less is what keeps every pre-existing
/// publish test on the artifact-only path, which is the guarantee that an
/// unwired workspace behaves exactly as it did before this cell.
fn brain_with_artifacts_and_workspace(dir: &std::path::Path) -> (HarnessBrain, Arc<FsOps>) {
    brain_with_stores(dir, true)
}

fn brain_with_stores(dir: &std::path::Path, with_workspace: bool) -> (HarnessBrain, Arc<FsOps>) {
    let ops = Arc::new(FsOps::new(dir));
    let artifacts = ops.clone() as Arc<dyn crate::ports::artifacts::ArtifactStore>;
    brain_with_injected_artifacts(dir, ops, artifacts, with_workspace)
}

/// As [`brain_with_stores`], but with the artifact store supplied by the
/// caller — so a test can make `upsert` refuse and observe what the publish
/// drain did to the *tree* before it got there.
///
/// That is the only way to pin issue #552's write ordering. An ordering
/// described in a comment is not an ordering: the next refactor reorders it
/// and nothing objects.
fn brain_with_injected_artifacts(
    dir: &std::path::Path,
    ops: Arc<FsOps>,
    artifacts: Arc<dyn crate::ports::artifacts::ArtifactStore>,
    with_workspace: bool,
) -> (HarnessBrain, Arc<FsOps>) {
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: Some(Arc::new(FsOps::new(dir))),
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(ops.clone()),
        artifacts: Some(artifacts),
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: with_workspace.then(|| ops.clone() as Arc<dyn crate::ports::WorkspaceStore>),
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_two()),
        ops,
    )
}

// -- issue #552: the write ordering, proven by failure injection ---------

/// An [`ArtifactStore`](crate::ports::artifacts::ArtifactStore) that refuses
/// `upsert` from the Nth call onward, delegating everything else.
///
/// The instrument the ordering tests need: with the artifact write made to
/// fail at a chosen point, what the *tree* holds afterwards says
/// unambiguously which surface was written first.
struct FailingArtifacts {
    inner: Arc<FsOps>,
    /// How many `upsert` calls succeed before the rest refuse.
    allowed: std::sync::atomic::AtomicUsize,
    seen: std::sync::atomic::AtomicUsize,
}

impl FailingArtifacts {
    fn new(inner: Arc<FsOps>, allowed: usize) -> Arc<Self> {
        Arc::new(Self {
            inner,
            allowed: std::sync::atomic::AtomicUsize::new(allowed),
            seen: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Let every later `upsert` through again, so a test can publish
    /// normally after the injected failure and watch the repair.
    fn heal(&self) {
        self.allowed
            .store(usize::MAX, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl crate::ports::artifacts::ArtifactStore for FailingArtifacts {
    async fn list(
        &self,
        company: &CompanyId,
        task_id: Option<&str>,
    ) -> crate::Result<Vec<ArtifactRecord>> {
        crate::ports::artifacts::ArtifactStore::list(&*self.inner, company, task_id).await
    }
    async fn get(&self, company: &CompanyId, id: &str) -> crate::Result<Option<ArtifactRecord>> {
        crate::ports::artifacts::ArtifactStore::get(&*self.inner, company, id).await
    }
    async fn upsert(&self, company: &CompanyId, artifact: &ArtifactRecord) -> crate::Result<()> {
        let n = self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n >= self.allowed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::error::OpenCompanyError::Store(
                "artifact store is down".to_string(),
            ));
        }
        crate::ports::artifacts::ArtifactStore::upsert(&*self.inner, company, artifact).await
    }
    async fn delete(&self, company: &CompanyId, id: &str) -> crate::Result<bool> {
        crate::ports::artifacts::ArtifactStore::delete(&*self.inner, company, id).await
    }
}

fn publish_of(source: &str, body: &str) -> crate::harness::publish::PendingPublish {
    crate::harness::publish::PendingPublish {
        agent: "maya".to_string(),
        source: source.to_string(),
        title: "Launch spec".to_string(),
        kind: crate::ports::artifacts::ArtifactKind::Markdown,
        note: None,
        payload: crate::harness::publish::PublishPayload::Text(body.to_string()),
    }
}

/// The named node under `agents/maya/t-1/`, with its body — the tree's own
/// answer, read without going through the artifact chain at all.
async fn note_in_tree(ops: &FsOps, company: &CompanyId, name: &str) -> Option<(String, String)> {
    use crate::ports::workspace::WorkspaceStore;
    let nodes = WorkspaceStore::tree(ops, company).await.unwrap();
    let found = nodes.iter().find(|n| n.name == name)?;
    let (_, body) = WorkspaceStore::read(ops, company, &found.id)
        .await
        .unwrap()?;
    Some((found.id.clone(), body))
}

// ── Issue #151 §3.2: a finished card answers where it was asked ──────

// The post-back's *text* rules — title, landing status, note folding,
// whitespace-only notes — moved with the renderer to
// `crate::harness::lifecycle` (issue #186), which owns them now and covers
// each case plus the new assignee-credit rule. What stays here is the
// wiring: that `run_task` reaches the relay at all, and attributes it to
// the orchestrator.

async fn only_card(tasks: &Arc<FsOps>) -> TaskRecord {
    tasks
        .list(&CompanyId::new("acme"))
        .await
        .expect("list")
        .into_iter()
        .next()
        .expect("one card")
}

// ── Issue #337: every finished card stops for a person ────────────────

// ── Issue #242: the attempt row records what the dispatch actually did ──

/// Wires a run store onto a task-capable brain, mints the `Pending` row the
/// dispatch choke point would have minted, and returns both.
async fn brain_with_a_pending_run(
    dir: &std::path::Path,
    assignee: &str,
) -> (HarnessBrain, Arc<FsOps>, Arc<dyn crate::ports::RunStore>) {
    use crate::ports::runs::NewRun;

    let (brain, tasks) = brain_with_tasks(dir);
    let runs: Arc<dyn crate::ports::RunStore> = Arc::new(FsOps::new(dir));
    let company = CompanyId::new("acme");
    tasks
        .upsert(&company, &card("t-1", assignee))
        .await
        .expect("seed");
    runs.create_run(&company, NewRun::for_task("run-1", "t-1", assignee))
        .await
        .expect("mint");
    (brain.with_runs(Arc::clone(&runs)), tasks, runs)
}

// ── Issue #205: the working agent is linked, and a bad assignee is refused ──

// --- Orchestrator routing + delegation ----------------------------------

/// A roster with an `orchestrator`-tier agent (not first) and a desk.
fn record_with_desk() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Runs Acme."

[[agent]]
id = "chief"
role = "Chief of Staff"
tier = "orchestrator"
description = "Coordinates the company."

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds it."

[[group_chat]]
id = "eng_desk"
name = "Engineering"
members = ["engineer"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

/// A roster with a desk whose id collides with a teammate id — the exact
/// shape `runtime::delegation_tools::a_prefixed_dm_reaches_the_teammate_
/// even_when_a_desk_shares_the_id` (issue #1743) exercises for
/// `chat_responder`. Manifest validation does not forbid the collision.
fn record_with_colliding_desk_and_teammate_id() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "chief"
role = "Chief of Staff"
tier = "orchestrator"
description = "Coordinates the company."

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds it."

[[group_chat]]
id = "engineer"
name = "Engineering desk"
members = ["chief"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

/// A brain over `record`, wired to a real task store.
fn brain_over(dir: &std::path::Path, record: CompanyRecord) -> (HarnessBrain, Arc<FsOps>) {
    let tasks = Arc::new(FsOps::new(dir));
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: Some(Arc::new(FsOps::new(dir))),
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(tasks.clone()),
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record),
        tasks,
    )
}

/// A brain over the desk-bearing record, wired to a real task store.
fn brain_with_desk(dir: &std::path::Path) -> (HarnessBrain, Arc<FsOps>) {
    brain_over(dir, record_with_desk())
}

// -----------------------------------------------------------------------
// Mention routing: naming somebody outranks the desk lead
// -----------------------------------------------------------------------

fn mention_of(id: &str) -> crate::ports::types::Mention {
    crate::ports::types::Mention {
        target: crate::ports::types::MentionTarget::Agent { id: id.to_string() },
        text: format!("@{id}"),
        offset: 0,
        quiet: false,
    }
}

// ── Issue #151 §3.3: a DM thread reaches the teammate it names ──

// ── Issue #1743: who answers the built-in `#general` channel ──

// ── Issue #884 D2: an unresolvable chat key is no longer silent ──

/// Captures everything logged on **this thread** while `body` runs.
///
/// Thread-local (`with_default`) rather than a global default on purpose:
/// `workflow_scheduler`'s capture already claims the process-wide slot in
/// this same test binary and asserts it wins that race, so installing a
/// second global here would turn its test red for an unrelated reason.
fn logs_from(body: impl FnOnce()) -> String {
    use std::io::Write;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl Write for Writer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log sink").extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Writer;
        fn make_writer(&'a self) -> Self::Writer {
            Writer(self.0.clone())
        }
    }

    let sink = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(Sink(sink.clone()))
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, body);
    let bytes = sink.lock().expect("log sink").clone();
    String::from_utf8_lossy(&bytes).to_string()
}

// ── Issue #186 part b: orchestrator lifecycle authority ────────────────

// --- MCP failure drain --------------------------------------------------

/// A two-member desk record, which is the smallest roster shape
/// `desk_episode` opens as a hive room (a `deliberates(members.len())`
/// floor of two, with no `hive` block needed to opt in).
fn record_with_hive_desk() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "engineer"
role = "Engineer"

[[agent]]
id = "designer"
role = "Designer"

[[group_chat]]
id = "eng_desk"
name = "Engineering"
members = ["engineer", "designer"]
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
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

/// Content-aware scripted model for
/// `two_hive_desk_episodes_in_one_cycle_do_not_fold_into_each_other`.
///
/// Reads the rendered episode prompt exactly as the operator's model
/// would: which seat is being asked (`You are @<id>`), whether the room
/// is still deliberating or has already been told a topic carried
/// (`commit_protocol`'s `carried \`#<topic>\`` line), and which of the
/// two questions this desk was actually asked (`ALPHA_QUESTION` /
/// `BETA_QUESTION`, planted in each operator message's own text so a
/// prompt scan can tell episode A's transcript from episode B's without
/// touching the journal directly).
struct HiveTopicProvider;

/// The topic a `commit_protocol` block is telling this seat to record, if
/// the prompt carries one — i.e. the room already reached quorum.
fn carried_topic(prompt: &str) -> Option<String> {
    let marker = "carried `#";
    let start = prompt.find(marker)? + marker.len();
    let rest = &prompt[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

#[async_trait]
impl ChatModel<()> for HiveTopicProvider {
    async fn invoke(&self, _state: &(), request: ModelRequest) -> TaResult<ModelResponse> {
        let all_text: String = request
            .messages
            .iter()
            .map(Message::text)
            .collect::<Vec<_>>()
            .join("\n");
        if !all_text.contains("You are @engineer") && !all_text.contains("You are @designer") {
            return Ok(ModelResponse::assistant("(not a hive turn)".to_string()));
        }
        let line = if let Some(topic) = carried_topic(&all_text) {
            format!("!commit #{topic} ^1 because the room already carried it.")
        } else {
            // The desk's own memory recall can surface a PAST episode's
            // task and outcome as remembered context (by design — see
            // `a_desk_reasons_with_what_it_stored_in_an_earlier_episode`),
            // so the marker is read from the live transcript this turn
            // was actually handed, not from the whole prompt: the recall
            // block is prose about a prior episode, not this episode's
            // own fold.
            let transcript = all_text
                .split("Shared attributed transcript:")
                .nth(1)
                .unwrap_or(all_text.as_str());
            let topic = if transcript.contains("ALPHA_QUESTION") {
                "alpha"
            } else {
                "beta"
            };
            format!("!propose #{topic} Because the marker says so.")
        };
        Ok(ModelResponse::assistant(line))
    }
}

impl HarnessModel for HiveTopicProvider {
    fn telemetry_provider_id(&self) -> String {
        "hive-topic-mock".to_string()
    }
}

// --- Approval parking (issue #172) --------------------------------------

/// A brain over `dir` whose deps carry `requests` as the shared
/// approval-request queue — the same handle every roster agent's
/// `ApprovalPolicy` pushes onto.
fn brain_with_approval_queue(
    dir: &std::path::Path,
    requests: crate::harness::policy::ApprovalRequestQueue,
) -> HarnessBrain {
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: requests,
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record())
}

/// A host that fails to park the *first* effect it is handed, then behaves.
/// Models a transient journal/IO fault mid-batch.
#[derive(Default)]
struct FlakyParkingHost {
    parked: std::sync::Mutex<Vec<Effect>>,
    seen: std::sync::atomic::AtomicUsize,
}

impl FlakyParkingHost {
    fn parked(&self) -> Vec<Effect> {
        self.parked.lock().expect("parked").clone()
    }
}

#[async_trait]
impl CycleHost for FlakyParkingHost {
    async fn call_tool(&self, _call: ToolCall) -> Result<ToolResult> {
        Ok(ToolResult {
            ok: true,
            output: serde_json::Value::Null,
        })
    }
    async fn context_op(&self, _op: ContextOp) -> Result<ContextOpResult> {
        Ok(ContextOpResult::Text(String::new()))
    }
    async fn emit_effect(&self, _effect: Effect) -> Result<EffectDisposition> {
        panic!("an approval request must be parked, never re-evaluated as an effect");
    }
    async fn park_effect(&self, effect: Effect) -> Result<ApprovalId> {
        if self.seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Err(crate::OpenCompanyError::Store(
                "journal on fire".to_string(),
            ));
        }
        let mut parked = self.parked.lock().expect("parked");
        parked.push(effect);
        Ok(ApprovalId::new(format!("appr-{}", parked.len())))
    }
}

// --- Re-dispatching a granted call (issue #243) --------------------------

/// A brain over the offline mock provider, wired to a real event log and a
/// shared approval queue (whose grant set the runtime would mint into).
fn brain_with_queue_and_events(
    dir: &std::path::Path,
    requests: crate::harness::policy::ApprovalRequestQueue,
    events: Arc<dyn crate::ports::EventLog>,
) -> HarnessBrain {
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: Some(events),
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: requests,
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record())
}

/// As [`brain_with_queue_and_events`], but every model call fails with a
/// budget-exhausted body via [`BudgetExhaustedProvider`] (issue #1846
/// review, Codex #3869725683) — otherwise byte-identical, so the only
/// variable a test built on this exercises is how the approval-
/// continuation redispatch path reacts to that one failure shape.
fn brain_with_queue_and_events_and_budget_exhausted_provider(
    dir: &std::path::Path,
    requests: crate::harness::policy::ApprovalRequestQueue,
    events: Arc<dyn crate::ports::EventLog>,
) -> HarnessBrain {
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(BudgetExhaustedProvider),
        provider_slug: "scripted".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: Some(events),
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: requests,
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record())
}

fn approval_resolved(id: &str, verdict: Verdict) -> CompanyEvent {
    CompanyEvent::ApprovalResolved {
        approval_id: ApprovalId::new(id),
        verdict,
        by: crate::ports::types::Actor {
            kind: crate::ports::types::ActorKind::Operator,
            id: "owner".into(),
        },
    }
}

fn cycle_over(events: Vec<CompanyEvent>) -> CycleRequest {
    CycleRequest {
        cycle_id: "cyc-1".to_string(),
        company_id: CompanyId::new("acme"),
        events,
        event_seqs: Vec::new(),
        policy: None,
    }
}

async fn assert_explicit_decision_continues(verdict: Verdict, expected: &str) {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    requests
        .grants()
        .continue_approval(crate::runtime::grants::ApprovalContinuation {
            call: crate::runtime::grants::GrantedCall {
                approval_id: ApprovalId::new("appr-explicit"),
                agent: "ceo".into(),
                tool: crate::harness::approval_tool::REQUEST_APPROVAL_TOOL.into(),
                args: serde_json::json!({
                    "title": "Publish the announcement",
                    "question": "May I publish it?"
                }),
                at_millis: now_millis(),
                origin_thread: None,
                origin_parent: None,
                origin_task: None,
            },
            verdict,
            by: crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::User,
                id: "operator".into(),
            },
        });
    let grants = requests.grants();
    let brain = brain_with_queue_and_events(dir.path(), requests, log);

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-explicit", verdict)]),
            &NoopHost,
        )
        .await
        .unwrap();

    assert_eq!(result.channel_responses.len(), 1);
    let text = &result.channel_responses[0].text;
    assert!(text.contains(expected), "{text}");
    assert!(text.contains("Publish the announcement"), "{text}");
    assert!(!text.contains("Re-issue it"), "{text}");
    assert!(
        grants
            .peek_continuation(&ApprovalId::new("appr-explicit"))
            .is_none()
    );
}

/// No continuation reply was journaled by the brain itself (issue #469).
async fn no_replies_journaled(log: &Arc<dyn crate::ports::EventLog>) -> bool {
    log.read_from(&CompanyId::new("acme"), crate::ports::EventSeq::new(0), 100)
        .await
        .unwrap()
        .iter()
        .all(|e| !matches!(e.event, CompanyEvent::AgentReply { .. }))
}

// --- Issue #453: a re-dispatch drains what its turn queued ---------------

/// What the scripted model does on each successive `/chat/completions` call.
#[derive(Clone, Debug)]
enum ScriptTurn {
    /// Emit a native tool call.
    Call {
        tool: &'static str,
        args: serde_json::Value,
    },
    /// Finish with plain assistant text.
    Say(&'static str),
}

/// Serves a scripted OpenAI-compatible endpoint on loopback and returns its
/// base URL.
///
/// `MockProvider` cannot express a tool call, and a tool call is the whole
/// point here: the defect is that a `review_task` made by a re-issued turn
/// was staged and never drained. Same shape `workspace_turn_test` and
/// `gated_tool_turn_test` established — stub exactly one boundary, the
/// model's choices, and run everything else for real.
async fn spawn_model_script(turns: Vec<ScriptTurn>) -> String {
    use axum::Json;
    use axum::routing::post;

    let script = Arc::new(std::sync::Mutex::new(turns));
    let app = axum::Router::new().route(
        "/chat/completions",
        post(move |Json(_body): Json<serde_json::Value>| {
            let script = Arc::clone(&script);
            async move {
                let next = {
                    let mut turns = script.lock().unwrap();
                    if turns.is_empty() {
                        None
                    } else {
                        Some(turns.remove(0))
                    }
                };
                // Running off the end means the loop went round more times
                // than expected; end the turn rather than hang.
                let message = match next.unwrap_or(ScriptTurn::Say("done")) {
                    ScriptTurn::Say(text) => {
                        serde_json::json!({ "role": "assistant", "content": text })
                    }
                    ScriptTurn::Call { tool, args } => serde_json::json!({
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": format!("call-{tool}"),
                            "type": "function",
                            "function": { "name": tool, "arguments": args.to_string() }
                        }]
                    }),
                };
                Json(serde_json::json!({
                    "choices": [{ "index": 0, "message": message }],
                    "usage": { "prompt_tokens": 12, "completion_tokens": 4 }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}")
}

/// A brain over the scripted model with a **real task store**, so a
/// `review_task` the re-dispatched turn makes can actually move a card.
fn brain_over_script(
    dir: &std::path::Path,
    requests: crate::harness::policy::ApprovalRequestQueue,
    base_url: String,
) -> HarnessBrain {
    use crate::company::credentials::Credential;
    use crate::harness::provider::{HostedProvider, HostedProviderConfig};

    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(HostedProvider::new(HostedProviderConfig {
            base_url,
            credential: Credential::from_value("stub-key"),
            extra_headers: Vec::new(),
        })),
        provider_slug: "managed".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: Some("stub-model".to_string()),
        tasks: Some(Arc::new(FsOps::new(dir))),
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: requests,
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record())
}

/// A card sitting in review, waiting on the verdict the operator approved.
fn card_in_review(id: &str) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        title: TaskTitle::authored(&format!("Work item {id}")),
        note: None,
        column: COLUMN_IN_REVIEW.to_string(),
        priority: "medium".to_string(),
        assignee: "ceo".to_string(),
        updated_at_millis: now_millis(),
        origin: None,
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

fn granted(approval: &str, tool: &str) -> crate::runtime::grants::GrantedCall {
    crate::runtime::grants::GrantedCall {
        approval_id: ApprovalId::new(approval),
        agent: "ceo".into(),
        tool: tool.into(),
        args: serde_json::json!({}),
        at_millis: now_millis(),
        origin_thread: None,
        origin_parent: None,
        origin_task: None,
    }
}

// --- Steer disposition (issue #111) -------------------------------------

use crate::company::steer::{InflightKind, InflightRegistry};
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

/// A model that steers its OWN in-flight run on selected turns (via the
/// shared registry), so the disposition matrix can be driven deterministically
/// over an offline turn. It pops one queued action per [`invoke`](ChatModel::invoke)
/// call and applies it against `key`, then echoes the last user message.
struct SteeringProvider {
    steer: InflightRegistry,
    company: CompanyId,
    key: String,
    actions: StdMutex<VecDeque<SteerAction>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChatModel<()> for SteeringProvider {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(action) = self.actions.lock().unwrap().pop_front() {
            let key = if self.key.is_empty() {
                self.steer
                    .list(&self.company)
                    .into_iter()
                    .next()
                    .map(|entry| entry.key)
                    .unwrap_or_default()
            } else {
                self.key.clone()
            };
            let _ = self.steer.steer(&self.company, &key, action);
        }
        let message = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::User(_)))
            .map(|m| m.text())
            .unwrap_or_default();
        Ok(ModelResponse::assistant(format!("did: {message}")))
    }
}

impl HarnessModel for SteeringProvider {
    fn telemetry_provider_id(&self) -> String {
        "steering".to_string()
    }
}

/// A deterministic turn result for scheduled-cycle edge-case tests.
struct FixedOutcomeTurn {
    outcome: crate::harness::built_in::TurnOutcome,
    approval_requests: Option<crate::harness::policy::ApprovalRequestQueue>,
}

#[async_trait]
impl crate::runtime::delegation::RunTurn for FixedOutcomeTurn {
    async fn run(
        &self,
        _company: &CompanyId,
        _agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        if let Some(requests) = &self.approval_requests {
            for index in 0..(crate::harness::policy::MAX_APPROVAL_REQUESTS_PER_TURN + 1) {
                requests.push(crate::harness::policy::ApprovalRequest {
                    tool: format!("test_tool_{index}"),
                    reason: "test approval".to_string(),
                    effect: Effect {
                        kind: format!("test_tool_{index}"),
                        group: crate::ports::types::EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: serde_json::json!({ "index": index }),
                        agent: None,
                        run_id: None,
                    },
                });
            }
        }
        Ok(self.outcome.clone())
    }

    async fn run_steered(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        _control: &crate::company::steer::SteerControl,
        chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        self.run(company, agent_id, message, chat).await
    }

    async fn run_steered_background(
        &self,
        company: &CompanyId,
        agent_id: &str,
        message: &str,
        _control: &crate::company::steer::SteerControl,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
        _run_sink: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        self.run(
            company,
            agent_id,
            message,
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await
    }
}

/// A `FixedOutcomeTurn` whose single turn reports a budget pause — the
/// account itself is out of inference credits, so `outcome.reply` is
/// host-authored pause copy, not an answer.
fn budget_paused_outcome(agent: &str) -> crate::harness::built_in::TurnOutcome {
    crate::harness::built_in::TurnOutcome {
        reply: BUDGET_PAUSED_PLACEHOLDER_REPLY.to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: Some(crate::harness::BudgetPause {
            agent: agent.to_string(),
            summary: "add credits and try again".to_string(),
        }),
    }
}

/// A `FixedOutcomeTurn` whose single turn halted for spend — the
/// teammate's own declared cap was reached mid-turn.
fn spend_halted_outcome(agent: &str) -> crate::harness::built_in::TurnOutcome {
    crate::harness::built_in::TurnOutcome {
        reply: "partial answer before the brake fired".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: Some(crate::harness::SpendHalt {
            agent: agent.to_string(),
            spent_usd: 5.5,
            cap_usd: 5.0,
        }),
        budget_paused: None,
    }
}

/// A `FixedOutcomeTurn` whose single turn is a pre-dispatch spend
/// refusal — the meter that a declared cap needs could not be read, so
/// no model call ran and `outcome.reply` is host-authored refusal copy.
fn abnormal_stop_outcome(reply: &str) -> crate::harness::built_in::TurnOutcome {
    crate::harness::built_in::TurnOutcome {
        reply: reply.to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: Some("[stopped: dispatch refused]".to_string()),
        halted_for_spend: None,
        budget_paused: None,
    }
}

/// A bare brain over a fresh temp-dir store, for tests that only need
/// `HiveDeskRunner`'s `brain`/`host` fields satisfied and are not
/// exercising the approval-parking path itself.
fn hive_test_brain(dir: &std::path::Path) -> HarnessBrain {
    brain_with_approval_queue(dir, crate::harness::policy::ApprovalRequestQueue::default())
}

fn hive_desk_runner<'a>(
    brain: &'a HarnessBrain,
    host: &'a dyn CycleHost,
    outcome: crate::harness::built_in::TurnOutcome,
) -> HiveDeskRunner<'a> {
    HiveDeskRunner {
        run_turn: Arc::new(FixedOutcomeTurn {
            outcome,
            approval_requests: None,
        }),
        company: CompanyId::new("acme"),
        chat_id: Some("lab".to_string()),
        thread_root: None,
        trigger_seq: None,
        brain,
        host,
    }
}

/// A brain whose provider steers the dispatched card `key` with `actions`
/// (one per turn). Returns the brain + its task store so a test can seed the
/// card and read the disposition back.
fn brain_that_steers_itself(
    dir: &std::path::Path,
    key: &str,
    actions: Vec<SteerAction>,
) -> (HarnessBrain, Arc<FsOps>, Arc<SteeringProvider>) {
    let steer = InflightRegistry::new();
    let tasks = Arc::new(FsOps::new(dir));
    let provider = Arc::new(SteeringProvider {
        steer: steer.clone(),
        company: CompanyId::new("acme"),
        key: key.to_string(),
        actions: StdMutex::new(actions.into_iter().collect()),
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: provider.clone(),
        provider_slug: "steering".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(tasks.clone()),
        // Same handle as `tasks` (FsOps is both stores), so a steered run's
        // artifact side effect — or the absence of one — is observable.
        artifacts: Some(tasks.clone()),
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer,
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record()),
        tasks,
        provider,
    )
}

// --- CEO-relay hand-back (delegate_to_desk second turn) ------------------

/// A provider that simulates the orchestrator queuing a `delegate_to_desk`
/// on its turns: on each invoke it pops the next scripted delegation (if any)
/// onto the shared queue — exactly what the real tool call does — then echoes
/// the last user message so a test can read the turn's reply. Sharing the
/// queue handle with [`HarnessDeps::delegations`] is what lets the brain
/// drain it after the turn.
/// Whether this request is a triage escalation rather than an agent turn
/// (issue #678).
///
/// Keyed on the system prompt's opening sentence, which
/// `harness::triage::system_prompt` owns. Coupling a fixture to prose is
/// ordinarily a smell; here the alternative is worse, because the only
/// other thing distinguishing the two is "carries no tools", and a turn
/// whose agent happens to have an empty belt would be misread as a
/// classification. Pinned by `a_triage_request_is_recognised_as_one`.
fn is_triage_request(request: &ModelRequest) -> bool {
    request
        .messages
        .first()
        .map(|m| m.text().contains("You classify one message"))
        .unwrap_or(false)
}

/// A provider for the selection rung (issue #1835): a request opening with
/// the selector's own system prompt gets the scripted reply; anything else
/// echoes. Keyed on the prompt's opening sentence for the reason
/// [`is_triage_request`] documents, and pinned the same way below.
struct SelectingProvider {
    reply: String,
    selector_calls: std::sync::atomic::AtomicUsize,
}

fn is_selection_request(request: &ModelRequest) -> bool {
    request
        .messages
        .first()
        .map(|m| {
            m.text()
                .contains("You route one message in a group channel")
        })
        .unwrap_or(false)
}

#[async_trait]
impl ChatModel<()> for SelectingProvider {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        if is_selection_request(&request) {
            self.selector_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Ok(ModelResponse::assistant(self.reply.clone()));
        }
        let message = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::User(_)))
            .map(|m| m.text())
            .unwrap_or_default();
        Ok(ModelResponse::assistant(format!("mock: {message}")))
    }
}

impl HarnessModel for SelectingProvider {
    fn telemetry_provider_id(&self) -> String {
        "selecting".to_string()
    }
}

/// A brain whose provider answers every selection request with `reply`.
/// The record is [`record_with_desk`] — `engineer` + `chief`, a lead
/// `eng_desk` — plus an `auto` overlay channel `launch` holding both.
fn brain_that_selects(
    dir: &std::path::Path,
    reply: &str,
) -> (HarnessBrain, Arc<SelectingProvider>) {
    brain_that_selects_with(dir, reply, None, None)
}

/// [`brain_that_selects`], plus an optional plan and usage meter, so a
/// test can place the company past its plan-level total-token ceiling.
fn brain_that_selects_with(
    dir: &std::path::Path,
    reply: &str,
    plan: Option<crate::harness::capability_budget::CapabilityPlan>,
    meter: Option<Arc<dyn crate::ports::usage::UsageMeter>>,
) -> (HarnessBrain, Arc<SelectingProvider>) {
    let provider = Arc::new(SelectingProvider {
        reply: reply.to_string(),
        selector_calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: provider.clone(),
        provider_slug: "selecting".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        artifacts: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer: InflightRegistry::new(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    let mut record = record_with_desk();
    record.overlay_desks.push(crate::ports::types::OverlayDesk {
        id: "launch".to_string(),
        name: "Launch week".to_string(),
        description: None,
        members: vec!["engineer".to_string(), "chief".to_string()],
        responder: crate::ports::types::ResponderMode::Auto,
        hive: Default::default(),
    });
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record),
        provider,
    )
}

/// A meter whose every query reports spend past any ceiling a test sets.
struct SpentMeter;

#[async_trait]
impl crate::ports::usage::UsageMeter for SpentMeter {
    async fn record(
        &self,
        _company: &CompanyId,
        _sample: &crate::ports::usage::UsageSample,
    ) -> crate::Result<()> {
        Ok(())
    }
    async fn query(
        &self,
        _company: &CompanyId,
        _since: u64,
    ) -> crate::Result<Vec<crate::ports::usage::UsageSample>> {
        Ok(vec![crate::ports::usage::UsageSample {
            at_millis: 0,
            agent: "someone".to_string(),
            provider: "test".to_string(),
            input_tokens: 10_000,
            output_tokens: 0,
            cached_input_tokens: 0,
            cost_usd: 0.0,
            kind: crate::ports::usage::SampleKind::Inference,
            run_id: None,
            model: None,
        }])
    }
}

struct DelegatingProvider {
    queue: orchestrator::DelegationQueue,
    pushes: StdMutex<VecDeque<Vec<Delegation>>>,
    calls: std::sync::atomic::AtomicUsize,
    /// The same task store the brain writes through, so each invoke can
    /// snapshot the board **as that turn sees it** — the only way a test can
    /// observe a dispatched card mid-run (issue #204).
    tasks: Arc<FsOps>,
    /// `(column, assignee)` of the company's card at each invoke, in order.
    board: StdMutex<Vec<(String, String)>>,
    /// How this provider misbehaves, by invoke number.
    faults: TurnFaults,
    /// The same registry wired into [`HarnessDeps::steer`], so a scripted
    /// invoke can cancel its own in-flight delegation.
    steer: InflightRegistry,
}

/// How a [`DelegatingProvider`] misbehaves, keyed by 1-based invoke number
/// (issue #213 review).
#[derive(Default)]
struct TurnFaults {
    /// Every invoke from here on ERRORS instead of answering, so a test can
    /// make a delegate's own run fail. A *from*, not an *on*: openhuman's
    /// agent loop retries a failed provider call within the same turn, so
    /// failing a single invoke only makes the turn succeed on its retry.
    fail_from: Option<usize>,
    /// Invokes that CANCEL their own in-flight delegation mid-run, so the
    /// delegated reply is discarded exactly as an operator cancel does.
    cancel_on: Vec<usize>,
    /// Desk keys the first turn's `delegate_to_desk` calls named and the
    /// tool REFUSED (issue #272). A refusal never becomes a `Delegation`,
    /// so this is how a test reproduces one without standing up the tool.
    refused_on_first: Vec<String>,
}

impl DelegatingProvider {
    /// The board snapshot each turn ran against, in invoke order.
    fn board(&self) -> Vec<(String, String)> {
        self.board.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatModel<()> for DelegatingProvider {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        // Issue #678: a triage escalation is a classification, not a turn.
        // It rides the same `HarnessModel` handle the roster runs on, so
        // without this it would consume a scripted push and shift every
        // turn's script by one — the delegation the test wrote for turn 1
        // would be staged by a call that is not turn 1.
        //
        // Answering `chatter` rather than declining keeps these fixtures on
        // the ungated path they were written for: only an `answer` verdict
        // narrows the claim, so `chatter` leaves the gate exactly where the
        // abstention left it. A test that wants the narrowing drives it
        // through `DelegationRunner::with_triage` directly, where the
        // verdict is scripted.
        if is_triage_request(&request) {
            return Ok(ModelResponse::assistant("chatter".to_string()));
        }
        let invoke = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if self.faults.fail_from.is_some_and(|from| invoke >= from) {
            return Err(tinyinference::Error::Model(
                "the delegate's provider fell over".to_string(),
            ));
        }
        if self.faults.cancel_on.contains(&invoke) {
            // Cancel the entry this path actually registered.
            //
            // A CHAT-turn delegation still runs inside its delegator's
            // turn, so it has its own `Delegation` entry and that is the
            // one to cancel — cancelling the card there would end the whole
            // run. A DISPATCHED card's hand-off no longer works that way:
            // since the async hand-off the delegate runs as its own
            // dispatch, so the only entry in flight is the card's `Task`,
            // and it IS the delegate's run. Preferring `Delegation` keeps
            // the chat path targeting exactly what it did before.
            let company = CompanyId::new("acme");
            let entries = self.steer.list(&company);
            let target = entries
                .iter()
                .find(|e| e.kind == InflightKind::Delegation)
                .or_else(|| entries.iter().find(|e| e.kind == InflightKind::Task))
                .cloned();
            if let Some(entry) = target {
                let _ = self.steer.steer(&company, &entry.key, SteerAction::Cancel);
            }
        }
        let snapshot = self
            .tasks
            .list(&CompanyId::new("acme"))
            .await
            .ok()
            .and_then(|cards| cards.into_iter().next())
            .map(|card| (card.column, card.assignee))
            .unwrap_or_default();
        self.board.lock().unwrap().push(snapshot);
        for delegation in self.pushes.lock().unwrap().pop_front().unwrap_or_default() {
            self.queue.push(delegation);
        }
        if invoke == 1 {
            for desk in &self.faults.refused_on_first {
                self.queue.push_refusal(desk.clone());
            }
        }
        let message = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::User(_)))
            .map(|m| m.text())
            .unwrap_or_default();
        Ok(ModelResponse::assistant(format!("did: {message}")))
    }
}

impl HarnessModel for DelegatingProvider {
    fn telemetry_provider_id(&self) -> String {
        "delegating".to_string()
    }
}

/// A brain over the desk-bearing record whose provider is a
/// [`DelegatingProvider`] scripted to push `pushes[i]` on invoke `i + 1`.
/// Returns the brain plus the shared provider so a test can read the invoke
/// count.
fn brain_that_delegates(
    dir: &std::path::Path,
    pushes: Vec<Option<Delegation>>,
) -> (HarnessBrain, Arc<DelegatingProvider>) {
    brain_that_delegates_with(
        dir,
        pushes.into_iter().map(Vec::from_iter).collect(),
        TurnFaults::default(),
    )
}

/// [`brain_that_delegates`], but each invoke pushes a whole *set* of
/// delegations (a single turn can queue several), and per-invoke faults can
/// make a delegate's run fail or be cancelled mid-flight.
fn brain_that_delegates_with(
    dir: &std::path::Path,
    pushes: Vec<Vec<Delegation>>,
    faults: TurnFaults,
) -> (HarnessBrain, Arc<DelegatingProvider>) {
    let queue = orchestrator::DelegationQueue::default();
    let tasks = Arc::new(FsOps::new(dir));
    let steer = InflightRegistry::new();
    let provider = Arc::new(DelegatingProvider {
        queue: queue.clone(),
        pushes: StdMutex::new(pushes.into_iter().collect()),
        calls: std::sync::atomic::AtomicUsize::new(0),
        tasks: tasks.clone(),
        board: StdMutex::new(Vec::new()),
        faults,
        steer: steer.clone(),
    });
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: provider.clone(),
        provider_slug: "delegating".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir)),
        store: Arc::new(FsCompanyStore::new(dir)),
        meter: None,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: Some(tasks),
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        artifacts: None,
        delegations: queue,
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: crate::harness::mcp_probe::McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_revisions: None,
        approval_requests: crate::harness::policy::ApprovalRequestQueue::default(),
        secrets: None,
        web_allowed_domains: Vec::new(),
        capabilities: crate::harness::toolbelt::CapabilityFilter::AllowAll,
        workflow_source_dir: None,
        plan: None,
        media: None,
        composio: None,
        #[cfg(feature = "chargebee")]
        chargebee: None,
        #[cfg(feature = "paypal")]
        paypal: None,
        hosting: None,
        steer,
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    (
        HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_with_desk()),
        provider,
    )
}

// --- Issue #204: a dispatched turn that delegates -----------------------

/// Seeds one dispatched card (blank assignee → the orchestrator runs it,
/// which is the shape that carries the delegation tools) and dispatches it.
/// Dispatches a card and drives its hand-off chain to a settle, the way
/// `CompanyRuntime::run_dispatch_cycle` does in production.
///
/// One `run_cycle` is one ATTEMPT, and since the async hand-off an attempt
/// that hands the card on settles `Delegated` and leaves the card
/// `in_progress` for the new owner — the delegate runs in its own attempt,
/// which is what makes their spend attributable and releases the
/// per-company lock between hops. A helper that ran a single cycle would
/// therefore stop one attempt short of every hand-off's outcome, and a test
/// asking "where does a cancelled hand-off end up?" would be reading a
/// card mid-chain.
///
/// The loop condition is the runtime's own: a settled dispatch still in
/// `in_progress` has handed on, because every other ending lands the card
/// in a terminal column.
async fn dispatch_card(brain: &HarnessBrain, tasks: &Arc<FsOps>, id: &str) {
    let mut c = card(id, "");
    c.column = "in_progress".to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();
    for _ in 0..=crate::company::runtime::MAX_HAND_OFF_HOPS {
        brain
            .run_cycle(
                request(vec![CompanyEvent::TaskDispatched {
                    task_id: id.to_string(),
                    run_id: None,
                }]),
                &NoopHost,
            )
            .await
            .expect("cycle runs");
        let handed_on = tasks
            .list(&CompanyId::new("acme"))
            .await
            .unwrap_or_default()
            .into_iter()
            .any(|card| card.id == id && card.column == crate::ports::tasks::COLUMN_IN_PROGRESS);
        if !handed_on {
            break;
        }
    }
}

// ---- named harnesses: does the wiring actually route? ------------------

/// Records which agents it ran, so a test can assert *which* lane served a
/// turn rather than only that one did.
struct SpyLane {
    label: String,
    seen: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl crate::runtime::delegation::RunTurn for SpyLane {
    async fn run(
        &self,
        _company: &CompanyId,
        agent_id: &str,
        _message: &str,
        _chat: crate::runtime::delegation::ChatTarget<'_>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        self.seen.lock().unwrap().push(agent_id.to_string());
        Ok(crate::harness::built_in::TurnOutcome {
            reply: self.label.clone(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            // Test fixture, not the ACP fold (PR #1880 review).
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        })
    }
    async fn run_steered(
        &self,
        c: &CompanyId,
        a: &str,
        m: &str,
        _: &crate::company::steer::SteerControl,
        chat: crate::runtime::delegation::ChatTarget<'_>,
        _: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        self.run(c, a, m, chat).await
    }
    async fn run_steered_background(
        &self,
        c: &CompanyId,
        a: &str,
        m: &str,
        _: &crate::company::steer::SteerControl,
        _: crate::runtime::delegation::ChatTarget<'_>,
        _: Option<Arc<crate::harness::run_trace::RunTraceSink>>,
    ) -> Result<crate::harness::built_in::TurnOutcome> {
        self.run(c, a, m, crate::runtime::delegation::ChatTarget::default())
            .await
    }
}

/// A roster spanning two declared harnesses.
fn two_harness_record() -> CompanyRecord {
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"

[[agent]]
id = "researcher"
role = "Researcher"
harness = "deep"

[[harness]]
id = "embedded"
kind = "built_in"
default = true

[[harness]]
id = "deep"
kind = "built_in"
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..record()
    }
}

// ── Issue #1861: blockers park instead of settling Failed ───────────────

#[path = "brain_tests_part1.rs"]
mod tests_part1;
#[path = "brain_tests_part10.rs"]
mod tests_part10;
#[path = "brain_tests_part11.rs"]
mod tests_part11;
#[path = "brain_tests_part2.rs"]
mod tests_part2;
#[path = "brain_tests_part3.rs"]
mod tests_part3;
#[path = "brain_tests_part4.rs"]
mod tests_part4;
#[path = "brain_tests_part5.rs"]
mod tests_part5;
#[path = "brain_tests_part6.rs"]
mod tests_part6;
#[path = "brain_tests_part7.rs"]
mod tests_part7;
#[path = "brain_tests_part8.rs"]
mod tests_part8;
#[path = "brain_tests_part9.rs"]
mod tests_part9;
