//! Shared fixtures for `built_in`'s own inline tests: mock stores, scripted
//! providers, and the `Fixture`/`manifest`/`record` builders every test group
//! reaches for. Split out of the inline `mod tests` block because the
//! combined module exceeded the 750-line file limit.

use super::*;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use tinyinference::model::{ChatModel, ModelRequest, ModelResponse};

use crate::company::CompanyManifest;
use crate::harness::provider::MockProvider;
use crate::ports::UsageSample;
use crate::ports::types::{
    ChunkAddr, ChunkHit, ChunkMeta, CompanySummary, ContextChunk, LedgerEntry,
};
// The two-level resolver. Test-only now: the roster build goes through
// `agent_scoped_grants`, and these tests assert the desk-less case still
// resolves identically to what shipped before desks could scope tools.
use crate::runtime::builder::agent_effective_grants;

pub(super) fn fp_entry_full(
    mode: Option<&str>,
    always: Option<Vec<&str>>,
    cap: Option<Option<f64>>,
    ttl: Option<u64>,
) -> PolicyOverride {
    use crate::ports::types::{Actor, ActorKind};
    PolicyOverride {
        mode: mode.map(str::to_string),
        always_approve: always.map(|v| v.into_iter().map(str::to_string).collect()),
        auto_approve_under_usd: cap,
        approval_ttl_hours: ttl,
        set_by: Actor {
            kind: ActorKind::User,
            id: "user-1".to_string(),
        },
        at_millis: 1_700_000_000_000,
    }
}

/// An effective `[policy]` block for fingerprint tests — what the roster is
/// actually built from (`CompanyRecord::effective_policy`).
pub(super) fn fp_policy(mode: &str, always: &[&str], cap: Option<f64>, ttl: Option<u64>) -> Policy {
    Policy {
        mode: mode.to_string(),
        always_approve: always.iter().map(|k| (*k).to_string()).collect(),
        auto_approve_under_usd: cap,
        approval_ttl_hours: ttl,
    }
}

/// In-memory `ContextStore` so `OcMemory` has somewhere to land.
#[derive(Default)]
pub(super) struct MockContext {
    pub(super) chunks: StdMutex<Vec<(ChunkAddr, ContextChunk)>>,
    // Monotonic, NOT chunks.len(): a delete shrinks the vec, and a
    // len-derived addr would then collide with a surviving chunk's.
    pub(super) next_addr: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ContextStore for MockContext {
    async fn put(&self, _id: &CompanyId, chunk: ContextChunk) -> crate::Result<ChunkAddr> {
        let n = self
            .next_addr
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut guard = self.chunks.lock().unwrap();
        let addr = ChunkAddr::new(format!("addr-{n}"));
        guard.push((addr.clone(), chunk));
        Ok(addr)
    }
    async fn list(&self, _id: &CompanyId, prefix: &str) -> crate::Result<Vec<ChunkMeta>> {
        let guard = self.chunks.lock().unwrap();
        Ok(guard
            .iter()
            .filter(|(_, c)| c.label.starts_with(prefix))
            .map(|(addr, c)| ChunkMeta {
                addr: addr.clone(),
                label: c.label.clone(),
                len: c.body.len(),
                // The mock does not model store time; these tests exercise
                // the harness, not the Brain's freshness stat.
                stored_at_millis: 0,
            })
            .collect())
    }
    async fn peek(
        &self,
        _id: &CompanyId,
        addr: &ChunkAddr,
        _range: Option<std::ops::Range<usize>>,
    ) -> crate::Result<String> {
        let guard = self.chunks.lock().unwrap();
        Ok(guard
            .iter()
            .find(|(a, _)| a == addr)
            .map(|(_, c)| c.body.clone())
            .unwrap_or_default())
    }
    async fn delete(&self, _id: &CompanyId, addr: &ChunkAddr) -> crate::Result<bool> {
        let mut guard = self.chunks.lock().unwrap();
        let before = guard.len();
        guard.retain(|(a, _)| a != addr);
        Ok(guard.len() < before)
    }
    async fn delete_label(
        &self,
        _id: &CompanyId,
        addr: &ChunkAddr,
        label: &str,
    ) -> crate::Result<bool> {
        let mut guard = self.chunks.lock().unwrap();
        let before = guard.len();
        guard.retain(|(a, c)| !(a == addr && c.label == label));
        Ok(guard.len() < before)
    }
    async fn search(
        &self,
        _id: &CompanyId,
        query: &str,
        limit: usize,
    ) -> crate::Result<Vec<ChunkHit>> {
        let guard = self.chunks.lock().unwrap();
        Ok(guard
            .iter()
            .filter(|(_, c)| c.body.contains(query))
            .take(limit)
            .map(|(addr, c)| ChunkHit {
                addr: addr.clone(),
                snippet: c.body.clone(),
                score: 1.0,
            })
            .collect())
    }
}

/// `CompanyStore` that records what the cost hook appends.
#[derive(Default)]
pub(super) struct RecordingStore {
    pub(super) ledger: StdMutex<Vec<LedgerEntry>>,
}

#[async_trait]
impl CompanyStore for RecordingStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(None)
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, entry: LedgerEntry) -> crate::Result<()> {
        self.ledger.lock().unwrap().push(entry);
        Ok(())
    }
}

/// `CompanyStore` whose `append_ledger` always fails — the "ledger write
/// that also failed" `turn_result_after_metering`'s own doc names, and
/// the fixture `a_metering_failure_does_not_swallow_a_budget_pause_marker`
/// needs to force `meter_turn_costs` into its `Err` arm on a turn that
/// otherwise succeeded (Codex review, PR #2053).
#[derive(Default)]
pub(super) struct FailingLedgerStore;

#[async_trait]
impl CompanyStore for FailingLedgerStore {
    async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
        Ok(None)
    }
    async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
        Ok(())
    }
    async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
        Ok(Vec::new())
    }
    async fn append_ledger(&self, _id: &CompanyId, _entry: LedgerEntry) -> crate::Result<()> {
        Err(OpenCompanyError::Harness(
            "scripted ledger outage".to_string(),
        ))
    }
}

/// Records usage samples so a zero-usage turn can be asserted inert.
#[derive(Default)]
pub(super) struct RecordingMeter {
    pub(super) samples: StdMutex<Vec<UsageSample>>,
}

#[async_trait]
impl UsageMeter for RecordingMeter {
    async fn record(&self, _company: &CompanyId, sample: &UsageSample) -> crate::Result<()> {
        self.samples.lock().unwrap().push(sample.clone());
        Ok(())
    }
    /// Honours `since_millis`, per the port contract ("every sample at or
    /// after `since_millis`"). The per-agent daily cap (issue #304) is a
    /// windowed read, so a double that returned everything regardless would
    /// make the day-rollover test pass against any boundary the code
    /// computed — including none at all.
    async fn query(&self, _company: &CompanyId, since: u64) -> crate::Result<Vec<UsageSample>> {
        Ok(self
            .samples
            .lock()
            .unwrap()
            .iter()
            .filter(|sample| sample.at_millis >= since)
            .cloned()
            .collect())
    }
}

/// A meter whose reads always fail — for the dispatch gate's fail-open pin.
pub(super) struct FailingMeter;

#[async_trait]
impl UsageMeter for FailingMeter {
    async fn record(&self, _company: &CompanyId, _sample: &UsageSample) -> crate::Result<()> {
        Ok(())
    }
    async fn query(
        &self,
        _company: &CompanyId,
        _since: u64,
    ) -> crate::Result<Vec<UsageSample>> {
        Err(OpenCompanyError::Store("meter unavailable".into()))
    }
}

pub(super) fn manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction."

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds the product."
"#,
    )
    .expect("valid manifest")
}

pub(super) fn record() -> CompanyRecord {
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest: manifest(),
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        setup: None,
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
        name_confirmed: false,
        activation_completed_at: None,
        created_at_millis: None,
    }
}

pub(super) struct Fixture {
    pub(super) deps: HarnessDeps,
    pub(super) store: Arc<RecordingStore>,
    pub(super) meter: Arc<RecordingMeter>,
    pub(super) _dir: tempfile::TempDir,
}

pub(super) fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(RecordingStore::default());
    let meter = Arc::new(RecordingMeter::default());
    Fixture {
        deps: HarnessDeps {
            emergency_gate: None,
            notifications: None,
            ledgers: None,
            ledger_registry: Default::default(),
            provider: Arc::new(MockProvider::new("mock: ")),
            provider_slug: "mock".to_string(),
            serves: None,
            context: Arc::new(MockContext::default()),
            store: store.clone(),
            meter: Some(meter.clone()),
            workspace_root: dir.path().to_path_buf(),
            mcp_home: None,
            workspace_git_enabled: false,
            audit_root: dir.path().to_path_buf(),
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
            delegations: DelegationQueue::default(),
            workflow_runner: crate::harness::orchestrator::WorkflowRunnerHandle::default(),
            mcp_failures: McpFailureQueue::default(),
            pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
            workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
            run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
            run_output_store: None,
            workflow_runs: None,
            deep_trace: None,
            workflow_revisions: None,
            approval_requests: ApprovalRequestQueue::default(),
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
        },
        store,
        meter,
        _dir: dir,
    }
}

/// Context routing: the resolution that feeds a persona, and the fingerprint
/// that decides whether an edit reaches the next turn.
mod routed_context {
    use super::*;
    use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin};

    fn docs(entries: &[(&str, &[(&str, &str)])]) -> HashMap<String, Vec<(String, String)>> {
        entries
            .iter()
            .map(|(agent, documents)| {
                (
                    (*agent).to_string(),
                    documents
                        .iter()
                        .map(|(p, b)| ((*p).to_string(), (*b).to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    /// The property the whole axis exists for. The routing table is manifest
    /// data and does not move when an operator edits a note, so a
    /// name-only hash would leave the edit invisible until a restart.
    #[test]
    fn the_fingerprint_moves_when_a_documents_body_changes() {
        let before = routed_context_fingerprint(&docs(&[("ceo", &[("brief.md", "old")])]));
        let after = routed_context_fingerprint(&docs(&[("ceo", &[("brief.md", "new")])]));
        assert_ne!(
            before, after,
            "an edited routed note must rebuild the roster"
        );
    }

    /// A `HashMap` has no order, so an order-sensitive hash would drop every
    /// live agent session on a rebuild that changed nothing.
    #[test]
    fn the_fingerprint_is_stable_across_map_iteration_order() {
        let one = docs(&[
            ("ceo", &[("brief.md", "b")]),
            ("engineer", &[("claims.md", "c")]),
        ]);
        let two = docs(&[
            ("engineer", &[("claims.md", "c")]),
            ("ceo", &[("brief.md", "b")]),
        ]);
        assert_eq!(
            routed_context_fingerprint(&one),
            routed_context_fingerprint(&two)
        );
    }

    /// Renaming a document is a real change even when its text is identical:
    /// the persona quotes the path as the section heading.
    #[test]
    fn the_fingerprint_moves_when_a_document_is_renamed() {
        let before = routed_context_fingerprint(&docs(&[("ceo", &[("brief.md", "same")])]));
        let after = routed_context_fingerprint(&docs(&[("ceo", &[("GOAL.md", "same")])]));
        assert_ne!(before, after);
    }

    /// A company with no workspace store keeps a stable fingerprint and never
    /// rebuilds on this axis — the pre-routing behaviour exactly.
    #[tokio::test]
    async fn no_workspace_store_resolves_to_nothing() {
        let fx = fixture();
        assert!(fx.deps.workspace.is_none(), "fixture has no store wired");

        let pool = HarnessPool::new();
        let routed = pool.resolve_routed_context(&record(), &fx.deps, &[]).await;
        assert!(routed.is_empty(), "{routed:?}");
        assert_eq!(
            routed_context_fingerprint(&routed),
            routed_context_fingerprint(&HashMap::new()),
            "a company that routes nothing must not rebuild on this axis"
        );
    }

    /// The real path: a routed document that exists in the tree is read and
    /// keyed to the agent whose manifest asked for it.
    #[tokio::test]
    async fn a_routed_document_is_resolved_per_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws: Arc<dyn crate::ports::WorkspaceStore> =
            Arc::new(crate::store::FsOps::new(dir.path()));
        let company = CompanyId::new("acme");
        ws.create(
            &company,
            &WorkspaceNode {
                id: "n-brief".to_string(),
                name: "brief.md".to_string(),
                kind: NodeKind::File,
                parent_id: None,
                updated_at_millis: 1,
                created_by: WorkspaceOrigin::Operator,
                updated_by: WorkspaceOrigin::Operator,
                mime: None,
                size: None,
                sha256: None,
                adopted: false,
            },
            Some("What the company established."),
        )
        .await
        .expect("create");

        let mut fx = fixture();
        fx.deps.workspace = Some(ws);

        let pool = HarnessPool::new();
        let routed = pool.resolve_routed_context(&record(), &fx.deps, &[]).await;

        // Both fixture agents default to the `reasoning` row, which routes
        // BRIEF — so both resolve it, and neither invents the notes that do
        // not exist in the tree.
        for agent in ["ceo", "engineer"] {
            let documents = routed
                .get(agent)
                .unwrap_or_else(|| panic!("no routed documents for {agent}: {routed:?}"));
            assert_eq!(
                documents,
                &vec![(
                    "brief.md".to_string(),
                    "What the company established.".to_string()
                )],
                "{agent}"
            );
        }
    }
}

/// A provider double with its own, distinct telemetry identity — stands
/// in for a pinned agent's own `TenantProvider` sibling
/// ([`HarnessModel::pinned`]) without going through real pin resolution.
pub(super) struct PinnedProvider;

#[async_trait]
impl ChatModel<()> for PinnedProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ok"))
    }
}

impl HarnessModel for PinnedProvider {
    fn telemetry_provider_id(&self) -> String {
        "pinned-provider".to_string()
    }

    fn telemetry_model(&self) -> Option<crate::metering::ModelSlug> {
        Some(crate::metering::ModelSlug::OTHER)
    }
}

/// A provider double whose every call fails — stands in for a company
/// with no default configured at all (X12's "pinned-only" case), so a
/// caller of [`pass_model`] can be proven to fall back to the agent's own
/// pin instead of losing the pass outright.
pub(super) struct AlwaysFailsProvider;

#[async_trait]
impl ChatModel<()> for AlwaysFailsProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Err(tinyinference::Error::Model(
            "no company default configured".to_string(),
        ))
    }
}

impl HarnessModel for AlwaysFailsProvider {
    fn telemetry_provider_id(&self) -> String {
        "no-default".to_string()
    }
}

/// A model that plays back a scripted sequence of outcomes, one per
/// [`invoke`](ChatModel::invoke) call, so the empty-response retry wrapper can
/// be driven deterministically. `Ok("")` is the transient empty class (the
/// harness turn raises the empty-response error on a blank assistant reply);
/// `Err(_)` is a hard error.
pub(super) struct ScriptedProvider {
    pub(super) script: StdMutex<std::collections::VecDeque<Result<String, String>>>,
    pub(super) calls: std::sync::atomic::AtomicUsize,
    /// Usage stamped on every scripted `Ok` response, when the case needs a
    /// provider that reports any (`None` — the default — mirrors
    /// `MockProvider`, whose replies carry none at all). Only a response
    /// carrying usage makes openhuman publish the live
    /// `TurnCostUpdated` tally the metering path depends on.
    pub(super) usage: Option<tinyinference::Usage>,
    /// What an exhausted script answers: the default `"exhausted"` reply,
    /// or a permanent error.
    ///
    /// A case that needs the turn to *fail* has to script a provider that
    /// stays failed, because openhuman retries a provider error inside its
    /// own loop — a finite run of `Err` entries is simply consumed and the
    /// turn then succeeds on the fallback reply, which is how this
    /// scripting seam quietly turned a failure case into a passing one.
    pub(super) fail_when_exhausted: bool,
}

impl ScriptedProvider {
    fn new(outcomes: Vec<Result<String, String>>) -> Self {
        Self {
            script: StdMutex::new(outcomes.into_iter().collect()),
            calls: std::sync::atomic::AtomicUsize::new(0),
            usage: None,
            fail_when_exhausted: false,
        }
    }

    /// Report `usage` on every scripted `Ok` response.
    fn reporting_usage(mut self, usage: tinyinference::Usage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Fail every call past the end of the script, permanently.
    fn failing_when_exhausted(mut self) -> Self {
        self.fail_when_exhausted = true;
        self
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedProvider {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let with_usage = |reply: &str| {
            let mut response = ModelResponse::assistant(reply);
            response.usage = self.usage;
            response
        };
        match self.script.lock().unwrap().pop_front() {
            Some(Ok(reply)) => Ok(with_usage(&reply)),
            Some(Err(err)) => Err(tinyinference::Error::Model(err)),
            None if self.fail_when_exhausted => Err(tinyinference::Error::Model(
                "scripted provider is permanently down".to_string(),
            )),
            None => Ok(with_usage("exhausted")),
        }
    }
}

impl HarnessModel for ScriptedProvider {
    fn telemetry_provider_id(&self) -> String {
        "scripted".to_string()
    }
}

/// Build a single [`CompanyAgent`] over a scripted provider so the wrapper can
/// be exercised directly (its retry logic is the unit under test).
pub(super) fn scripted_agent(outcomes: Vec<Result<String, String>>) -> (Arc<CompanyAgent>, HarnessDeps) {
    scripted_agent_over(ScriptedProvider::new(outcomes))
}

/// As [`scripted_agent`], over an already-configured provider — the seam a
/// case that needs the provider to *report usage* builds through.
pub(super) fn scripted_agent_over(provider: ScriptedProvider) -> (Arc<CompanyAgent>, HarnessDeps) {
    let dir = tempfile::tempdir().expect("tempdir");
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(provider),
        provider_slug: "scripted".to_string(),
        serves: None,
        context: Arc::new(MockContext::default()),
        store: Arc::new(RecordingStore::default()),
        meter: None,
        workspace_root: dir.path().to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.path().to_path_buf(),
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
        delegations: DelegationQueue::default(),
        workflow_runner: crate::harness::orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_runs: None,
        deep_trace: None,
        workflow_revisions: None,
        approval_requests: ApprovalRequestQueue::default(),
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
    };
    let roster = build_roster(&record(), &deps, &[], &HashMap::new()).expect("roster");
    // Keep the tempdir alive for the agent's workspace by leaking it into the
    // test's lifetime — the process ends the test anyway.
    std::mem::forget(dir);
    (roster.into_iter().next().expect("one agent"), deps)
}

/// The leaf as the vendored harness actually writes it, taken verbatim from
/// the failing run in issue #1680.
pub(super) fn ceiling_error() -> anyhow::Error {
    anyhow::anyhow!(
        "tinyagents harness run failed; model error; run timed out; model call for run \
         'agent_turn' exceeded its remaining wall-clock budget (56636 ms)"
    )
}

/// The same failure as [`ceiling_error`], but arriving as an `anyhow`
/// context chain instead of one flattened string. Nothing guarantees the
/// vendored crate keeps flattening it, and `is_wall_clock_ceiling` already
/// assumes it might not.
pub(super) fn chained_ceiling_error() -> anyhow::Error {
    use anyhow::Context as _;
    Err::<(), _>(anyhow::anyhow!(
        "model call for run 'agent_turn' exceeded its remaining wall-clock budget (56636 ms)"
    ))
    .context("run timed out")
    .context("model error")
    .context("tinyagents harness run failed")
    .unwrap_err()
}

/// In-memory secret store so `ensure` can re-resolve the runtime MCP index.
#[derive(Default)]
pub(super) struct MemSecrets {
    pub(super) map: StdMutex<std::collections::HashMap<String, String>>,
}

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(
        &self,
        _c: &CompanyId,
        key: &str,
    ) -> crate::Result<Option<crate::ports::types::SecretValue>> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|v| crate::ports::types::SecretValue(v.clone())))
    }
    async fn set(
        &self,
        _c: &CompanyId,
        key: &str,
        value: crate::ports::types::SecretValue,
    ) -> crate::Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

/// An in-memory `SkillStateStore` whose delta set a test can mutate between
/// two `ensure` calls — the same way the console Skills tab authors, edits,
/// enables, or disables a skill — so the freshness gate can be observed
/// reacting with no restart.
#[derive(Default)]
pub(super) struct MemSkills {
    pub(super) deltas: StdMutex<Vec<SkillState>>,
}

#[async_trait]
impl SkillStateStore for MemSkills {
    async fn list(&self, _company: &CompanyId) -> crate::Result<Vec<SkillState>> {
        Ok(self.deltas.lock().unwrap().clone())
    }
    async fn set(&self, _company: &CompanyId, state: &SkillState) -> crate::Result<()> {
        let mut deltas = self.deltas.lock().unwrap();
        match deltas.iter_mut().find(|s| s.slug == state.slug) {
            Some(slot) => *slot = state.clone(),
            None => deltas.push(state.clone()),
        }
        Ok(())
    }
    async fn remove(&self, _company: &CompanyId, slug: &str) -> crate::Result<bool> {
        let mut deltas = self.deltas.lock().unwrap();
        let before = deltas.len();
        deltas.retain(|s| s.slug != slug);
        Ok(deltas.len() != before)
    }
}

/// A valid custom-skill delta (its `custom_doc` parses, so `materialize`
/// writes it to the scratch tree).
pub(super) fn custom_skill(slug: &str, enabled: bool, body: &str) -> SkillState {
    SkillState {
        slug: slug.to_string(),
        enabled,
        source: crate::ports::skills_state::SkillSource::Custom,
        custom_doc: Some(body.to_string()),
    }
}

pub(super) const STANDUP_MD: &str =
    "---\nname: Standup Digest\ndescription: Summarize the standup\n---\n\n# Standup Digest\n";

/// The scratch path a materialized skill lands at for the first roster agent
/// (`ceo`) under a company's workspace root.
fn skill_scratch(ws: &std::path::Path, slug: &str) -> std::path::PathBuf {
    ws.join("acme")
        .join("ceo")
        .join("skill-catalog")
        .join("skills")
        .join(slug)
        .join("SKILL.md")
}

/// A `CompanyStore` backed by a live, mutable record — so a test can mutate
/// it between two `ensure` calls the same way the console `POST .../team`
/// route or the orchestrator's `add_agent` tool would, and observe the
/// freshness gate react.
#[derive(Default)]
pub(super) struct LiveStore {
    pub(super) record: StdMutex<Option<CompanyRecord>>,
}

#[async_trait]
impl CompanyStore for LiveStore {
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

/// A manifest that grants every tool namespace, so the roster actually builds
/// the exec tools the capability filter then trims. (The default `manifest()`
/// grants nothing, so no exec tools would be present to gate.)
pub(super) fn granting_manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[tools]
allow = ["shell", "code", "web", "files"]

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction."
"#,
    )
    .expect("valid manifest")
}

pub(super) fn granting_record() -> CompanyRecord {
    CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest: granting_manifest(),
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        setup: None,
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
        name_confirmed: false,
        activation_completed_at: None,
        created_at_millis: None,
    }
}

/// The `ceo` roster agent's live tool names (test introspection via the
/// public `Agent::tools()` accessor).
pub(super) async fn ceo_tool_names(pool: &HarnessPool, id: &CompanyId) -> Vec<String> {
    let guard = pool.agents.read().await;
    let roster = guard.get(id).expect("roster present");
    let ceo = roster
        .iter()
        .find(|a| a.agent_id == "ceo")
        .expect("ceo present");
    let agent = ceo.agent.lock().await;
    agent.tools().iter().map(|t| t.name().to_string()).collect()
}

/// Builds a `HarnessDeps` carrying the given plan + meter, for the total-
/// ceiling dispatch tests (issue #188). Everything else is the inert fixture
/// wiring (mock provider/context, recording store).
pub(super) fn deps_with_plan(
    dir: &std::path::Path,
    context: Arc<MockContext>,
    meter: Option<Arc<dyn UsageMeter>>,
    plan: Option<crate::harness::capability_budget::CapabilityPlan>,
) -> HarnessDeps {
    HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context,
        store: Arc::new(RecordingStore::default()),
        meter,
        workspace_root: dir.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: dir.to_path_buf(),
        model_override: None,
        tasks: None,
        skills: None,
        skills_source_dir: None,
        skills_registry: std::sync::Arc::from([]),
        default_mcp_servers: Vec::new(),
        mcp_servers: Vec::new(),
        facts: None,
        events: None,
        delegations: DelegationQueue::default(),
        workflow_runner: crate::harness::orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: McpFailureQueue::default(),
        pending_publishes: crate::harness::publish::PendingPublishQueue::default(),
        workflow_refs: crate::harness::workflow_refs::WorkflowRefQueue::default(),
        run_outputs: crate::harness::orchestrator::RunOutputCache::default(),
        run_output_store: None,
        workflow_runs: None,
        deep_trace: None,
        workflow_revisions: None,
        approval_requests: ApprovalRequestQueue::default(),
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
        artifacts: None,
        steer: crate::company::steer::InflightRegistry::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
    }
}

/// A company whose `ceo` carries a $5/day cap and whose `engineer` carries
/// none — the pair that proves the gate is per-teammate, not per-company.
pub(super) fn capped_record() -> CompanyRecord {
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction."
budget_usd_daily = 5.0

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds the product."
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..record()
    }
}

/// A `$usd` inference sample for `agent`, stamped at `at_millis`.
pub(super) fn spend_sample(agent: &str, usd: f64, at_millis: u64) -> UsageSample {
    UsageSample {
        at_millis,
        agent: agent.into(),
        provider: "managed".into(),
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cost_usd: usd,
        kind: crate::ports::SampleKind::Inference,
        run_id: None,
        model: None,
    }
}

/// A company whose `treasurer` carries a `budget_usd_daily` of exactly
/// `0.0` — the value `validate_cap` in `server::ops::team` accepts as a
/// non-negative, finite number with no special-case.
pub(super) fn zero_capped_record() -> CompanyRecord {
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "treasurer"
role = "Treasurer"
description = "Handles spend."
budget_usd_daily = 0.0

[[agent]]
id = "engineer"
role = "Engineer"
description = "Builds the product."
"#,
    )
    .expect("valid manifest");
    CompanyRecord {
        manifest,
        ..record()
    }
}

/// Build one agent and return the tools it actually received.
///
/// A local mirror of `build`'s own `built_tool_names` — that one is private
/// to its test module, and this file owns `deps_with_plan`, which is the
/// expensive half.
pub(super) fn belt(grants: &[&str], is_orchestrator: bool, wire_everything: bool) -> Vec<String> {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut deps = deps_with_plan(dir.path(), Arc::new(MockContext::default()), None, None);
    if wire_everything {
        // The three tool families gated on a wired dependency rather than
        // on a cargo feature. Without these the belt is missing exactly the
        // tools most likely to be misclassified — the workspace writes and
        // the priced search.
        deps.workspace = Some(Arc::new(crate::store::FsOps::new(dir.path())));
        deps.artifacts = Some(Arc::new(crate::store::FsOps::new(dir.path())));
        deps.search = Some(crate::harness::search::SearchBackend::new(
            "https://api.example.test".to_string(),
            crate::company::credentials::Credential::from_value("managed-platform-token"),
            crate::company::DEFAULT_SEARCH_DAILY_CALLS,
        ));
        // A registered MCP server is what puts `mcp_list_servers`,
        // `mcp_list_tools` and `mcp_call_tool` on the belt — the three
        // tools issue #443 is about. Without one the coverage check would
        // pass while never having looked at them.
        // A skills source dir is what puts `list_skills`, `describe_skill`
        // and `read_skill_resource` on the belt (named for skills since
        // issue #845; upstream calls them `*_workflow*`). Leaving it `None`
        // is how those three stayed invisible to this check while
        // `describe_workflow` parked in production.
        let company_src = dir.path().join("company-src");
        std::fs::create_dir_all(company_src.join("skills").join("brief")).expect("skill dir");
        std::fs::write(
            company_src.join("skills").join("brief").join("SKILL.md"),
            "---\nname: brief\ndescription: Write a brief\n---\n\nWrite one.\n",
        )
        .expect("skill file");
        deps.skills_source_dir = Some(company_src);
        deps.mcp_servers = vec![McpServerDecl {
            name: "notes".to_string(),
            endpoint: "https://mcp.example.test".to_string(),
            description: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            read_only_tools: Vec::new(),
            timeout_secs: 30,
            enabled: true,
            source: crate::company::mcp::McpSource::Runtime,
            auth: crate::company::mcp::AuthMaterial::None,
        }];
    }
    let manifest_agent = ManifestAgent {
        provider: None,
        global: false,
        id: "desk".to_string(),
        role: "Desk Lead".to_string(),
        name: None,
        description: None,
        tier: None,
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
    };
    let policy = ApprovalPolicy::new(&Policy::default(), None);
    let grants: Vec<String> = grants.iter().map(|g| g.to_string()).collect();
    let agent = build::build_agent(
        &CompanyId::new("acme"),
        "Acme",
        &manifest_agent,
        policy,
        &deps,
        &grants,
        &[],
        &[],
        None,
        is_orchestrator,
        /* speech_enabled */ false,
    )
    .expect("agent builds");
    agent.tools().iter().map(|t| t.name().to_string()).collect()
}

/// A secret store that reads back what was seeded, or fails every read.
#[cfg(any(feature = "chargebee", feature = "paypal"))]
#[derive(Default)]
pub(super) struct BillingSecrets {
    pub(super) map: StdMutex<std::collections::HashMap<String, String>>,
    pub(super) fail: bool,
}

#[cfg(any(feature = "chargebee", feature = "paypal"))]
#[async_trait]
impl SecretStore for BillingSecrets {
    async fn get(
        &self,
        _c: &CompanyId,
        key: &str,
    ) -> crate::Result<Option<crate::ports::types::SecretValue>> {
        if self.fail {
            return Err(crate::error::OpenCompanyError::Store(
                "the secret store is unreachable".into(),
            ));
        }
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|v| crate::ports::types::SecretValue(v.clone())))
    }
    async fn set(
        &self,
        _c: &CompanyId,
        key: &str,
        value: crate::ports::types::SecretValue,
    ) -> crate::Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

/// A company whose manifest allows exactly `grants`.
#[cfg(any(feature = "chargebee", feature = "paypal"))]
pub(super) fn record_granting(grants: &[&str]) -> CompanyRecord {
    let mut rec = record();
    rec.manifest.tools.allow = grants.iter().map(|g| g.to_string()).collect();
    rec
}

/// The inert fixture deps, with a secret store and a "last known" connection.
#[cfg(any(feature = "chargebee", feature = "paypal"))]
pub(super) fn billing_deps(dir: &std::path::Path, secrets: Arc<dyn SecretStore>) -> HarnessDeps {
    let mut deps = deps_with_plan(dir, Arc::new(MockContext::default()), None, None);
    deps.secrets = Some(secrets);
    deps
}

#[cfg(feature = "chargebee")]
pub(super) async fn pool_resolve_chargebee(
    deps: &HarnessDeps,
) -> Option<crate::harness::chargebee::TenantChargebee> {
    HarnessPool::new()
        .resolve_chargebee(&record_granting(&["chargebee"]), deps)
        .await
}
