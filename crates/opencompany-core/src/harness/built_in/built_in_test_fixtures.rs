//! Shared fixtures for `built_in`'s own inline tests (part 1 of 2):
//! mock stores, scripted providers, and the `Fixture`/`manifest`/`record`
//! builders test groups reach for. Split out of the inline `mod tests` block,
//! and split further across files because the combined fixtures exceeded the
//! 750-line file limit.

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
