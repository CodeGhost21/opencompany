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

#[tokio::test]
async fn operator_message_gets_an_agent_reply() {
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock(dir.path());
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "status?".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(result.channel_responses[0].channel, "operator");
    // The mock provider prefixes the routed message, proving the turn ran
    // through the openhuman agent rather than an echo.
    assert!(
        result.channel_responses[0].text.contains("status?"),
        "{:?}",
        result.channel_responses[0].text
    );
    // The offline mock runs no tools and emits no progress, so the operator
    // bubble carries zero steps — the tell that distinguishes a tool-less
    // (here, memory/echo-style) answer from a tool-backed one.
    assert!(
        result.channel_responses[0].steps.is_empty(),
        "a tool-less turn carries no steps: {:?}",
        result.channel_responses[0].steps
    );
    assert_eq!(result.new_traces.len(), 1);
    // Single cost-accounting site: the cycle result carries no ledger delta.
    assert!(result.ledger_deltas.is_empty());
}

#[tokio::test]
async fn schedule_fired_gets_an_agent_reply() {
    // A cron tick (`ScheduleFired`) must drive a real turn and surface its
    // reply, not fall through the match and vanish — the same guarantee an
    // operator message gets. Without this arm a scheduled prompt ran to
    // nowhere: the turn produced an answer that was never journaled, so the
    // desk history had no record it fired.
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock(dir.path());
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::ScheduleFired {
                cron: "0 9 * * *".into(),
                prompt: "daily standup".into(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 1);
    // The mock provider prefixes the routed prompt, proving the turn ran
    // through the agent rather than falling through the match.
    assert!(
        result.channel_responses[0].text.contains("daily standup"),
        "{:?}",
        result.channel_responses[0].text
    );
    assert_eq!(result.new_traces.len(), 1);
}

/// A cron tick's reply is journaled onto the General desk under the
/// responder's name, and the returned bubble is stamped with the journaled
/// event's sequence — the same contract an operator reply's bubble carries
/// (issue #885: destination and author are separate facts).
#[tokio::test]
async fn schedule_fired_journals_an_agent_reply_on_the_general_desk() {
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let brain = brain_with_queue_and_events(dir.path(), Default::default(), log.clone());
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::ScheduleFired {
                cron: "0 9 * * *".into(),
                prompt: "daily standup".into(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    // The reply lands on the General desk, authored by the responder —
    // destination and author stay separate.
    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    assert_eq!(bubble.channel, crate::server::ops::language::DEFAULT_DESK);
    assert_eq!(bubble.agent.as_deref(), Some("ceo"));

    // The journal holds one AgentReply, on the General desk, attributed to
    // the responder — not to the channel the reply was routed over.
    let events = log
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    let reply = events
        .iter()
        .find(|e| matches!(&e.event, CompanyEvent::AgentReply { .. }))
        .expect("a scheduled reply was journaled");
    match &reply.event {
        CompanyEvent::AgentReply {
            chat_id,
            agent_id,
            text,
            ..
        } => {
            assert_eq!(chat_id, crate::server::ops::language::DEFAULT_DESK);
            assert_eq!(agent_id, "ceo");
            assert!(text.contains("daily standup"), "{text}");
        }
        _ => unreachable!(),
    }

    // The returned bubble carries the appended event's sequence as its
    // durable id.
    assert_eq!(bubble.message_id, Some(reply.seq.value().to_string()));
}

#[tokio::test]
async fn schedule_fired_journals_halt_notices() {
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let outcome = crate::harness::built_in::TurnOutcome {
        reply: "checkpoint".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: true,
        // Test fixture, not the ACP fold (PR #1880 review).
        abnormal_stop: None,
        halted_for_spend: Some(crate::harness::SpendHalt {
            agent: "ceo".to_string(),
            spent_usd: 1.25,
            cap_usd: 1.0,
        }),
        // This fixture scripts a SPEND halt; a budget pause is the separate
        // signal added in issue #1846 and is not what it exercises.
        budget_paused: None,
    };
    let brain = brain_with_queue_and_events(dir.path(), Default::default(), log.clone())
        .with_default_engine(Some(Arc::new(FixedOutcomeTurn {
            outcome,
            approval_requests: None,
        })));
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::ScheduleFired {
                cron: "0 9 * * *".into(),
                prompt: "daily standup".into(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 3);
    assert!(
        result.channel_responses[1]
            .text
            .contains("maximum number of steps")
    );
    assert!(result.channel_responses[2].text.contains("spend cap"));
    assert!(
        result
            .channel_responses
            .iter()
            .skip(1)
            .all(|response| response.agent.as_deref() == Some(crate::ports::SYSTEM_AUTHOR))
    );
    let events = log
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    let replies: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.event, CompanyEvent::AgentReply { .. }))
        .collect();
    assert_eq!(replies.len(), 3, "all scheduled notices are durable");
}

#[tokio::test]
async fn schedule_fired_journals_a_budget_pause_notice() {
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let outcome = crate::harness::built_in::TurnOutcome {
        reply: "checkpoint".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        // Test fixture, not the ACP fold (PR #1880 review).
        abnormal_stop: None,
        // Issue #1906: this fixtures a BUDGET pause, not a spend halt — the
        // halt sibling is pinned by `schedule_fired_journals_halt_notices`.
        halted_for_spend: None,
        budget_paused: Some(crate::harness::BudgetPause {
            agent: "ceo".to_string(),
            summary: "the provider is exhausted".to_string(),
        }),
    };
    let brain = brain_with_queue_and_events(dir.path(), Default::default(), log.clone())
        .with_default_engine(Some(Arc::new(FixedOutcomeTurn {
            outcome,
            approval_requests: None,
        })));
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::ScheduleFired {
                cron: "0 9 * * *".into(),
                prompt: "daily standup".into(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    // Issue #1906: a scheduled tick that pauses for lack of credits must
    // not present the interrupted turn as a completed answer — the primary
    // bubble carries the pause placeholder and a system notice follows it.
    assert_eq!(result.channel_responses.len(), 2);
    assert_eq!(
        result.channel_responses[0].text,
        BUDGET_PAUSED_PLACEHOLDER_REPLY
    );
    assert!(
        result.channel_responses[1]
            .text
            .starts_with(BUDGET_PAUSE_NOTICE_PREFIX)
    );
    assert!(
        result
            .channel_responses
            .iter()
            .skip(1)
            .all(|response| response.agent.as_deref() == Some(crate::ports::SYSTEM_AUTHOR))
    );
    let events = log
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    let replies: Vec<_> = events
        .iter()
        .filter(|event| matches!(event.event, CompanyEvent::AgentReply { .. }))
        .collect();
    assert_eq!(
        replies.len(),
        2,
        "the pause placeholder and notice are durable"
    );
}

#[tokio::test]
async fn schedule_fired_journals_approval_overflow_notice() {
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    let brain = brain_with_queue_and_events(dir.path(), requests.clone(), log.clone())
        .with_default_engine(Some(Arc::new(FixedOutcomeTurn {
            outcome: crate::harness::built_in::TurnOutcome {
                reply: "checkpoint".to_string(),
                steps: Vec::new(),
                hit_iteration_cap: false,
                // Test fixture, not the ACP fold (PR #1880 review).
                abnormal_stop: None,
                halted_for_spend: None,
                // Added by #1846 after these fixtures were written.
                budget_paused: None,
            },
            approval_requests: Some(requests.clone()),
        })));
    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::ScheduleFired {
                cron: "0 9 * * *".into(),
                prompt: "daily standup".into(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 2);
    let notice = &result.channel_responses[1];
    assert!(
        notice.text.contains("further gated tool call"),
        "{}",
        notice.text
    );
    assert_eq!(notice.agent.as_deref(), Some(crate::ports::SYSTEM_AUTHOR));
    assert_eq!(requests.queued(), 0);
    let events = log
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(events.iter().any(|event| {
        matches!(&event.event, CompanyEvent::AgentReply { text, .. } if text.contains("further gated tool call"))
    }));
}

#[tokio::test]
async fn no_events_still_acknowledges() {
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock(dir.path());
    let result = brain
        .run_cycle(request(Vec::new()), &NoopHost)
        .await
        .expect("cycle runs");
    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(result.channel_responses[0].text, "Acknowledged.");
    // Issue #966, asserted here rather than only on `system_notice`: this
    // drives the real cycle, so it pins that the fallback *calls* the
    // constructor. Asserting the constructor alone leaves the call site free
    // to go back to an inline bubble with no author, which is the shape that
    // caused the defect.
    assert_eq!(
        result.channel_responses[0].agent.as_deref(),
        Some(crate::ports::SYSTEM_AUTHOR),
        "the runtime's own fallback is authored by the runtime, not by its destination"
    );
}

#[test]
fn responder_defaults_to_first_roster_agent() {
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock(dir.path());
    assert_eq!(brain.responder, "ceo");
    let brain = brain.with_responder("cfo");
    assert_eq!(brain.responder, "cfo");
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
fn brain_with_tasks_notified(
    dir: &std::path::Path,
    notify: bool,
) -> (HarnessBrain, Arc<FsOps>) {
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

/// **The regression.** Issue #1846 review (Codex #3864988168): `run_task`
/// never inspected `outcome.budget_paused` before this fix — a dispatched
/// card whose model call ran out of credits fell straight into the
/// `None => { ... None => settle(Completed) }` arm, since a budget pause
/// carries an `Ok(TurnOutcome)` with no delegation queued, and landed in
/// `in_review` looking like a finished, reviewable result instead of the
/// graceful pause the operator-chat path already gave the same failure.
#[tokio::test]
async fn a_dispatched_tasks_budget_exhaustion_pauses_rather_than_completes() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks_and_budget_exhausted_provider(dir.path());
    let company = CompanyId::new("acme");
    tasks
        .upsert(&company, &card("t-1", "engineer"))
        .await
        .expect("seed");
    // Driven directly, not through `run_cycle`, so the roster has to be
    // built explicitly — see
    // `dispatched_card_with_an_origin_stops_in_review_and_still_posts_back`.
    brain
        .pool
        .ensure(&brain.record(), &brain.deps)
        .await
        .expect("roster");

    brain.run_task("t-1", None).await.expect("run");

    let settled = only_card(&tasks).await;
    assert_eq!(
        settled.column, COLUMN_PAUSED,
        "a budget-exhausted model call is a graceful pause, not a completed result — \
         it must not read on the board as a finished, reviewable card"
    );
    let note = settled.note.expect("note");
    assert!(
        note.contains("add credits") || note.contains("Add credits"),
        "the note must carry the actionable ask a genuine budget pause gives, not just \
         an opaque dispatch failure: {note}"
    );
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

fn brain_with_stores(
    dir: &std::path::Path,
    with_workspace: bool,
) -> (HarnessBrain, Arc<FsOps>) {
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
    async fn get(
        &self,
        company: &CompanyId,
        id: &str,
    ) -> crate::Result<Option<ArtifactRecord>> {
        crate::ports::artifacts::ArtifactStore::get(&*self.inner, company, id).await
    }
    async fn upsert(
        &self,
        company: &CompanyId,
        artifact: &ArtifactRecord,
    ) -> crate::Result<()> {
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
async fn note_in_tree(
    ops: &FsOps,
    company: &CompanyId,
    name: &str,
) -> Option<(String, String)> {
    use crate::ports::workspace::WorkspaceStore;
    let nodes = WorkspaceStore::tree(ops, company).await.unwrap();
    let found = nodes.iter().find(|n| n.name == name)?;
    let (_, body) = WorkspaceStore::read(ops, company, &found.id)
        .await
        .unwrap()?;
    Some((found.id.clone(), body))
}

/// **Chain first, proven.** A re-publish whose artifact write fails must
/// leave the note holding the PREVIOUS body — the version was stored before
/// the tree was touched, so a refused version means an untouched tree.
///
/// The opposite ordering is what this rules out, and it is not a stylistic
/// difference: a note one version ahead of the chain shows the operator
/// content the version history has no record of, which makes
/// `human_edit_diff` quietly wrong rather than loudly broken — the same rot
/// the artifact port exists to prevent, arriving through the tree instead.
#[tokio::test]
async fn a_refused_republish_leaves_the_note_on_the_previous_body() {
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let ops = Arc::new(FsOps::new(dir.path()));
    // v1 costs two upserts: the record, then the link once the node exists.
    let artifacts = FailingArtifacts::new(ops.clone(), 2);
    let (brain, _) =
        brain_with_injected_artifacts(dir.path(), ops.clone(), artifacts.clone(), true);
    let company = CompanyId::new("acme");
    let c = card("t-1", "maya");

    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v1")], None)
        .await
        .expect("the first publish lands");
    let (node_id, body) = note_in_tree(&ops, &company, "launch.md")
        .await
        .expect("v1 is in the tree");
    assert_eq!(body, "v1");

    // Now the artifact store refuses. The re-publish must fail *before*
    // reaching the tree.
    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v2")], None)
        .await
        .expect_err("a refused artifact write fails the publish");

    let (still, body) = note_in_tree(&ops, &company, "launch.md")
        .await
        .expect("the note is still there");
    assert_eq!(still, node_id, "no rival note was minted");
    assert_eq!(
        body, "v1",
        "the tree must not hold a body the version history never recorded"
    );
    // And the chain is unchanged too — one version, not a half-written two.
    let stored = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(stored[0].versions.len(), 1);
    assert_eq!(stored[0].latest().unwrap().body, "v1");
}

/// The same ordering on a **fresh** publish: an artifact write that fails
/// creates nothing in the tree at all.
///
/// This is what makes the fresh path's residual an *orphan note* rather
/// than a lost deliverable — a node is only ever created for a deliverable
/// that is already recorded, so this path cannot leave a file in the tree
/// with no artifact behind it.
#[tokio::test]
async fn a_refused_first_publish_creates_nothing_in_the_tree() {
    use crate::ports::workspace::WorkspaceStore;

    let dir = tempfile::tempdir().unwrap();
    let ops = Arc::new(FsOps::new(dir.path()));
    let artifacts = FailingArtifacts::new(ops.clone(), 0);
    let (brain, _) = brain_with_injected_artifacts(dir.path(), ops.clone(), artifacts, true);
    let company = CompanyId::new("acme");

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![publish_of("launch.md", "v1")],
            None,
        )
        .await
        .expect_err("a refused artifact write fails the publish");

    assert!(
        note_in_tree(&ops, &company, "launch.md").await.is_none(),
        "no note may exist for a deliverable that was never recorded"
    );
    assert!(
        WorkspaceStore::tree(&*ops, &company)
            .await
            .unwrap()
            .is_empty(),
        "not even the agent's folder is minted for a publish that failed"
    );
}

/// The fresh path's one residual, and its repair.
///
/// A fresh publish has no node id to inherit, so v1 is stored unlinked and
/// a *second* artifact write stamps the link. If that second write fails,
/// both surfaces hold the body and only the pointer between them is
/// missing. That is deliberately warned-and-tolerated rather than fatal:
/// failing would discard the rest of the batch to report a link that the
/// next publish repairs.
///
/// The repair is the half worth proving. `materialize` find-or-creates by
/// path, so the next publish of the same source **re-adopts the very same
/// note** rather than duplicating it — which is what makes the orphan
/// self-healing rather than permanent.
#[tokio::test]
async fn an_unlinked_first_publish_is_repaired_by_the_next_one() {
    use crate::ports::artifacts::ArtifactStore;
    use crate::ports::workspace::WorkspaceStore;

    let dir = tempfile::tempdir().unwrap();
    let ops = Arc::new(FsOps::new(dir.path()));
    // Exactly one upsert succeeds: the record lands, the link does not.
    let artifacts = FailingArtifacts::new(ops.clone(), 1);
    let (brain, _) =
        brain_with_injected_artifacts(dir.path(), ops.clone(), artifacts.clone(), true);
    let company = CompanyId::new("acme");
    let c = card("t-1", "maya");

    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v1")], None)
        .await
        .expect("a missing link must not fail the publish");

    // Both surfaces hold the body; only the pointer is absent.
    let (orphan, body) = note_in_tree(&ops, &company, "launch.md")
        .await
        .expect("the note was still written");
    assert_eq!(body, "v1");
    let stored = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(stored[0].latest().unwrap().body, "v1");
    assert_eq!(
        stored[0].workspace_node_id(),
        None,
        "this is the orphan: recorded and written, but not linked"
    );

    // The next publish of the same source repairs it.
    artifacts.heal();
    let nodes_before = WorkspaceStore::tree(&*ops, &company).await.unwrap().len();
    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v2")], None)
        .await
        .expect("the repairing publish lands");

    let stored = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(stored[0].versions.len(), 2, "one record, extended");
    assert_eq!(
        stored[0].workspace_node_id(),
        Some(orphan.as_str()),
        "the very same note is re-adopted, which is what makes the orphan self-healing"
    );
    assert_eq!(
        WorkspaceStore::tree(&*ops, &company).await.unwrap().len(),
        nodes_before,
        "re-adoption, not duplication: no rival note beside the orphan"
    );
    assert_eq!(
        WorkspaceStore::read(&*ops, &company, &orphan)
            .await
            .unwrap()
            .unwrap()
            .1,
        "v2"
    );
}

/// The ordinary re-publish stores **once**, not twice. The second artifact
/// write exists only for a link that actually changed — a fresh publish, or
/// a note the operator deleted — and a re-publish that reuses its note has
/// nothing to restate.
#[tokio::test]
async fn an_ordinary_republish_writes_the_artifact_once() {
    let dir = tempfile::tempdir().unwrap();
    let ops = Arc::new(FsOps::new(dir.path()));
    let artifacts = FailingArtifacts::new(ops.clone(), usize::MAX);
    let (brain, _) =
        brain_with_injected_artifacts(dir.path(), ops.clone(), artifacts.clone(), true);
    let c = card("t-1", "maya");

    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v1")], None)
        .await
        .unwrap();
    // v1: the record, then the link once the node id exists.
    assert_eq!(
        artifacts.seen.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "a fresh publish stores the record, then stamps the link"
    );

    brain
        .record_published_artifacts(&c, "maya", vec![publish_of("launch.md", "v2")], None)
        .await
        .unwrap();
    assert_eq!(
        artifacts.seen.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "a re-publish inherits its note, so one store is enough"
    );
}
// -- issue #552: a published deliverable reaches the shared workspace -----

/// The headline of #552. A published file used to reach the artifact store
/// and stop, which left it visible only in the Artifacts tab of one card.
/// It must now also land in the shared tree, under the publishing agent's
/// own folder, attributed to that agent — and the version that wrote it
/// must carry the node id, which is the link the console's cross-link and
/// every later mirror depend on.
#[tokio::test]
async fn a_publish_lands_in_the_shared_workspace_and_the_version_names_the_node() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;
    use crate::ports::workspace::{WorkspaceOrigin, WorkspaceStore};

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts_and_workspace(dir.path());
    let company = CompanyId::new("acme");

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: "maya".to_string(),
                source: "specs/launch.md".to_string(),
                title: "Launch spec".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text(
                    "the spec body".to_string(),
                ),
            }],
            Some("run-1"),
        )
        .await
        .expect("records");

    let listed = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    let node_id = listed[0]
        .workspace_node_id()
        .expect("the version must name the node its body was mirrored into");

    let (node, body) = WorkspaceStore::read(&*ops, &company, node_id)
        .await
        .unwrap()
        .expect("the node exists in the shared tree");
    assert_eq!(body, "the spec body");
    assert_eq!(node.name, "launch.md");
    assert_eq!(
        node.created_by,
        WorkspaceOrigin::Agent {
            id: "maya".to_string()
        },
        "the tree must say which teammate produced this"
    );
}

/// The "zero tool work" claim in #552, proven rather than asserted: a
/// second agent reads the first agent's deliverable through the ordinary
/// `workspace_read` path, with nothing published-specific involved.
///
/// The read goes through the *same* index-and-resolve the tool uses (a
/// company-scoped `tree()` then a `read()` by id), so what this pins is
/// that the node is reachable by path from the shared tree — which is
/// exactly what makes it readable by every teammate.
#[tokio::test]
async fn a_second_agent_can_read_what_the_first_published() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::workspace::WorkspaceStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts_and_workspace(dir.path());
    let company = CompanyId::new("acme");

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: "maya".to_string(),
                source: "launch.md".to_string(),
                title: "Launch spec".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text(
                    "what maya produced".to_string(),
                ),
            }],
            None,
        )
        .await
        .expect("records");

    // Agent B knows nothing about the artifact. It walks the shared tree by
    // path, exactly as `workspace_list` / `workspace_read` do.
    let nodes = WorkspaceStore::tree(&*ops, &company).await.unwrap();
    let name_of = |id: &str| nodes.iter().find(|n| n.id == id).map(|n| n.name.clone());
    let found = nodes
        .iter()
        .find(|n| {
            n.name == "launch.md"
                && n.parent_id
                    .as_deref()
                    .and_then(name_of)
                    // Issue #1687: the task folder is named for the work
                    // and keyed by the card id — `<title>.<id>`, not the
                    // bare id — so browsing by path lands on that name.
                    .is_some_and(|parent| parent == "ship-the-thing.t-1")
        })
        .expect("agent B finds the deliverable by browsing the shared tree");

    let (_, body) = WorkspaceStore::read(&*ops, &company, &found.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body, "what maya produced");
}

/// A re-publish revises the SAME node rather than opening a rival beside
/// it, so the operator's open tab and any link to it keep working — and
/// the new version carries the same node id, which is what lets the next
/// re-publish find it again.
#[tokio::test]
async fn a_republish_updates_the_same_node() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;
    use crate::ports::workspace::WorkspaceStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts_and_workspace(dir.path());
    let company = CompanyId::new("acme");
    let c = card("t-1", "maya");
    let publish = |body: &str| PendingPublish {
        agent: "maya".to_string(),
        source: "specs/launch.md".to_string(),
        title: "Launch spec".to_string(),
        kind: crate::ports::artifacts::ArtifactKind::Markdown,
        note: None,
        payload: crate::harness::publish::PublishPayload::Text(body.to_string()),
    };

    brain
        .record_published_artifacts(&c, "maya", vec![publish("v1")], Some("run-1"))
        .await
        .unwrap();
    let first_node = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap()[0]
        .workspace_node_id()
        .expect("v1 named a node")
        .to_string();
    let tree_before = WorkspaceStore::tree(&*ops, &company).await.unwrap().len();

    brain
        .record_published_artifacts(&c, "maya", vec![publish("v2")], Some("run-2"))
        .await
        .unwrap();

    let record = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap()[0]
        .clone();
    assert_eq!(record.versions.len(), 2, "one record, two versions");
    assert_eq!(
        record.workspace_node_id(),
        Some(first_node.as_str()),
        "the second version must name the node the first one already had"
    );
    assert_eq!(
        WorkspaceStore::tree(&*ops, &company).await.unwrap().len(),
        tree_before,
        "a re-publish must create no new nodes"
    );
    assert_eq!(
        WorkspaceStore::read(&*ops, &company, &first_node)
            .await
            .unwrap()
            .unwrap()
            .1,
        "v2",
        "the node holds the current body"
    );
}

/// An unwired workspace must behave **exactly** as before this cell: the
/// artifact is recorded, nothing is attempted against a tree that does not
/// exist, and no version claims a node.
///
/// This is what keeps every pre-#552 publish test honest, since they all
/// run on this path.
#[tokio::test]
async fn without_a_workspace_store_the_publish_path_is_unchanged() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: "maya".to_string(),
                source: "specs/launch.md".to_string(),
                title: "Launch spec".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("body".to_string()),
            }],
            None,
        )
        .await
        .expect("the artifact is still recorded");

    let listed = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].workspace_node_id(),
        None,
        "with no tree to mirror into, a version names no node"
    );
}

/// A deliverable is never dropped for tree bookkeeping. When the node
/// cannot be written — here an operator's *file* squatting the `Artifacts`
/// root name, which the fail-closed resolver refuses rather than guesses —
/// the artifact is still recorded, just without a node id.
///
/// The opposite behaviour (propagating the error) would lose an explicitly
/// published file because a folder could not be made, which is the worse
/// of the two failures by a wide margin.
#[tokio::test]
async fn a_failed_node_write_still_records_the_artifact() {
    use crate::company::workspace_scaffold::ARTIFACTS_ROOT;
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;
    use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts_and_workspace(dir.path());
    let company = CompanyId::new("acme");

    // A *file* named `Artifacts` at the workspace root — the root a publish
    // now resolves through. The minter refuses to resolve a folder through
    // it rather than clobbering an operator's note.
    WorkspaceStore::create(
        &*ops,
        &company,
        &WorkspaceNode {
            id: crate::ports::generate_id(),
            name: ARTIFACTS_ROOT.to_string(),
            kind: NodeKind::File,
            parent_id: None,
            updated_at_millis: now_millis(),
            created_by: WorkspaceOrigin::Operator,
            updated_by: WorkspaceOrigin::Operator,
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        Some("an operator's note, in the way"),
    )
    .await
    .unwrap();

    let written = brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: "maya".to_string(),
                source: "launch.md".to_string(),
                title: "Launch spec".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text(
                    "the deliverable".to_string(),
                ),
            }],
            None,
        )
        .await
        .expect("a tree that refuses must not fail the publish");

    assert_eq!(written.len(), 1, "the deliverable is still recorded");
    let listed = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(listed[0].latest().unwrap().body, "the deliverable");
    assert_eq!(
        listed[0].workspace_node_id(),
        None,
        "no node was written, so no version may claim one"
    );
}
fn card(id: &str, assignee: &str) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        title: TaskTitle::authored("Ship the thing"),
        note: None,
        column: "in_progress".to_string(),
        priority: "high".to_string(),
        assignee: assignee.to_string(),
        updated_at_millis: 0,
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

// ── Issue #151 §3.2: a finished card answers where it was asked ──────

// The post-back's *text* rules — title, landing status, note folding,
// whitespace-only notes — moved with the renderer to
// `crate::harness::lifecycle` (issue #186), which owns them now and covers
// each case plus the new assignee-credit rule. What stays here is the
// wiring: that `run_task` reaches the relay at all, and attributes it to
// the orchestrator.

/// The compatibility guarantee: a card with no remembered origin — one made
/// straight on the board, or written before `origin_chat_id` existed —
/// posts back nowhere and behaves exactly as it did before.
#[tokio::test]
async fn a_card_with_no_origin_posts_back_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    // A roster assignee: since #205 an off-roster one is refused outright,
    // which would satisfy the no-post-back assertion below without ever
    // running the dispatch this test is about.
    let mut c = card("t-no-origin", "engineer");
    c.origin = TaskOrigin::new(None, None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    let posted = brain.run_task("t-no-origin", None).await.expect("run");
    assert!(
        posted.is_none(),
        "a card with no originating thread must not post back"
    );
    // The note is still the durable record.
    assert!(only_card(&tasks).await.note.is_some());
}

/// …and one that does remember its origin answers there, threaded with
/// `reply_to` and — since issue #186 — attributed to the **orchestrator**
/// rather than to the assignee that did the work.
#[tokio::test]
async fn a_card_with_an_origin_posts_back_to_that_thread() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    // A roster assignee, deliberately: an off-roster one falls back to the
    // default responder (`task_responder`), which in this fixture *is* the
    // orchestrator — so the credit would be correctly suppressed and this
    // test would prove nothing about the one-voice relay.
    let mut c = card("t-origin", "engineer");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    let posted = brain
        .run_task("t-origin", None)
        .await
        .expect("run")
        .expect("a card with an origin must post back");
    assert_eq!(
        posted.reply_to.as_ref().map(|r| r.chat_id.as_str()),
        Some("strategy")
    );
    // Issue #186: one voice. The bubble belongs to the orchestrator, and
    // the assignee that ran the card is credited in the text instead of
    // speaking to the operator directly.
    assert_eq!(
        posted.channel,
        brain.orchestrator(),
        "the orchestrator relays a finished card, not the assignee"
    );
    assert_ne!(
        posted.channel, "engineer",
        "the assignee must not address the operator directly"
    );
    assert!(posted.text.contains("Ship the thing"), "{}", posted.text);
    assert!(
        posted.text.contains("engineer"),
        "the relay must still credit who did the work: {}",
        posted.text
    );
    // A dispatched card discards its steps into the note.
    assert!(posted.steps.is_empty());
}

/// Issue #1890 B: and the **terminal** carries both halves of that origin.
///
/// The relay bubble above answers in the origin thread on its own; the
/// marker is the structural half, and it is the one that was landing in the
/// wrong place. `desk` is a responder id and a channel is a desk id, so
/// nothing on this event could recover either half — it is captured off the
/// card at the single settle emission point every dispatch ending passes
/// through, which is why capturing it there cannot miss a path.
#[tokio::test]
async fn a_settled_card_journals_the_thread_it_was_raised_in() {
    use crate::ports::EventSeq;
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks, events) = brain_with_tasks_and_events(dir.path());
    let mut c = card("t-threaded", "engineer");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), Some(EventSeq::new(41)));
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    brain.run_task("t-threaded", None).await.expect("run");

    let logged = events
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    let terminal = logged
        .iter()
        .find_map(|e| match &e.event {
            CompanyEvent::DeskTaskCompleted {
                origin_chat_id,
                origin_parent,
                ..
            } => Some((origin_chat_id.clone(), *origin_parent)),
            _ => None,
        })
        .expect("the settle journals a terminal");
    assert_eq!(
        terminal,
        (Some("strategy".to_string()), Some(EventSeq::new(41))),
        "the terminal carries the channel AND the thread the card recorded",
    );
}

/// …and a card raised at channel level still settles flat there. `None` is
/// the channel-level conversation, not a lost id, and a marker that started
/// threading itself onto an unrelated root would be worse than no marker.
#[tokio::test]
async fn a_settled_channel_level_card_journals_no_thread() {
    use crate::ports::EventSeq;
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks, events) = brain_with_tasks_and_events(dir.path());
    let mut c = card("t-flat", "engineer");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    brain.run_task("t-flat", None).await.expect("run");

    let logged = events
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(
        logged.iter().any(|e| matches!(
            &e.event,
            CompanyEvent::DeskTaskCompleted {
                origin_chat_id,
                origin_parent: None,
                ..
            } if origin_chat_id.as_deref() == Some("strategy")
        )),
        "an unthreaded settle names its channel and no thread: {logged:?}"
    );
}

async fn only_card(tasks: &Arc<FsOps>) -> TaskRecord {
    tasks
        .list(&CompanyId::new("acme"))
        .await
        .expect("list")
        .into_iter()
        .next()
        .expect("one card")
}

/// A dispatched **board-created** card (no `origin_chat_id`) runs a turn and
/// moves to `in_review` — the operator who made it is the reviewer — with
/// its result folded into the note under the responder that ran it.
#[tokio::test]
async fn task_dispatch_runs_and_moves_to_in_review() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", ""))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let moved = only_card(&tasks).await;
    assert_eq!(moved.column, "in_review");
    let note = moved.note.expect("result written to note");
    // Default responder (first roster agent) ran it, and the mock provider
    // echoes the instruction (the card title) back into the reply.
    assert!(note.contains("[ceo]"), "{note:?}");
    assert!(note.contains("Ship the thing"), "{note:?}");
}

// ── Issue #337: every finished card stops for a person ────────────────

/// **Rewritten by #337.** This used to pin the opposite: a card spawned by
/// a delegating turn (so it carries an `origin_chat_id`) completed straight
/// to `done`, on #171's argument that nobody was watching the board for it.
///
/// The operator decision of 2026-08-05 removed every automatic route to
/// Done, so it now stops in `in_review` like any other card. The thing #171
/// actually cared about — that the originating conversation gets its answer
/// rather than waiting on a board nobody is reading — is unaffected and is
/// asserted here: the post-back still fires, and it now says the card is
/// ready for review instead of claiming it is finished.
#[tokio::test]
async fn dispatched_card_with_an_origin_stops_in_review_and_still_posts_back() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    // A roster assignee: since #205 an off-roster one never runs a turn, so
    // it would settle to `todo` and prove nothing about the terminal.
    let mut c = card("t-origin", "engineer");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");
    // `run_task` is driven directly here rather than through `run_cycle`,
    // so the roster the turn runs on has to be built explicitly. Without it
    // every dispatch fails with "company not found" and settles to
    // `todo` — which still satisfies this test's post-back assertions
    // while proving nothing about the terminal column.
    brain
        .pool
        .ensure(&brain.record(), &brain.deps)
        .await
        .expect("roster");

    let posted = brain
        .run_task("t-origin", None)
        .await
        .expect("run")
        .expect("a card with an origin posts back");

    let moved = only_card(&tasks).await;
    assert_eq!(
        moved.column, COLUMN_IN_REVIEW,
        "Done is a person's decision; no dispatch may reach it on its own"
    );
    // The note stays the durable record of what came back.
    assert!(moved.note.expect("note").contains("Ship the thing"));
    // …and the bubble answers in the originating thread either way, which
    // is what the handoff was actually waiting on.
    assert!(posted.text.contains("ready for review"), "{}", posted.text);
    assert!(!posted.text.contains("is done"), "{}", posted.text);
}

/// **The headline of #244, stated as its own test.** This used to assert
/// the opposite — that a completed dispatch always mints an artifact from
/// its chat reply.
///
/// It does not any more. A run that published nothing yields **no
/// artifact**, and that is a first-class outcome rather than a gap. The old
/// behaviour is exactly what made the Artifacts tab present refusals and
/// blocker messages as deliverables: capture was gated on run disposition
/// and never on whether anything had been produced.
///
/// Nothing is lost. The reply still reaches the card note, the timeline, the
/// completion event and the run trace — five records, none of which claims
/// to be a deliverable.
#[tokio::test]
async fn a_completed_run_that_published_nothing_records_no_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    // Empty assignee → the default responder, so the turn actually runs.
    let mut c = card("t-origin", "");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    ops.upsert(&CompanyId::new("acme"), &c).await.expect("seed");

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t-origin".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let moved = only_card(&ops).await;
    assert_eq!(
        moved.column, COLUMN_IN_REVIEW,
        "since #337 every finished card stops for a person"
    );
    let artifacts = crate::ports::artifacts::ArtifactStore::list(
        &*ops,
        &CompanyId::new("acme"),
        Some("t-origin"),
    )
    .await
    .expect("list");
    assert!(
        artifacts.is_empty(),
        "a run that published nothing has no deliverable: {artifacts:?}"
    );
    // …and the reply is still recorded where it belongs: on the card.
    assert!(
        moved.note.expect("note").contains("Ship the thing"),
        "the reply must survive even though it is not an artifact"
    );
}

/// **The identity-vs-recency regression** — the second defect #244 names,
/// and the one with teeth.
///
/// The old extend target was `max_by_key(updated_at_millis)`: whichever
/// artifact on the card had been touched most recently. An **operator edit**
/// bumps that timestamp. So an operator who tidied the invoice made the
/// invoice the target for the agent's next write to the spec — the spec's v2
/// landed as the invoice's v3, and `human_edit_diff` then reported a human
/// rewriting a document they had never opened.
///
/// The setup here reproduces exactly that: publish two files, operator-edit
/// the **second** so it is unambiguously the most recent, then republish the
/// **first**. Under recency this appends to the invoice. Under identity it
/// extends the spec, and the invoice is untouched.
#[tokio::test]
async fn a_republish_extends_by_identity_not_by_whatever_was_edited_last() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::{ArtifactAuthor, ArtifactStore};

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");
    let c = card("t-1", "maya");
    let publish = |source: &str, body: &str| PendingPublish {
        agent: "maya".to_string(),
        source: source.to_string(),
        title: source.to_string(),
        kind: crate::ports::artifacts::ArtifactKind::Markdown,
        note: None,
        payload: crate::harness::publish::PublishPayload::Text(body.to_string()),
    };

    // Run 1 publishes both files.
    let ids = brain
        .record_published_artifacts(
            &c,
            "maya",
            vec![
                publish("specs/launch.md", "# Spec v1"),
                publish("billing/invoice.md", "# Invoice v1"),
            ],
            Some("run-1"),
        )
        .await
        .expect("records");
    assert_eq!(ids.len(), 2, "two files, two records");

    let by_source = |list: &[crate::ports::artifacts::ArtifactRecord], source: &str| {
        list.iter()
            .find(|a| a.source.as_deref() == Some(source))
            .expect("record for source")
            .clone()
    };
    let listed = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    let mut invoice = by_source(&listed, "billing/invoice.md");

    // The operator edits the INVOICE, making it the most recently updated
    // artifact on the card. This is the trap.
    invoice.push_version(
        "# Invoice v1, corrected",
        ArtifactAuthor::Operator,
        "operator",
        now_millis() + 1_000,
        Some("operator edit before approval".to_string()),
    );
    ArtifactStore::upsert(&*ops, &company, &invoice)
        .await
        .unwrap();

    // Run 2 republishes the SPEC only.
    brain
        .record_published_artifacts(
            &c,
            "maya",
            vec![publish("specs/launch.md", "# Spec v2")],
            Some("run-2"),
        )
        .await
        .expect("records");

    let after = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .unwrap();
    assert_eq!(after.len(), 2, "no duplicate record was opened");

    let spec = by_source(&after, "specs/launch.md");
    assert_eq!(
        spec.versions.len(),
        2,
        "the republished path must extend its OWN record"
    );
    assert_eq!(spec.latest().unwrap().body, "# Spec v2");
    assert_eq!(spec.latest().unwrap().run_id.as_deref(), Some("run-2"));
    assert_eq!(
        spec.versions[0].run_id.as_deref(),
        Some("run-1"),
        "an earlier attempt keeps the attempt that wrote it"
    );

    let invoice = by_source(&after, "billing/invoice.md");
    assert_eq!(
        invoice.versions.len(),
        2,
        "the agent's spec must not have landed on the invoice"
    );
    assert_eq!(invoice.latest().unwrap().body, "# Invoice v1, corrected");
    assert_eq!(
        invoice.latest().unwrap().author,
        ArtifactAuthor::Operator,
        "the invoice's newest version is still the human's"
    );
    // And the reason all of this matters: the human-edit diff still says
    // what a human actually did, on each document separately.
    let diff = invoice.human_edit_diff().expect("the operator edited it");
    assert_eq!((diff.from_version, diff.to_version), (1, 2));
    assert!(
        spec.human_edit_diff().is_none(),
        "nobody edited the spec, so it must report no human edit"
    );
}

/// Two publishes of the same path within one run extend one record rather
/// than opening two — the working set the loop keeps has to stay current.
#[tokio::test]
async fn republishing_the_same_path_twice_in_one_run_extends_once() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let c = card("t-1", "maya");
    let publish = |body: &str| PendingPublish {
        agent: "maya".to_string(),
        source: "spec.md".to_string(),
        title: "spec.md".to_string(),
        kind: crate::ports::artifacts::ArtifactKind::Markdown,
        note: None,
        payload: crate::harness::publish::PublishPayload::Text(body.to_string()),
    };

    brain
        .record_published_artifacts(&c, "maya", vec![publish("draft"), publish("final")], None)
        .await
        .expect("records");

    let listed = ArtifactStore::list(&*ops, &CompanyId::new("acme"), Some("t-1"))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].versions.len(), 2);
    assert_eq!(listed[0].latest().unwrap().body, "final");
}

/// A store fault on an **explicit** publish propagates. The old path
/// returned a silent `Ok(())`, which made a lost deliverable
/// indistinguishable from a successful one.
#[tokio::test]
async fn a_store_error_on_a_published_file_propagates() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::{ArtifactRecord, ArtifactStore};

    /// Reads fine, refuses every write.
    struct BrokenArtifacts;
    #[async_trait]
    impl ArtifactStore for BrokenArtifacts {
        async fn list(
            &self,
            _: &CompanyId,
            _: Option<&str>,
        ) -> crate::Result<Vec<ArtifactRecord>> {
            Ok(Vec::new())
        }
        async fn get(&self, _: &CompanyId, _: &str) -> crate::Result<Option<ArtifactRecord>> {
            Ok(None)
        }
        async fn upsert(&self, _: &CompanyId, _: &ArtifactRecord) -> crate::Result<()> {
            Err(crate::error::OpenCompanyError::Store(
                "the disk is full".to_string(),
            ))
        }
        async fn delete(&self, _: &CompanyId, _: &str) -> crate::Result<bool> {
            Ok(false)
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (mut brain, _ops) = brain_with_artifacts(dir.path());
    // Sole owner at this point — no turn has run, so nothing has cloned the
    // deps into a lane yet.
    Arc::get_mut(&mut brain.deps)
        .expect("the brain is the only holder of its deps before any turn")
        .artifacts = Some(Arc::new(BrokenArtifacts));

    let err = brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: "maya".to_string(),
                source: "spec.md".to_string(),
                title: "spec.md".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("# Spec".to_string()),
            }],
            None,
        )
        .await
        .expect_err("a lost deliverable must not read as a success");
    assert!(err.to_string().contains("the disk is full"), "{err}");
}

/// Issue #463, the headline. A publish made when this message already has a
/// card files **onto that card** rather than minting a second one beside it.
///
/// #445 minted unconditionally, which was right on its own and wrong beside
/// #442's card-by-construction: one substantial ask that ended in a
/// published file left two cards, and the reply bubble linked to the empty
/// one because that is the card the turn opened.
#[tokio::test]
async fn a_publish_with_a_card_in_scope_files_onto_it_instead_of_minting() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");
    // The card #442 opens for the work — no owner yet, exactly like the
    // To-do card the REST chat handler writes.
    let mut open = card("t-open", "");
    open.column = COLUMN_TODO.to_string();
    TaskStore::upsert(&*ops, &company, &open)
        .await
        .expect("seed");

    let filed = brain
        .file_publishes_on_card(
            "t-open",
            "writer",
            ChatTarget::default(),
            vec![PendingPublish {
                agent: "writer".to_string(),
                source: "memo.md".to_string(),
                title: "Q3 board memo".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("# Memo".to_string()),
            }],
        )
        .await
        .expect("files onto the card");

    assert_eq!(filed, "t-open", "no second card was minted");
    let cards = TaskStore::list(&*ops, &company).await.expect("list");
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(
        cards[0].column, COLUMN_IN_REVIEW,
        "a delivered file lands for a person to accept"
    );
    assert_eq!(
        cards[0].assignee, "writer",
        "an unowned card becomes the publisher's"
    );
    assert!(
        cards[0]
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("memo.md"),
        "the note names what landed: {:?}",
        cards[0].note
    );
    let artifacts = ArtifactStore::list(&*ops, &company, Some("t-open"))
        .await
        .expect("list artifacts");
    assert_eq!(artifacts.len(), 1, "the deliverable is ON the card");
    assert_eq!(artifacts[0].title, "Q3 board memo");
}

/// …but filing a file never takes somebody's card away from them. Only an
/// **unowned** card is claimed by the publisher.
#[tokio::test]
async fn filing_a_publish_leaves_an_owned_card_with_its_owner() {
    use crate::harness::publish::PendingPublish;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");
    ops.upsert(&company, &card("t-owned", "maya"))
        .await
        .expect("seed");

    brain
        .file_publishes_on_card(
            "t-owned",
            "writer",
            ChatTarget::default(),
            vec![PendingPublish {
                agent: "writer".to_string(),
                source: "memo.md".to_string(),
                title: "Memo".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("# Memo".to_string()),
            }],
        )
        .await
        .expect("files onto the card");

    assert_eq!(only_card(&ops).await.assignee, "maya");
}

/// A card that vanished between the turn and the drain falls back to
/// minting. The rule exists to stop a second card, not to lose a
/// deliverable — dropping the artifact would be #445 all over again.
#[tokio::test]
async fn a_publish_onto_a_card_that_vanished_mints_one_rather_than_dropping_it() {
    use crate::harness::publish::PendingPublish;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");

    let filed = brain
        .file_publishes_on_card(
            "t-gone",
            "writer",
            ChatTarget::channel(Some("strategy")),
            vec![PendingPublish {
                agent: "writer".to_string(),
                source: "memo.md".to_string(),
                title: "Memo".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("# Memo".to_string()),
            }],
        )
        .await
        .expect("mints a card instead");

    let cards = TaskStore::list(&*ops, &company).await.expect("list");
    assert_eq!(cards.len(), 1, "the deliverable still has a card");
    assert_eq!(cards[0].assignee, "writer");
    // The returned id must be the REPLACEMENT, not the id that is gone: the
    // caller links the operator's reply to it (#463 review).
    assert_eq!(
        filed, cards[0].id,
        "the returned id must name the card the deliverable landed on"
    );
    assert_ne!(filed, "t-gone");
    // …and it belongs to the same conversation, like the card the
    // no-card-in-scope path mints. Two minting paths must not disagree
    // about where their card posts back.
    assert_eq!(cards[0].origin_chat_id(), Some("strategy"));
}

/// Each artifact records the agent that published **it** (#463 review).
///
/// One drain can hold publishes from more than one agent — the desk lead's
/// turn and the orchestrator's own turn both run with the full toolbelt
/// under a single `Conversation` claim. Collapsing the batch to one author
/// stamps the writer's name on the orchestrator's file and the reverse.
#[tokio::test]
async fn each_published_artifact_records_its_own_author() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());
    let company = CompanyId::new("acme");
    let publish = |agent: &str, source: &str| PendingPublish {
        agent: agent.to_string(),
        source: source.to_string(),
        title: source.to_string(),
        kind: crate::ports::artifacts::ArtifactKind::Markdown,
        note: None,
        payload: crate::harness::publish::PublishPayload::Text(format!("# {source}")),
    };

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            // The batch-level fallback, which must NOT win over the
            // per-item agents below.
            "maya",
            vec![publish("writer", "memo.md"), publish("ceo", "notes.md")],
            None,
        )
        .await
        .expect("records");

    let mut authors: Vec<(String, String)> = ArtifactStore::list(&*ops, &company, Some("t-1"))
        .await
        .expect("list")
        .into_iter()
        .map(|a| {
            (
                a.source.clone().unwrap_or_default(),
                a.versions[0].author_id.clone(),
            )
        })
        .collect();
    authors.sort();
    assert_eq!(
        authors,
        vec![
            ("memo.md".to_string(), "writer".to_string()),
            ("notes.md".to_string(), "ceo".to_string()),
        ]
    );
}

/// …and a `PendingPublish` built by hand — not by the tool, which always
/// stamps its agent — still falls back to the caller's responder rather
/// than recording a blank author.
#[tokio::test]
async fn a_publish_with_no_agent_falls_back_to_the_responder() {
    use crate::harness::publish::PendingPublish;
    use crate::ports::artifacts::ArtifactStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, ops) = brain_with_artifacts(dir.path());

    brain
        .record_published_artifacts(
            &card("t-1", "maya"),
            "maya",
            vec![PendingPublish {
                agent: String::new(),
                source: "memo.md".to_string(),
                title: "Memo".to_string(),
                kind: crate::ports::artifacts::ArtifactKind::Markdown,
                note: None,
                payload: crate::harness::publish::PublishPayload::Text("# Memo".to_string()),
            }],
            None,
        )
        .await
        .expect("records");

    let listed = ArtifactStore::list(&*ops, &CompanyId::new("acme"), Some("t-1"))
        .await
        .expect("list");
    assert_eq!(listed[0].versions[0].author_id, "maya");
}

/// The other half: a run that did NOT succeed records nothing either, and
/// its note still says what happened.
#[tokio::test]
async fn a_cancelled_delegated_card_records_no_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, ops, _provider) =
        brain_that_steers_itself(dir.path(), "t-cancel", vec![SteerAction::Cancel]);
    let mut c = card("t-cancel", "");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    ops.upsert(&CompanyId::new("acme"), &c).await.expect("seed");

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t-cancel".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(only_card(&ops).await.column, COLUMN_TODO);
    let artifacts = crate::ports::artifacts::ArtifactStore::list(
        &*ops,
        &CompanyId::new("acme"),
        Some("t-cancel"),
    )
    .await
    .expect("list");
    assert!(
        artifacts.is_empty(),
        "a cancelled run has no deliverable to version"
    );
}

/// **Rewritten by #337.** The landing no longer depends on the card at
/// all: a card's origin used to pick between two success terminals, and now
/// there is one. The decision itself lives in
/// [`crate::ports::tasks::column_for_settled_run`] (and is unit-tested
/// there); this pins that `settle` — every run-ending path in this file —
/// actually consults it, for both card shapes.
#[test]
fn settle_lands_every_finished_card_in_review_whatever_its_origin() {
    let mut board_card = card("t1", "maya");
    settle(&mut board_card, TaskRunEnd::Completed, "maya", "shipped");
    assert_eq!(board_card.column, COLUMN_IN_REVIEW);

    let mut delegated = card("t2", "maya");
    delegated.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    settle(&mut delegated, TaskRunEnd::Completed, "maya", "shipped");
    assert_eq!(
        delegated.column, COLUMN_IN_REVIEW,
        "an origin no longer buys a card its own terminal"
    );
}

/// The redirect-cap finalize branch is the other success ending, so it has
/// to make the same choice — otherwise a steered handoff diverges from an
/// unsteered one.
#[tokio::test]
async fn redirect_cap_finalizes_a_card_with_an_origin_to_review() {
    let dir = tempfile::tempdir().unwrap();
    let redirect = || SteerAction::Redirect {
        instruction: "focus on the API".to_string(),
    };
    let (brain, tasks, _provider) = brain_that_steers_itself(
        dir.path(),
        "t1",
        vec![redirect(), redirect(), redirect(), redirect()],
    );
    let mut c = card("t1", "");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(only_card(&tasks).await.column, COLUMN_IN_REVIEW);
}

/// The relay has to have wording for the `done` landing column — without it
/// the fallback arm renders the raw column id into the sentence.
#[test]
fn postback_reads_naturally_for_a_done_card() {
    let mut finished = card("t1", "maya");
    finished.column = "done".to_string();
    finished.note = None;
    assert_eq!(
        lifecycle::relay_text(&finished, "maya", "ceo", &[]),
        "\"Ship the thing\" is done (maya ran it)."
    );
}

/// An `assignee` that names a roster member routes the turn to that member.
#[tokio::test]
async fn task_dispatch_routes_to_assignee() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", "engineer"))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let note = only_card(&tasks).await.note.expect("note");
    assert!(note.contains("[engineer]"), "{note:?}");
}

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

/// The settle. This fixture's pool holds no roster, so the turn errors —
/// which is exactly the `TaskRunEnd::Failed` path — and the row must end
/// **terminal**, carrying the reason the card's note carries, rather than
/// sitting `Pending` for the boot reaper to find.
#[tokio::test]
async fn a_dispatch_settles_its_attempt_row_from_how_the_run_ended() {
    use crate::ports::runs::RunStatus;

    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks, runs) = brain_with_a_pending_run(dir.path(), "engineer").await;
    let company = CompanyId::new("acme");

    brain.run_task("t-1", Some("run-1")).await.expect("run");

    let settled = runs
        .get_run(&company, "run-1")
        .await
        .expect("read")
        .expect("the row survives");
    assert_eq!(settled.status, RunStatus::Failed);
    assert!(
        settled.finished_at_millis.is_some(),
        "a terminal settle stamps when the attempt ended"
    );
    let reason = settled.error.expect("a failure carries its reason");
    assert!(reason.contains("dispatch failed"), "{reason}");
    // …and the row agrees with the card, rather than telling a second story.
    let note = only_card(&tasks).await.note.expect("note");
    assert!(note.contains("dispatch failed"), "{note}");
    // No turn ran on this offline fixture, so there is nothing to charge.
    assert_eq!(settled.step_count, 0);
    assert_eq!(settled.usage, TokenUsage::default());
}

/// Issue #1865 (CodeRabbit review, PR #1883 review comment 3892338104): an
/// ordinary assigned board card — no `origin_chat_id`, so no relay target
/// — whose turn genuinely fails (not a refusal) reaches this same
/// rich-settle tail with a bounce chip but, before this fix, filed no
/// `dispatch_failed` notification. `refuse_dispatch` files this
/// notification for an off-roster assignee, and the cycle's terminality
/// backstop files it for a crash-recovered dispatch — but the backstop
/// explicitly skips any run no longer active, and `settle_run` just above
/// this test's call site already terminalizes the attempt, so the
/// backstop never sees it either. That left an ordinary failed dispatch
/// with no origin chat completely silent: no chat reply, no badge,
/// nothing but the board itself.
#[tokio::test]
async fn an_ordinary_failed_dispatch_with_no_origin_chat_files_a_dispatch_failed_notification()
{
    use crate::ports::runs::NewRun;

    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks_notified(dir.path(), true);
    let runs: Arc<dyn crate::ports::RunStore> = Arc::new(FsOps::new(dir.path()));
    let company = CompanyId::new("acme");
    tasks
        .upsert(&company, &card("t-1", "engineer"))
        .await
        .expect("seed");
    runs.create_run(&company, NewRun::for_task("run-1", "t-1", "engineer"))
        .await
        .expect("mint");
    let brain = brain.with_runs(Arc::clone(&runs));

    brain.run_task("t-1", Some("run-1")).await.expect("run");

    let settled = only_card(&tasks).await;
    assert_eq!(settled.column, COLUMN_TODO);
    assert!(
        settled.bounced.is_some(),
        "an ordinary turn failure must carry the bounce chip: {settled:?}"
    );
    assert!(
        settled.origin_chat_id().is_none(),
        "this is exactly the board-created shape with no relay target: {settled:?}"
    );

    let notes = crate::ports::notifications::NotificationStore::list(
        tasks.as_ref(),
        &company,
        "anyone",
    )
    .await
    .expect("list notifications");
    assert!(
        notes
            .iter()
            .any(|n| n.notification.kind == "dispatch_failed"
                && n.notification.subject.id == "t-1"),
        "an ordinary failed dispatch with no origin chat must still file a \
         dispatch_failed notification, got {notes:?}"
    );
}

/// A refusal is an attempt too. It spends nothing and runs no turn, but it
/// is a real, terminal outcome — the card's history must show "this was
/// tried and refused, and why" rather than a gap where an attempt was.
#[tokio::test]
async fn a_refused_dispatch_settles_its_attempt_rather_than_leaving_a_gap() {
    use crate::ports::runs::RunStatus;

    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks, runs) = brain_with_a_pending_run(dir.path(), "Shane").await;
    let company = CompanyId::new("acme");

    brain.run_task("t-1", Some("run-1")).await.expect("run");

    let settled = runs
        .get_run(&company, "run-1")
        .await
        .expect("read")
        .expect("row");
    assert_eq!(settled.status, RunStatus::Failed);
    let reason = settled.error.expect("reason");
    assert!(reason.contains("dispatch refused"), "{reason}");
    assert!(
        reason.contains("Shane"),
        "the row must name what was wrong, like the note does: {reason}"
    );
}

/// The degraded path stays degraded, not broken: a dispatch carrying no run
/// id runs the card exactly as before and invents no row for it.
#[tokio::test]
async fn an_untracked_dispatch_runs_the_card_and_records_no_attempt() {
    use crate::ports::runs::RunFilter;

    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let runs: Arc<dyn crate::ports::RunStore> = Arc::new(FsOps::new(dir.path()));
    let brain = brain.with_runs(Arc::clone(&runs));
    let company = CompanyId::new("acme");
    tasks
        .upsert(&company, &card("t-1", "engineer"))
        .await
        .expect("seed");

    brain.run_task("t-1", None).await.expect("run");

    assert!(
        only_card(&tasks).await.note.is_some(),
        "the card still ran and still recorded its outcome"
    );
    assert!(
        runs.list_runs(&company, &RunFilter::default())
            .await
            .expect("list")
            .is_empty(),
        "no row was minted for this dispatch, so none may be invented"
    );
}

/// A card that vanished between the dispatch write and the cycle still
/// closes its attempt. Otherwise the row would sit `Pending` until a
/// restart reaped it with a misleading "the host restarted" reason.
#[tokio::test]
async fn a_dispatch_whose_card_is_gone_still_closes_its_attempt() {
    use crate::ports::runs::{NewRun, RunStatus};

    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_tasks(dir.path());
    let runs: Arc<dyn crate::ports::RunStore> = Arc::new(FsOps::new(dir.path()));
    let brain = brain.with_runs(Arc::clone(&runs));
    let company = CompanyId::new("acme");
    runs.create_run(&company, NewRun::for_task("run-1", "t-gone", "engineer"))
        .await
        .expect("mint");

    assert!(
        brain
            .run_task("t-gone", Some("run-1"))
            .await
            .expect("run")
            .is_none(),
        "a missing card posts nothing back"
    );

    let settled = runs
        .get_run(&company, "run-1")
        .await
        .expect("read")
        .expect("row");
    assert_eq!(settled.status, RunStatus::Failed);
    assert_eq!(settled.error.as_deref(), Some(CARD_VANISHED));
}

// ── Issue #205: the working agent is linked, and a bad assignee is refused ──

/// The reported bug. A card assigned to "Shane" — nobody this company has —
/// used to dispatch to the orchestrator anyway, keeping `assignee = "Shane"`
/// while the timeline read "reply from ceo" and nothing said the name was
/// invalid. It must now be **refused**: the card goes back to `todo`
/// carrying the reason, and the orchestrator runs no turn on its behalf.
#[tokio::test]
async fn task_dispatch_off_roster_assignee_is_refused_not_silently_reassigned() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", "Shane"))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let refused = only_card(&tasks).await;
    assert_eq!(
        refused.column, COLUMN_TODO,
        "a card nobody can work must not sit in in_progress"
    );
    assert_eq!(
        refused.assignee, "Shane",
        "the invalid name is left as typed for the operator to correct"
    );
    // Issue #1865 (CodeRabbit review, PR #1883): a refusal is a failed
    // dispatch landing on `todo` exactly like any other, so it must carry
    // the same bounce chip `run_task`'s rich settle and the system mover
    // apply — the board must not read this card any differently just
    // because nobody ever ran.
    assert!(
        refused.bounced.is_some(),
        "an off-roster refusal must set the bounce chip like any other failed dispatch: {refused:?}"
    );
    let note = refused.note.expect("the refusal is written to the note");
    assert!(
        note.contains("Shane"),
        "the operator must be told which name is not a teammate: {note:?}"
    );
    assert!(
        note.contains("dispatch refused"),
        "the note must read as a refusal, not as work the CEO did: {note:?}"
    );
    assert!(
        !note.contains("mock: "),
        "no turn may run for an assignee nobody answers to: {note:?}"
    );
}

/// Issue #1865 (CodeRabbit review, PR #1883 review comment 3878668326): a
/// board-created card (no `origin_chat_id`, exactly [`card`]'s shape) with
/// an off-roster assignee bounces to `todo` and gets the bounce chip
/// (c6c3a3083), but before this fix filed no `dispatch_failed`
/// notification — the relay `refuse_dispatch` falls back to only fires
/// when an `origin_chat_id` exists, and `settle_run_end` makes the
/// attempt terminal before the cycle's own backstop notifier ever sees
/// it. That left the refusal visible only to someone already looking at
/// the board, unlike every other bounced-dispatch path
/// (`CompanyRuntime::abandon_run`, the cycle's terminality backstop, the
/// boot reaper's card sweep, and `workflow_build`'s `settle_to_todo`),
/// which all raise this same notification.
#[tokio::test]
async fn a_refused_dispatch_with_no_origin_chat_files_a_dispatch_failed_notification() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks_notified(dir.path(), true);
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", "Shane"))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let refused = only_card(&tasks).await;
    assert_eq!(refused.column, COLUMN_TODO);
    assert!(
        refused.origin_chat_id().is_none(),
        "this is exactly the board-created shape with no relay target: {refused:?}"
    );

    let notes = crate::ports::notifications::NotificationStore::list(
        tasks.as_ref(),
        &CompanyId::new("acme"),
        "anyone",
    )
    .await
    .expect("list notifications");
    assert!(
        notes
            .iter()
            .any(|n| n.notification.kind == "dispatch_failed"
                && n.notification.subject.id == "t1"),
        "a board card refused with no origin chat must still file a \
         dispatch_failed notification, got {notes:?}"
    );
}

/// The other half of #205: a card the orchestrator picks up because nobody
/// was named gets that orchestrator written onto it, so the board names who
/// is actually doing the work instead of showing a blank assignee.
#[tokio::test]
async fn task_dispatch_links_the_working_agent_to_an_unassigned_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", ""))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(
        only_card(&tasks).await.assignee,
        "ceo",
        "the orchestrator that worked the card must be linked to it"
    );
}

/// A card assigned to a **desk** is worked by that desk's lead, but the card
/// stays the desk's. `delegate_to_desk` writes a desk id into `assignee`, so
/// this is the shape a hand-off actually produces. Dispatch picks who runs
/// this turn, not who owns the card, so only the note names the member that
/// ran it — relinking the lead onto `assignee` would erase the desk from the
/// board the first time the card ran (#214 review).
#[tokio::test]
async fn task_dispatch_routes_a_desk_assignee_to_its_lead_but_keeps_the_desk() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_desk_tasks(dir.path());
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", "eng"))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let worked = only_card(&tasks).await;
    assert_eq!(
        worked.assignee, "eng",
        "a desk assignment is ownership: the card stays the desk's"
    );
    let note = worked.note.expect("note");
    assert!(
        note.contains("[engineer]"),
        "the desk's lead member still did the work, and the note names them: {note:?}"
    );
}

/// An **operator-overlay** teammate is a roster teammate. The narrow
/// `manifest.agents`-only lookup used to drop these onto the orchestrator.
#[tokio::test]
async fn task_dispatch_routes_to_an_overlay_teammate() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    brain.mutate_record(|r| {
        r.overlay_agents.push(OverlayAgent {
            provider: None,
            id: "nova".into(),
            name: "Nova".into(),
            role: "Growth".into(),
            description: None,
            tools: None,
            model: None,
            harness: None,
        })
    });
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", "nova"))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let note = only_card(&tasks).await.note.expect("note");
    assert!(
        note.contains("[nova]"),
        "an overlay teammate must work their own card: {note:?}"
    );
}

/// A refused dispatch still answers in the thread the card came from —
/// otherwise a delegated hand-off to a bad assignee is silent twice over.
#[tokio::test]
async fn a_refused_dispatch_posts_the_reason_back_to_its_origin() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-origin", "Shane");
    c.origin = TaskOrigin::new(Some("strategy".to_string()), None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    let posted = brain
        .run_task("t-origin", None)
        .await
        .expect("run")
        .expect("a refused card with an origin must still post back");
    assert_eq!(
        posted.reply_to.as_ref().map(|r| r.chat_id.as_str()),
        Some("strategy")
    );
    assert_eq!(
        posted.channel,
        brain.orchestrator(),
        "the orchestrator answers for its own roster"
    );
    assert!(posted.text.contains("Shane"), "{}", posted.text);
    // Issue #1852: `refuse_dispatch` relays through the same
    // `relay_reply` as a settled run, so it carries the card id too — but
    // `CompanyRuntime::journal_dispatch_replies` strips it back to `None`
    // before journaling, since the settle already left a
    // `DeskTaskCompleted` link and this would only duplicate it.
    assert_eq!(posted.task_id.as_deref(), Some("t-origin"));
}

/// A refusal into a private DM must not claim anyone ran the card.
///
/// `refuse_dispatch` used to pass the orchestrator's own id into
/// `relay_reply`'s `responder` slot while the DM's speaker went into the
/// `orchestrator` slot — the opposite of every other call site. For a
/// desk/shared origin the two values collide (`relay_speaker` returns the
/// orchestrator) and the swap is invisible, but a private DM's speaker is
/// the teammate, not the orchestrator, so the mismatch fires the "ran it"
/// credit onto a card that never ran at all (CodeRabbit review, PR #1949
/// thread 3895107568).
#[tokio::test]
async fn a_refused_dispatch_into_a_dm_credits_no_one() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-dm-origin", "Shane");
    // "engineer" is a real roster agent (not the orchestrator "ceo"), so
    // `relay_speaker` claims this as a private DM and returns "engineer"
    // instead of falling back to the orchestrator.
    c.origin = TaskOrigin::new(Some("engineer".to_string()), None);
    tasks
        .upsert(&CompanyId::new("acme"), &c)
        .await
        .expect("seed");

    let posted = brain
        .run_task("t-dm-origin", None)
        .await
        .expect("run")
        .expect("a refused card with an origin must still post back");
    assert!(
        !posted.text.contains("ran it"),
        "a refusal must never credit anyone with running the card: {}",
        posted.text
    );
}

/// A dispatch for a card that no longer exists is a silent no-op, not an
/// error.
#[tokio::test]
async fn task_dispatch_missing_card_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "nope".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs without a card");
    assert!(
        tasks
            .list(&CompanyId::new("acme"))
            .await
            .unwrap()
            .is_empty()
    );
}

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

/// A dispatch relay into a teammate's private DM is authored by that
/// teammate; every shared surface keeps the orchestrator's voice.
#[test]
fn relay_speaker_claims_a_private_dm_for_its_own_agent() {
    let record = record_with_desk();
    // A teammate DM: the origin is that teammate, so they speak.
    assert_eq!(relay_speaker(&record, "engineer", "chief"), "engineer");
    // The console's `dm:<id>` channel key resolves the same teammate.
    assert_eq!(relay_speaker(&record, "dm:engineer", "chief"), "engineer");
    // A desk is a shared surface — the orchestrator stays the one voice.
    assert_eq!(relay_speaker(&record, "eng_desk", "chief"), "chief");
    // The orchestrator's own DM is answered by the orchestrator, not doubled.
    assert_eq!(relay_speaker(&record, "chief", "chief"), "chief");
    // General / empty / unknown origins all keep the orchestrator.
    assert_eq!(relay_speaker(&record, "General", "chief"), "chief");
    assert_eq!(relay_speaker(&record, "", "chief"), "chief");
    assert_eq!(relay_speaker(&record, "nobody-here", "chief"), "chief");
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

/// PR #1949 review (Codex thread 3895066480): a `dm:` key names a
/// teammate even when a desk shares that id — the same invariant issue
/// #1743 established for `chat_responder`
/// (`runtime::delegation_tools::a_prefixed_dm_reaches_the_teammate_even_
/// when_a_desk_shares_the_id`). `relay_speaker` used to re-run the bare,
/// desk-first `assignee::resolve` on the key once the prefix was
/// stripped, so a card dispatched from that teammate's private DM
/// resolved to `Desk` and fell through to the orchestrator — reopening
/// #1743's bug in the relay's own resolver, and misattributing a private
/// DM's card as though it were answered on the shared desk.
#[test]
fn relay_speaker_reaches_the_dm_teammate_even_when_a_desk_shares_the_id() {
    let record = record_with_colliding_desk_and_teammate_id();
    assert_eq!(
        relay_speaker(&record, "dm:engineer", "chief"),
        "engineer",
        "the prefix names the teammate, not the desk that shares its id"
    );
    // The bare key still belongs to the desk, exactly as it does for
    // `chat_responder` — only the prefixed address reaches the teammate.
    assert_eq!(
        relay_speaker(&record, "engineer", "chief"),
        "chief",
        "the desk still answers its own bare id"
    );
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

/// Issue #707, the retention half: a store with **no persisted record**
/// leaves the brain's record exactly as it was.
///
/// `Ok(None)` is not a failure and must not be treated as one — an absent
/// record is what a company whose bundle has not been written yet looks
/// like, and clearing on it would leave that company with no roster and no
/// desks at all. Uses the real `FsCompanyStore` over a directory nothing was
/// saved to, which is precisely the shape that returns `Ok(None)`.
#[tokio::test]
async fn a_refresh_with_no_persisted_record_keeps_the_one_it_has() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    let before = brain.record();

    brain
        .refresh_record()
        .await
        .expect("an absent record is not an error");

    let after = brain.record();
    assert_eq!(
        after.manifest.company.name, before.manifest.company.name,
        "the brain kept its record"
    );
    assert_eq!(
        after.manifest.agents.len(),
        before.manifest.agents.len(),
        "an absent record must not empty the roster"
    );
    assert_eq!(
        delegation::desk_lead(&brain.record(), "eng_desk"),
        Some("engineer".to_string()),
        "nor cost the company its desks"
    );
}

/// Issue #707, the loud half: a store that **cannot be read** fails the
/// refresh rather than falling back to the record already in hand.
///
/// Falling back is the defect this whole change removes, and it would come
/// back invisibly — a turn that looked successful while routing on state the
/// operator had already replaced. So the error propagates and the cycle
/// fails. A corrupt `company.toml` is a real way to reach that arm, and
/// reaching it through the real store rather than a double is what keeps
/// this test honest about the failure it claims to cover.
#[tokio::test]
async fn a_refresh_that_cannot_read_the_store_fails_rather_than_going_stale() {
    use crate::ports::store::CompanyStore;

    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    let store = FsCompanyStore::new(dir.path());
    store.save(&brain.record()).await.expect("seed the record");

    // A bundle whose manifest no longer parses: the store reports a failure
    // rather than an absence, which is the arm under test.
    let toml_path =
        crate::store::Bundle::new(dir.path().to_path_buf(), &brain.record().id).company_toml();
    tokio::fs::write(&toml_path, b"this is not = valid toml [[[")
        .await
        .expect("corrupt the manifest");

    let err = brain
        .refresh_record()
        .await
        .expect_err("an unreadable record must fail the refresh, not be ignored");
    assert!(
        err.to_string().contains("company.toml"),
        "the failure names what could not be read: {err}"
    );

    // And through `run_cycle`, which is the level that actually protects
    // the promise. Asserting only on `refresh_record` would leave the call
    // site free to become `let _ = self.refresh_record().await;` — the
    // refresh would still run, the error would be dropped, the turn would
    // report success while routing on the record it already held, and every
    // other test here would stay green. That is issue #707 returning by a
    // different door, so the propagation is pinned where it is relied on.
    let cycle = brain.run_cycle(request(Vec::new()), &NoopHost).await;
    assert!(
        cycle.is_err(),
        "a cycle must fail when the record cannot be read, rather than \
         quietly routing on a stale one"
    );
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

/// The whole point of the feature: `@engineer` in the main line is answered
/// by the engineer, not by the orchestrator that would otherwise take it.
#[test]
fn a_mentioned_teammate_answers_instead_of_the_default_responder() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    // Without a mention, the orchestrator answers an unaddressed message.
    assert_eq!(brain.responder_for(None), "chief");
    // With one, the named teammate does.
    assert_eq!(
        crate::runtime::mentions::mention_responder(
            &brain.record(),
            None,
            &[mention_of("engineer")]
        ),
        Some("engineer".to_string()),
    );
}

/// And it outranks the *desk lead*, which is the stronger claim: a message
/// addressed to a desk still goes to the teammate it names.
#[test]
fn a_mention_outranks_the_addressed_desks_lead() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
    assert_eq!(
        crate::runtime::mentions::mention_responder(
            &brain.record(),
            None,
            &[mention_of("ceo")]
        ),
        Some("ceo".to_string()),
        "the named teammate answers even on a desk with its own lead",
    );
}

/// A message that mentions nobody routes exactly as it did before mentions
/// existed — which is every message already in every journal.
#[test]
fn a_message_with_no_mentions_routes_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(
        crate::runtime::mentions::mention_responder(&brain.record(), None, &[]),
        None,
        "so the caller falls through to responder_for",
    );
    assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
    assert_eq!(brain.responder_for(None), "chief");
}

/// `@everyone` names the addressed desk's teammates for the turn's context
/// — and still leaves exactly one responder, because it is a list and not a
/// fan-out.
#[test]
fn everyone_names_the_desk_without_choosing_a_responder() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    let mentions = [crate::ports::types::Mention {
        target: crate::ports::types::MentionTarget::Everyone,
        text: "@everyone".to_string(),
        offset: 0,
        quiet: false,
    }];
    assert_eq!(
        crate::runtime::mentions::mention_responder(&brain.record(), None, &mentions),
        None,
        "a broadcast names no single teammate, so the desk lead still answers",
    );
    assert_eq!(
        crate::runtime::mentions::mentioned_agents(
            &brain.record(),
            "eng_desk",
            &mentions,
            Some("engineer"),
        ),
        Vec::<String>::new(),
        "the only member is the responder, and it is not told it was mentioned",
    );
}

/// `@everyone` from the console's default thread (`chat: "main"`) expands
/// against the General desk, not no desk at all. The console-only alias is
/// not a desk key `resolve_desk_id` knows, so the brain folds it — and the
/// other General-desk spellings — to the General desk id before expanding.
#[test]
fn everyone_desk_folds_the_main_thread_alias_to_general() {
    let record = record_with_desk();
    assert_eq!(HarnessBrain::everyone_desk(&record, None), "General");
    assert_eq!(HarnessBrain::everyone_desk(&record, Some("")), "General");
    assert_eq!(
        HarnessBrain::everyone_desk(&record, Some("main")),
        "General"
    );
    assert_eq!(
        HarnessBrain::everyone_desk(&record, Some("General")),
        "General"
    );
    assert_eq!(
        HarnessBrain::everyone_desk(&record, Some("eng_desk")),
        "eng_desk"
    );
}

/// A blueprint that declares a desk under one of the General spellings is
/// grandfathered by this host — `is_general_channel` is guarded on
/// `!desk_exists`, the desk keeps its members, and `responder_for` routes
/// to its lead. The fold must not run over it: asking `resolve_desk_id`
/// for the *name* `General` misses a desk called anything else, and
/// `@everyone` would then expand to the whole roster instead of the two
/// people actually on the line — a broadcast escaping the scope of the one
/// case the fold exists to preserve.
#[test]
fn a_grandfathered_general_desk_keeps_its_own_membership() {
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"

[[agent]]
id = "chief"
role = "Chief of Staff"
tier = "orchestrator"

[[agent]]
id = "engineer"
role = "Engineer"

[[group_chat]]
id = "main"
name = "Front office"
members = ["ceo", "engineer"]
"#,
    )
    .expect("valid manifest");
    let mut record = record_with_desk();
    record.manifest.group_chats = manifest.group_chats;

    // The raw key, not the General fold: this desk answers to it.
    assert_eq!(HarnessBrain::everyone_desk(&record, Some("main")), "main");

    // Every folded alias names the same membership as the raw key. A desk
    // that claims the line by *id* is missed by `resolve_desk_id("General")`,
    // so the alias used to fall through to a `General` desk that does not
    // exist — scoping `@everyone` to the whole roster in a channel whose
    // own lead answers (issue #1743).
    for alias in ["", "General", "general", "MAIN"] {
        assert_eq!(
            HarnessBrain::everyone_desk(&record, Some(alias)),
            "main",
            "the alias {alias:?} must scope @everyone to the claiming desk"
        );
    }
    // And with no such desk, the fold still applies as before.
    assert_eq!(
        HarnessBrain::everyone_desk(&record_with_desk(), Some("main")),
        "General"
    );

    let mentions = [crate::ports::types::Mention {
        target: crate::ports::types::MentionTarget::Everyone,
        text: "@everyone".to_string(),
        offset: 0,
        quiet: false,
    }];
    let expanded = crate::runtime::mentions::mentioned_agents(
        &record,
        &HarnessBrain::everyone_desk(&record, Some("main")),
        &mentions,
        None,
    );
    assert_eq!(
        expanded,
        vec!["ceo".to_string(), "engineer".to_string()],
        "a broadcast stays inside the desk that was addressed"
    );
    assert!(
        !expanded.contains(&"chief".to_string()),
        "and does not reach a teammate who is not on it: {expanded:?}"
    );
}

/// The default responder is the `orchestrator`-tier agent, even when it is
/// not first on the roster.
#[test]
fn default_responder_is_the_orchestrator() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(brain.responder, "chief");
}

/// An addressed desk routes to its lead member (by id or name); anything else
/// — the "General" desk, an unknown id, or no address — falls to the
/// orchestrator.
#[test]
fn responder_for_routes_desk_to_lead_else_orchestrator() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
    assert_eq!(brain.responder_for(Some("Engineering")), "engineer");
    assert_eq!(brain.responder_for(Some("General")), "chief");
    assert_eq!(brain.responder_for(Some("nope")), "chief");
    assert_eq!(brain.responder_for(None), "chief");
}

// ── Issue #151 §3.3: a DM thread reaches the teammate it names ──

/// A chat id naming a roster teammate answers as that teammate, which is
/// what a per-agent DM thread is. Before this it fell through to the
/// orchestrator, so the console would show an agent's thread while someone
/// else answered in it.
#[test]
fn responder_for_routes_a_roster_agent_id_to_that_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(brain.responder_for(Some("engineer")), "engineer");
    assert_eq!(brain.responder_for(Some("chief")), "chief");
}

// ── Issue #1743: who answers the built-in `#general` channel ──

/// An **overlay** desk that took a General spelling before those were
/// reserved must not answer the company-wide line.
///
/// `create_desk` accepted `main` until issue #1743, so this is persisted
/// state rather than a hypothesis. Such a desk is already hidden from
/// `GET .../desks` and refused every mutation, but hiding a desk does not
/// stop it routing: `desk_lead` resolves through
/// `CompanyRecord::resolve_desk_id`, which used to match it, so the console
/// rendered `#general` and named the orchestrator as who answers while this
/// desk's lead answered instead. The resolver declines the key now, and the
/// arm below it hands the line back to the orchestrator.
#[test]
fn responder_for_does_not_let_a_hidden_overlay_desk_answer_the_general_line() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    brain.mutate_record(|r| {
        r.overlay_desks.push(crate::ports::types::OverlayDesk {
            id: "main".into(),
            name: "Front office".into(),
            description: None,
            responder: Default::default(),
            members: vec!["engineer".into()],
            hive: Default::default(),
        })
    });
    for spelling in ["", "main", "Main", "general", "General"] {
        assert_eq!(
            brain.responder_for(Some(spelling)),
            "chief",
            "the orchestrator answers the company-wide line as {spelling:?}"
        );
    }
    // The desk is not retired — it simply has no General key any more, and
    // the desk it is still routes to its own lead.
    assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
}

/// A **teammate** whose id is a General spelling keeps its DM, and does not
/// take the company-wide line with it (issue #1743).
///
/// `mint_agent_id` reserves `main` and `General`, but a manifest can still
/// declare one, and a manifest is not something this host overrules. Before
/// this, `resolve_roster_agent_id` matched the bare key and that teammate
/// answered every unaddressed message — while `GET chat/history?desk=main`
/// returned the *folded General conversation* (`is_general_chat` has folded
/// `""`, `main`, `General` and `general` into one since issue #65). The
/// responder and the transcript disagreed about whose conversation it was.
/// The bare key is the line; `dm:<id>` is the teammate.
#[test]
fn responder_for_gives_the_general_line_to_the_orchestrator_not_a_teammate_called_main() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    brain.mutate_record(|r| {
        r.overlay_agents.push(OverlayAgent {
            provider: None,
            id: "main".into(),
            name: "Mainard".into(),
            role: "Analyst".into(),
            description: None,
            tools: None,
            model: None,
            harness: None,
        })
    });
    assert!(
        brain.record().is_roster_agent("main"),
        "the teammate really is on the roster, so the old arm would have matched"
    );
    assert_eq!(
        brain.responder_for(Some("main")),
        "chief",
        "the bare key is the company's line, whatever a teammate is called"
    );
    assert_eq!(
        brain.responder_for(Some("dm:main")),
        "main",
        "and the teammate keeps its own DM, addressed the way the console addresses one"
    );
}

/// The grandfathered case the two tests above must not break: a
/// `[[group_chat]]` the **blueprint** declares under a General spelling is
/// the company's own General desk, and its lead still answers it.
#[test]
fn responder_for_still_routes_a_blueprint_general_desk_to_its_lead() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    let declared = toml::from_str::<CompanyManifest>(
        r#"
[company]
name = "Acme"

[[agent]]
id = "engineer"
role = "Engineer"

[[group_chat]]
id = "main"
name = "Front office"
members = ["engineer"]
"#,
    )
    .expect("valid manifest")
    .group_chats;
    brain.mutate_record(|r| r.manifest.group_chats.extend(declared));
    assert_eq!(
        brain.responder_for(Some("main")),
        "engineer",
        "a blueprint desk keeps the line and its lead keeps answering it"
    );
}

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

/// A key that resolves to no desk and no teammate still answers as the
/// orchestrator — the fallback is deliberate — but it now says so.
///
/// This is the whole of D2: before it, "nobody addressed anybody" and
/// "somebody addressed a teammate that does not exist" produced the same
/// confident answer from an agent nobody asked, and the tenant log carried
/// nothing to tell them apart.
#[test]
fn responder_for_warns_before_falling_back_to_the_orchestrator() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    // `dm:engineer` was this fixture until issue #982 made it resolve; the
    // key here has to be one that names nothing at all, or the test would
    // pass by asserting the wrong fact and #884's coverage would be gone.
    let logs = logs_from(|| {
        assert_eq!(
            brain.responder_for(Some("dm:nobody_by_that_name")),
            "chief",
            "the fallback itself is unchanged"
        );
    });
    assert!(
        logs.contains("dm:nobody_by_that_name"),
        "the unresolved key must be named so the fall-through is greppable: {logs}"
    );
    assert!(logs.contains("WARN"), "{logs}");

    // …and a key that DOES resolve stays silent, or the line is noise
    // rather than a signal.
    let quiet = logs_from(|| {
        assert_eq!(brain.responder_for(Some("engineer")), "engineer");
        assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
    });
    assert!(quiet.is_empty(), "a resolved key must not warn: {quiet}");
}

/// Issue #982: the console mints a DM channel id as `dm:<teammate-id>`, and
/// a sibling route documents that form as a valid channel key — so a thread
/// keyed on it has to be answered by the teammate it names, not by the
/// orchestrator.
///
/// The prefix is stripped **after** the desk and roster attempts, so this
/// can only ever claim a key that resolved to nothing: `engineer` and
/// `eng_desk` still route exactly as they did, which the test above pins.
#[test]
fn responder_for_answers_a_console_dm_channel_key_as_the_teammate() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    assert_eq!(
        brain.responder_for(Some("dm:engineer")),
        "engineer",
        "a DM channel key addresses the teammate it names"
    );
    assert_eq!(
        brain.responder_for(Some("dm:")),
        "chief",
        "a prefix with nothing after it names nobody"
    );
}

/// A human- or console-typed teammate key resolves case-insensitively to the
/// **canonical** roster id, so a capital letter no longer reads as "nobody"
/// and hands the turn to the orchestrator.
///
/// Returning the canonical id rather than the key as typed is the load-bearing
/// half: the persona lookup downstream matches on the roster id, so echoing
/// `"Engineer"` back would move the miss one layer along instead of fixing it.
#[test]
fn responder_for_resolves_a_teammate_key_case_insensitively() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    for typed in ["Engineer", "ENGINEER", "engineer"] {
        assert_eq!(
            brain.responder_for(Some(typed)),
            "engineer",
            "`{typed}` must reach the engineer under its canonical id"
        );
    }
    // A key that resolves to nothing is still the orchestrator's — folding
    // the case may only claim keys that reached nobody before.
    assert_eq!(brain.responder_for(Some("engineeer")), "chief");
}

/// Desks still win. A desk id is resolved as a desk even if an agent shares
/// the name, so no existing thread changes where it lands.
#[test]
fn a_desk_still_outranks_an_agent_of_the_same_name() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    // `eng_desk` is a desk led by `engineer`; it must resolve through the
    // desk path, not the DM path.
    assert_eq!(brain.responder_for(Some("eng_desk")), "engineer");
    // And an id that is neither still reaches the orchestrator.
    assert_eq!(brain.responder_for(Some("not-a-teammate")), "chief");
}

/// An operator-added overlay member is resolved as a desk's lead (issue #72):
/// on a desk the manifest left empty, the overlay addition becomes the lead,
/// and an addressed message routes to it. Proves `desk_lead`/`responder_for`
/// read the effective (manifest ∪ overlay) membership.
#[test]
fn overlay_member_resolves_as_desk_lead() {
    let dir = tempfile::tempdir().unwrap();
    // `design` is a manifest desk with no declared members; the operator adds
    // `engineer` to it through the overlay.
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "chief"
role = "Chief of Staff"
tier = "orchestrator"

[[agent]]
id = "engineer"
role = "Engineer"

[[group_chat]]
id = "design"
name = "Design"
"#,
    )
    .expect("valid manifest");
    let record = CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest,
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        overlay_agents: Vec::new(),
        overlay_desk_members: vec![crate::ports::types::OverlayDeskMember {
            desk_id: "design".to_string(),
            agent_id: "engineer".to_string(),
        }],
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
    };
    let (brain, _tasks) = brain_over(dir.path(), record);
    assert_eq!(
        delegation::desk_lead(&brain.record(), "design"),
        Some("engineer".to_string())
    );
    assert_eq!(brain.responder_for(Some("design")), "engineer");
}

/// The operator's desk hierarchy drives the desk lead: a desk with manifest
/// members `[eng1, eng2]` plus an overlay `cto`, ordered `[cto, eng1, eng2]`,
/// resolves its lead to `cto` — `desk_lead` reads `effective_desk_members`,
/// so the reorder flows through with no change to the resolver (issue #131).
#[test]
fn desk_order_drives_the_desk_lead() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "eng1"
role = "Engineer One"

[[agent]]
id = "eng2"
role = "Engineer Two"

[[group_chat]]
id = "eng"
name = "Engineering"
members = ["eng1", "eng2"]
"#,
    )
    .expect("valid manifest");
    let record = CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest,
        ledger: Vec::new(),
        lifecycle: "running".to_string(),
        overlay_agents: vec![crate::ports::types::OverlayAgent {
            provider: None,
            id: "cto".to_string(),
            name: "Cto".to_string(),
            role: "CTO".to_string(),
            description: None,
            tools: None,
            model: None,
            harness: None,
        }],
        overlay_desk_members: vec![crate::ports::types::OverlayDeskMember {
            desk_id: "eng".to_string(),
            agent_id: "cto".to_string(),
        }],
        overlay_desk_order: vec![crate::ports::types::OverlayDeskOrder {
            desk_id: "eng".to_string(),
            ordered: vec!["cto".to_string(), "eng1".to_string(), "eng2".to_string()],
        }],
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
    };
    let (brain, _tasks) = brain_over(dir.path(), record);
    assert_eq!(
        delegation::desk_lead(&brain.record(), "eng"),
        Some("cto".to_string())
    );
}

/// Regression for the builder seeding path (#133): a desk-order change written
/// to the store must take effect on routing once the brain is rebuilt from the
/// persisted record. The builder used to construct the brain with an empty
/// `overlay_desk_order`, so desk chats kept routing to the pre-reorder lead.
/// Here we persist a record, build a brain from the loaded record (blueprint
/// lead), then write a new order and rebuild the brain from the reloaded record
/// — the lead must update, not stay stale.
#[tokio::test]
async fn desk_order_change_updates_routing_after_rebuild() {
    use crate::ports::store::CompanyStore;

    let dir = tempfile::tempdir().unwrap();
    let store = FsCompanyStore::new(dir.path());
    let manifest = toml::from_str(
        r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "eng1"
role = "Engineer One"

[[agent]]
id = "eng2"
role = "Engineer Two"

[[group_chat]]
id = "eng"
name = "Engineering"
members = ["eng1", "eng2"]
"#,
    )
    .expect("valid manifest");
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
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
        })
        .await
        .unwrap();

    // Brain built from the persisted record before any reorder: blueprint lead.
    let loaded = store.load(&id).await.unwrap().unwrap();
    let (brain, _tasks) = brain_over(dir.path(), loaded);
    assert_eq!(
        delegation::desk_lead(&brain.record(), "eng"),
        Some("eng1".to_string()),
        "blueprint lead before reorder"
    );

    // Operator reorders the desk (as `set_desk_order` does), promoting eng2.
    let mut record = store.load(&id).await.unwrap().unwrap();
    record
        .overlay_desk_order
        .push(crate::ports::types::OverlayDeskOrder {
            desk_id: "eng".to_string(),
            ordered: vec!["eng2".to_string(), "eng1".to_string()],
        });
    store.save(&record).await.unwrap();

    // Rebuild the brain from the reloaded record: routing follows the reorder,
    // no stale lead.
    let reloaded = store.load(&id).await.unwrap().unwrap();
    let (rebuilt, _tasks2) = brain_over(dir.path(), reloaded);
    assert_eq!(
        delegation::desk_lead(&rebuilt.record(), "eng"),
        Some("eng2".to_string()),
        "reorder did not take effect on routing after rebuild"
    );
}

/// A `spawn_task` delegation opens a To-do card and surfaces no bubble.
#[tokio::test]
async fn spawn_task_delegation_opens_a_todo_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_desk(dir.path());
    let out = brain
        .run_delegation(
            Delegation::SpawnTask {
                title: "Draft the plan".to_string(),
                note: Some("by friday".to_string()),
                assignee: Some("engineer".to_string()),
            },
            None,
        )
        .await
        .expect("delegation runs");
    assert!(
        out.bubble.is_none() && out.desk_reply.is_none(),
        "spawn_task surfaces nothing to relay or bubble"
    );

    let cards = tasks.list(&CompanyId::new("acme")).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].title, "Draft the plan");
    assert_eq!(cards[0].column, COLUMN_TODO);
    assert_eq!(cards[0].assignee, "engineer");
    // Issue #246: it surfaces no *bubble*, but it no longer surfaces
    // *nothing* — the card it opened is reported, which is what lets the
    // caller tell the operator a card exists instead of leaving them to
    // notice it on the board.
    assert_eq!(
        out.spawned_task.as_deref(),
        Some(cards[0].id.as_str()),
        "the opened card must be reported, and be the one actually written"
    );
}

/// A spawned card is grounded on the same terms an assigned one is: a name
/// that resolves to nobody opens the card unowned, rather than stamping an
/// owner the board renders and no dispatch can reach.
#[tokio::test]
async fn spawn_task_refuses_to_stamp_an_off_roster_owner_on_a_new_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_desk(dir.path());
    brain
        .run_delegation(
            Delegation::SpawnTask {
                title: "Draft the plan".to_string(),
                note: None,
                assignee: Some("not-a-real-agent-xyz".to_string()),
            },
            None,
        )
        .await
        .expect("delegation runs");

    let cards = tasks.list(&CompanyId::new("acme")).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].assignee, "",
        "an unresolvable name leaves the card unowned"
    );
}

/// Issue #246: a chat turn that opened a card says so on the bubble it
/// answered from. Before this the card appeared on the board and the reply
/// carried nothing tying the two together, so an operator had no way to
/// tell a turn that opened work from one that only talked about it.
#[tokio::test]
async fn a_turn_that_opens_a_card_reports_it_on_the_operator_bubble() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::SpawnTask {
            title: "Draft the announcement".to_string(),
            note: None,
            assignee: None,
        })],
    );

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "we should announce this".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    let reported = bubble.task_id.as_deref().expect("the bubble names a card");
    let cards = brain
        .deps
        .tasks
        .as_ref()
        .unwrap()
        .list(&brain.record().id)
        .await
        .unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        reported, cards[0].id,
        "the reported card must be the one on the board"
    );
}

/// Issue #246, the documented limitation stated as a test rather than only
/// as prose: a turn that opens several cards reports the **first**. The
/// journal field this feeds is a single optional id, so widening it would
/// break the byte-identical round-trip every already-stored reply relies
/// on. Pinned to *first* — not "whichever won" — because a later spawn
/// silently overwriting an earlier one would make the reported card depend
/// on queue order, which is the model's choice, not a contract.
#[tokio::test]
async fn a_turn_that_opens_several_cards_reports_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _provider) = brain_that_delegates_with(
        dir.path(),
        vec![vec![
            Delegation::SpawnTask {
                title: "First".to_string(),
                note: None,
                assignee: None,
            },
            Delegation::SpawnTask {
                title: "Second".to_string(),
                note: None,
                assignee: None,
            },
        ]],
        TurnFaults::default(),
    );

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "two things".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let reported = result.channel_responses[0]
        .task_id
        .as_deref()
        .expect("the bubble names a card");
    let cards = brain
        .deps
        .tasks
        .as_ref()
        .unwrap()
        .list(&brain.record().id)
        .await
        .unwrap();
    assert_eq!(cards.len(), 2, "both cards are opened either way");
    let first = cards
        .iter()
        .find(|c| c.title == "First")
        .expect("the first card exists");
    assert_eq!(
        reported, first.id,
        "the bubble reports the first card opened, not the last"
    );
}

/// The other side of the same contract: a turn that opened no card must
/// leave the field empty, so no bubble grows a "card opened" chip it has
/// not earned.
#[tokio::test]
async fn a_turn_that_opens_no_card_reports_none() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _provider) = brain_that_delegates(dir.path(), Vec::new());

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "status?".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert!(
        result.channel_responses[0].task_id.is_none(),
        "an ordinary chat turn must not claim a card"
    );
}

// ── Issue #186 part b: orchestrator lifecycle authority ────────────────

/// `assign_task` changes who owns an existing card, records the change in
/// the orchestrator's voice, and — deliberately — does **not** touch the
/// column: dispatch fires from `CompanyRuntime::upsert_task`, which the
/// `TaskStore` port this drain writes through cannot reach.
#[tokio::test]
async fn assign_task_reassigns_the_card_without_dispatching_it() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-assign", "engineer");
    c.column = COLUMN_TODO.to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    let out = brain
        .run_delegation(
            Delegation::AssignTask {
                task_id: "t-assign".to_string(),
                assignee: "ceo".to_string(),
                note: Some("closer to the customer".to_string()),
            },
            None,
        )
        .await
        .expect("delegation runs");
    assert!(
        out.bubble.is_none() && out.desk_reply.is_none(),
        "the orchestrator is mid-turn; a second voice here would be it talking to itself"
    );

    let after = only_card(&tasks).await;
    assert_eq!(after.assignee, "ceo");
    assert_eq!(
        after.column, COLUMN_TODO,
        "assignment records ownership; it must not start the work"
    );
    let note = after.note.expect("note");
    assert!(note.contains("assigned to ceo"), "{note}");
    assert!(note.contains("closer to the customer"), "{note}");
    assert!(
        note.contains(&format!("[{}]", brain.orchestrator())),
        "the assignment is recorded in the orchestrator's voice: {note}"
    );
}

/// #205: `assign_task` takes its `assignee` from an LLM tool call, so it can
/// name somebody the company does not have just as easily as the operator's
/// free-text field can. The bad name must not reach the card — the previous
/// owner stays, and the refusal is recorded on the note.
#[tokio::test]
async fn assign_task_refuses_an_off_roster_assignee_and_keeps_the_current_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-assign", "engineer");
    c.column = COLUMN_TODO.to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    brain
        .run_delegation(
            Delegation::AssignTask {
                task_id: "t-assign".to_string(),
                assignee: "Shane".to_string(),
                note: None,
            },
            None,
        )
        .await
        .expect("delegation runs");

    let after = only_card(&tasks).await;
    assert_eq!(
        after.assignee, "engineer",
        "a name nobody answers to must not displace the real owner"
    );
    let note = after.note.expect("note");
    assert!(note.contains("could not assign to Shane"), "{note}");
}

/// #214 review: a blank `assignee` resolves to `Unassigned`, whose canonical
/// form is `""`. Clearing the owner is correct — unassigning is a real
/// request — but the note must say so. It used to fall through the named
/// arm and record `assigned to ` with nothing after it.
#[tokio::test]
async fn assign_task_with_a_blank_assignee_clears_the_owner_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-assign", "engineer");
    c.column = COLUMN_TODO.to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    brain
        .run_delegation(
            Delegation::AssignTask {
                task_id: "t-assign".to_string(),
                assignee: "   ".to_string(),
                note: None,
            },
            None,
        )
        .await
        .expect("delegation runs");

    let after = only_card(&tasks).await;
    assert_eq!(
        after.assignee, "",
        "a blank assignee unassigns the card, which is a legitimate write"
    );
    let note = after.note.expect("note");
    assert!(
        note.contains("cleared the assignee"),
        "the note names the effect rather than trailing off: {note}"
    );
    assert!(
        !note.contains("assigned to "),
        "the truncated 'assigned to <nothing>' note must not come back: {note}"
    );
}

/// Approving finishes a board-created card: this is #171's `in_review →
/// done` write (PR #179) for the card shape #179's own origin rule cannot
/// reach, with the verdict recorded on the note.
#[tokio::test]
async fn review_approve_records_the_verdict_and_completes_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-review", "engineer");
    c.column = "in_review".to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    brain
        .run_delegation(
            Delegation::ReviewTask {
                task_id: "t-review".to_string(),
                decision: lifecycle::ReviewDecision::Approve,
                note: Some("ships as-is".to_string()),
            },
            None,
        )
        .await
        .expect("delegation runs");

    let after = only_card(&tasks).await;
    assert_eq!(
        after.column, "done",
        "an approving verdict is the in_review -> done transition (#171)"
    );
    let note = after.note.expect("note");
    assert!(note.contains("reviewed: approved"), "{note}");
    assert!(note.contains("ships as-is"), "{note}");
}

/// `revise` is a transition #186 does own: the card goes back to the
/// To-do so it can be picked up and re-dispatched.
#[tokio::test]
async fn review_revise_sends_the_card_back_to_todo() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());
    let mut c = card("t-revise", "engineer");
    c.column = "in_review".to_string();
    tasks.upsert(&CompanyId::new("acme"), &c).await.unwrap();

    brain
        .run_delegation(
            Delegation::ReviewTask {
                task_id: "t-revise".to_string(),
                decision: lifecycle::ReviewDecision::Revise,
                note: None,
            },
            None,
        )
        .await
        .expect("delegation runs");

    let after = only_card(&tasks).await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(
        after.note.expect("note").contains("needs another pass"),
        "the verdict must be recorded even without a reviewer comment"
    );
}

/// A card that has since been deleted is a silent no-op, matching every
/// other task path in this file — never an error that kills the turn.
#[tokio::test]
async fn a_lifecycle_delegation_for_a_missing_card_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks) = brain_with_tasks(dir.path());

    for delegation in [
        Delegation::AssignTask {
            task_id: "ghost".to_string(),
            assignee: "ceo".to_string(),
            note: None,
        },
        Delegation::ReviewTask {
            task_id: "ghost".to_string(),
            decision: lifecycle::ReviewDecision::Approve,
            note: None,
        },
    ] {
        let out = brain
            .run_delegation(delegation, None)
            .await
            .expect("a missing card must not error");
        assert!(out.bubble.is_none() && out.desk_reply.is_none());
    }
    assert!(
        tasks
            .list(&CompanyId::new("acme"))
            .await
            .unwrap()
            .is_empty()
    );
}

/// A `delegate_to_desk` delegation runs the desk lead and hands its reply
/// back to relay (a `DeskReply` attributed to the lead, no standalone
/// bubble); an unknown desk yields nothing.
#[tokio::test]
async fn delegate_to_desk_delegation_answers_as_the_desk_lead() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _tasks) = brain_with_desk(dir.path());
    // The pool must have the roster before a member turn can run.
    brain
        .pool
        .ensure(&brain.record(), &brain.deps)
        .await
        .expect("roster");

    let out = brain
        .run_delegation(
            Delegation::DelegateToDesk {
                desk: "eng_desk".to_string(),
                instruction: "ship-marker".to_string(),
            },
            None,
        )
        .await
        .expect("delegation runs");
    // The answer comes back as a DeskReply to relay — not a standalone
    // bubble — attributed to the desk lead, and the mock provider echoes the
    // instruction, proving the member's turn ran.
    assert!(
        out.bubble.is_none(),
        "the desk reply is relayed, not bubbled"
    );
    let desk = out.desk_reply.expect("desk lead replies");
    assert_eq!(desk.member, "engineer");
    assert!(desk.reply.contains("ship-marker"), "{:?}", desk.reply);

    // An unknown desk delegates to nobody.
    let none = brain
        .run_delegation(
            Delegation::DelegateToDesk {
                desk: "ghost".to_string(),
                instruction: "hello".to_string(),
            },
            None,
        )
        .await
        .expect("delegation runs");
    assert!(
        none.bubble.is_none() && none.desk_reply.is_none(),
        "an unknown desk yields nothing"
    );
}

// --- MCP failure drain --------------------------------------------------

/// A recorded MCP failure re-skins into an **error step** on the operator
/// bubble's timeline AND a scrubbed `McpCallFailed` audit event when the
/// event log is wired (the Activity-trace re-skin of the old warning bubble).
#[tokio::test]
async fn mcp_failures_surface_as_error_steps_and_event() {
    use crate::harness::mcp_probe::McpFailure;
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let events: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let failures = crate::harness::mcp_probe::McpFailureQueue::default();
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir.path())),
        store: Arc::new(FsCompanyStore::new(dir.path())),
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
        events: Some(events.clone()),
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: failures.clone(),
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
    let brain = HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record());

    // A failure recorded during the turn (its message already scrubbed).
    failures.push(McpFailure {
        server: "browserbase".into(),
        tool: "browse".into(),
        status: "tool_call_rejected".into(),
        hint: None,
        scrubbed_message: "server rejected the call".into(),
    });

    let mut steps: Vec<TurnStep> = Vec::new();
    // `None` — this is the chat-turn drain, which journals no `task_id`
    // (#185). The dispatch drain passes the card id; see `run_task`.
    brain
        .surface_mcp_failures(&mut steps, None)
        .await
        .expect("drain surfaces failures");

    assert_eq!(steps.len(), 1, "one error step");
    assert_eq!(steps[0].kind, TurnStepKind::Note);
    assert_eq!(steps[0].status, TurnStepStatus::Error);
    assert!(
        steps[0].label.contains("browserbase"),
        "{:?}",
        steps[0].label
    );
    assert_eq!(steps[0].detail.as_deref(), Some("server rejected the call"));

    let logged = events
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(
        logged.iter().any(|e| matches!(
            &e.event,
            CompanyEvent::McpCallFailed { server, status, .. }
                if server == "browserbase" && status == "tool_call_rejected"
        )),
        "an McpCallFailed audit event was journaled"
    );
}

/// #185 review follow-up: one bad journal write must not swallow the rest of
/// the batch.
///
/// `McpFailureQueue::drain` is a `mem::take` — by the time the loop runs the
/// queue is empty and the batch exists only in that iterator. Propagating
/// the first append error with `?` therefore did not merely skip one audit
/// event, it discarded every failure behind it with nothing left to retry
/// from. Journaling is per-item best-effort so the drain always completes.
#[tokio::test]
async fn a_failed_journal_write_does_not_swallow_the_rest_of_the_drain() {
    use crate::harness::mcp_probe::McpFailure;
    use crate::ports::EventLog;
    use crate::ports::types::{EventSeq, StoredEvent};
    use futures::stream::{self, BoxStream};

    /// An event log whose FIRST append fails and whose later appends
    /// succeed, recording what got through.
    #[derive(Default)]
    struct FailFirstLog {
        seen: StdMutex<Vec<CompanyEvent>>,
        appends: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl EventLog for FailFirstLog {
        async fn append(&self, _id: &CompanyId, event: CompanyEvent) -> Result<EventSeq> {
            let nth = self
                .appends
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if nth == 0 {
                return Err(crate::error::OpenCompanyError::Store(
                    "journal unavailable".to_string(),
                ));
            }
            let mut guard = self.seen.lock().unwrap();
            guard.push(event);
            Ok(EventSeq::new(guard.len() as u64))
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            _seq: EventSeq,
            _limit: usize,
        ) -> Result<Vec<StoredEvent>> {
            Ok(Vec::new())
        }
        fn subscribe(
            &self,
            _id: &CompanyId,
        ) -> BoxStream<'static, crate::ports::events::EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let log = Arc::new(FailFirstLog::default());
    let failures = crate::harness::mcp_probe::McpFailureQueue::default();
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir.path())),
        store: Arc::new(FsCompanyStore::new(dir.path())),
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
        events: Some(log.clone()),
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: failures.clone(),
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
    let brain = HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record());

    for server in ["first", "second", "third"] {
        failures.push(McpFailure {
            server: server.into(),
            tool: "browse".into(),
            status: "tool_call_rejected".into(),
            hint: None,
            scrubbed_message: "server rejected the call".into(),
        });
    }

    let mut steps: Vec<TurnStep> = Vec::new();
    brain
        .surface_mcp_failures(&mut steps, Some("t1"))
        .await
        .expect("a journal error is best-effort, not fatal");

    // Every failure is re-skinned onto the timeline regardless…
    assert_eq!(steps.len(), 3, "all three failures surfaced as steps");
    // …and the two after the failed write still reached the journal. Before
    // this fix `seen` was empty: the `?` returned on `first` and `second` /
    // `third` were dropped with the drained batch.
    let seen = log.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "the drain continued past the failed append");
    assert!(
        seen.iter().any(|e| matches!(
            e,
            CompanyEvent::McpCallFailed { server, .. } if server == "third"
        )),
        "the last failure in the batch was still journaled"
    );
}

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

/// **A hive episode drains queued MCP failures and surfaces a response.**
///
/// Two findings in one test, because they share the exact same
/// production call site (the hive branch of `run_cycle_scoped`) and a
/// fix to one without the other leaves the interaction untested:
///
/// - Before the fix, the hive branch never called
///   `self.surface_mcp_failures`, so an MCP tool-call failure queued
///   during a member's turn produced neither an error step nor an
///   `McpCallFailed` journal row — it sat on `self.deps.mcp_failures`
///   until a later, unrelated chat turn cleared it silently.
/// - Before the fix, the hive branch pushed nothing onto
///   `channel_responses`, so `CycleResult.channel_responses` — which
///   becomes `CycleReport.responses`, what a synchronous chat-API caller
///   and `emit_cycle_webhooks` both read — stayed empty even though the
///   desk had just answered, and no `work.completed` webhook ever fired.
#[tokio::test]
async fn a_hive_episode_drains_mcp_failures_and_surfaces_a_response() {
    use crate::harness::mcp_probe::McpFailure;
    use crate::ports::EventLog;
    use crate::ports::types::EventSeq;
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let events: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let failures = crate::harness::mcp_probe::McpFailureQueue::default();
    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir.path())),
        store: Arc::new(FsCompanyStore::new(dir.path())),
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
        events: Some(events.clone()),
        delegations: orchestrator::DelegationQueue::default(),
        workflow_runner: orchestrator::WorkflowRunnerHandle::default(),
        mcp_failures: failures.clone(),
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
    let brain = HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_with_hive_desk());

    // Queued as though a tool call inside a hive member's turn failed —
    // the same shape `mcp_failures_surface_as_error_steps_and_event`
    // seeds for the ordinary responder path.
    failures.push(McpFailure {
        server: "browserbase".into(),
        tool: "browse".into(),
        status: "tool_call_rejected".into(),
        hint: None,
        scrubbed_message: "server rejected the call".into(),
    });

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "Decide the rollout.".into(),
                by: None,
                chat: Some("eng_desk".into()),
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("the cycle runs even though a tool call failed inside it");

    // Finding: a synchronous caller (and `emit_cycle_webhooks`, which reads
    // the exact same collection) must see the hive desk's answer.
    assert_eq!(
        result.channel_responses.len(),
        1,
        "the hive episode's closing report must reach channel_responses: {:?}",
        result.channel_responses
    );

    // Finding: the queued MCP failure must be drained during the episode,
    // not left for a later, unrelated turn.
    assert!(
        failures.drain().is_empty(),
        "the hive episode must drain the queue itself"
    );
    let logged = events
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
        .await
        .expect("read events");
    assert!(
        logged.iter().any(|e| matches!(
            &e.event,
            CompanyEvent::McpCallFailed { server, status, .. }
                if server == "browserbase" && status == "tool_call_rejected"
        )),
        "an McpCallFailed audit event must be journaled from inside the hive episode: \
         {logged:?}"
    );
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

/// **Two operator messages to the same hive desk in one cycle must not
/// fold into each other.**
///
/// Both `OperatorMessage` events are journaled up front (mirroring
/// `CycleRequest::event_seqs`, which names a sequence every caller
/// already durable-wrote before the brain ever sees the event) and
/// `run_cycle_scoped` then answers each in turn on the *same* desk. The
/// first episode (`ALPHA_QUESTION`) runs to completion before the second
/// (`BETA_QUESTION`) ever opens, so by the time episode B's very first
/// `EpisodeDriver::run` iteration reads the desk's transcript, episode
/// A's turns already sit in the journal at sequences *above* B's own
/// trigger.
///
/// Before the fix, a top-level hive send never threaded its turns to the
/// triggering operator message (`in_thread(*parent)` with `parent: None`),
/// so both episodes shared the same desk-channel conversation. Episode
/// B's fold has only a lower watermark and no upper bound, so it read
/// episode A's already-carried `#alpha` votes as its own live traces and
/// converged on `#alpha` immediately — zero turns of its own, and on the
/// wrong question entirely.
///
/// After the fix, each episode's turns are parented to its own triggering
/// message, so episode B's conversation is a distinct thread and cannot
/// see episode A's turns at all: it deliberates on its own and converges
/// on `#beta`.
#[tokio::test]
async fn two_hive_desk_episodes_in_one_cycle_do_not_fold_into_each_other() {
    use crate::store::FsEventLog;

    let dir = tempfile::tempdir().unwrap();
    let events: Arc<dyn crate::ports::EventLog> = Arc::new(FsEventLog::new(dir.path()));
    let company = CompanyId::new("acme");

    let message_a = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "ALPHA_QUESTION".into(),
        by: None,
        chat: Some("eng_desk".into()),
        deliverable: None,
        attachments: Vec::new(),
    };
    let message_b = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "BETA_QUESTION".into(),
        by: None,
        chat: Some("eng_desk".into()),
        deliverable: None,
        attachments: Vec::new(),
    };
    // Both journaled before the brain ever runs a cycle over them —
    // exactly the ordering `CycleRequest::event_seqs`'s doc names as the
    // caller's contract, and the ordering the finding depends on: episode
    // A's turns (journaled below) land at sequences above `seq_b`.
    let seq_a = events
        .append(&company, message_a.clone())
        .await
        .expect("journal message A");
    let seq_b = events
        .append(&company, message_b.clone())
        .await
        .expect("journal message B");

    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        provider: Arc::new(HiveTopicProvider),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(dir.path())),
        store: Arc::new(FsCompanyStore::new(dir.path())),
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
        events: Some(events.clone()),
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
    let brain = HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record_with_hive_desk());

    let req = CycleRequest {
        cycle_id: "cycle-hive-isolation".to_string(),
        company_id: company.clone(),
        events: vec![message_a, message_b],
        event_seqs: vec![seq_a, seq_b],
        policy: None,
    };
    let result = brain
        .run_cycle(req, &NoopHost)
        .await
        .expect("both hive episodes in the cycle answer");

    assert_eq!(
        result.channel_responses.len(),
        2,
        "each operator message gets its own hive-report response: {:?}",
        result.channel_responses
    );
    let report_a = &result.channel_responses[0].text;
    let report_b = &result.channel_responses[1].text;
    assert!(
        report_a.contains("#alpha"),
        "episode A must settle on its own question: {report_a}"
    );
    assert!(
        report_b.contains("#beta") && !report_b.contains("#alpha"),
        "episode B must settle on its OWN question rather than inheriting \
         episode A's already-carried #alpha vote: {report_b}"
    );

    // The journal itself must show the two episodes parented to their own
    // triggering message, not sharing one unparented desk-channel thread.
    let logged = events
        .read_from(&company, EventSeq::new(0), usize::MAX)
        .await
        .expect("read the journal back");
    let turns_under = |root: EventSeq| {
        logged
            .iter()
            .filter(|stored| {
                matches!(
                    &stored.event,
                    CompanyEvent::AgentReply { parent, .. } if *parent == Some(root)
                )
            })
            .count()
    };
    assert!(
        turns_under(seq_a) > 0,
        "episode A's turns must be parented to message A: {logged:?}"
    );
    assert!(
        turns_under(seq_b) > 0,
        "episode B's turns must be parented to message B rather than left \
         unparented on the shared desk channel: {logged:?}"
    );
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

/// The regression for #172: a `RequireApproval` recorded during a turn is
/// **parked** on the host, so it lands in the journal the Approvals page
/// reads instead of being narrated away in chat and lost.
///
/// `ParkingHost` panics on `emit_effect`, which pins the other half of the
/// fix: the request must NOT be re-evaluated by the runtime gate (which
/// allows — and so silently "executes" — the `Other` group most gated tool
/// calls classify into).
#[tokio::test]
async fn approval_requests_are_parked_for_the_operator() {
    use crate::harness::policy::{ApprovalPolicy, ApprovalRequestQueue};
    use openhuman_core::agent::tool_policy::{
        ToolCallContext, ToolPolicy, ToolPolicyDecision, ToolPolicyRequest,
    };

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());

    // Exactly what a supervised policy records when the agent reaches for a
    // gated tool mid-turn.
    let policy = ApprovalPolicy::new(
        &crate::company::Policy {
            mode: "supervised".to_string(),
            always_approve: Vec::new(),
            auto_approve_under_usd: None,
            approval_ttl_hours: None,
        },
        None,
    )
    .with_requests(requests.clone());
    let args = crate::policy::test_support::composio_send_args();
    let request = ToolPolicyRequest::new(
        "composio_execute",
        args.clone(),
        ToolCallContext::session("s", "chat", "ceo", "call-1", 0),
    );
    assert!(
        matches!(
            policy.check(&request).await,
            ToolPolicyDecision::RequireApproval { .. }
        ),
        "the fixture must reproduce a gated call"
    );
    assert_eq!(requests.queued(), 1, "the decision was recorded to park");

    let host = ParkingHost::default();
    brain
        .park_approval_requests(&host)
        .await
        .expect("the drain parks");

    let parked = host.parked();
    assert_eq!(parked.len(), 1, "one approval reached the operator");
    assert_eq!(parked[0].kind, "composio_execute");
    assert_eq!(
        parked[0].payload, args,
        "the call's arguments are preserved"
    );
    assert_eq!(requests.queued(), 0, "the queue is drained");
}

/// A second drain parks nothing: the queue is emptied, so a later cycle
/// can't re-park a request the operator has already been shown.
#[tokio::test]
async fn draining_twice_parks_nothing_the_second_time() {
    use crate::harness::policy::{ApprovalRequest, ApprovalRequestQueue};
    use crate::ports::types::EffectGroup;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());
    requests.push(ApprovalRequest {
        tool: "media_generate_image".to_string(),
        reason: "supervised".to_string(),
        effect: Effect {
            kind: "media_generate_image".to_string(),
            group: EffectGroup::Spend,
            amount_usd: None,
            established_thread: false,
            first_time_counterparty: false,
            payload: serde_json::json!({ "prompt": "a logo" }),
            agent: None,
            run_id: None,
        },
    });

    let host = ParkingHost::default();
    brain.park_approval_requests(&host).await.expect("drain");
    brain
        .park_approval_requests(&host)
        .await
        .expect("second drain");
    assert_eq!(host.parked().len(), 1, "parked once, not twice");
}

/// Issue #561: a turn that gates more calls than one turn may raise tells
/// the operator so, with the count.
///
/// The cap itself is not the bug and is not touched here. The bug is that
/// exceeding it was **silent**: the operator saw eight cards and had no way
/// to learn that five more gated calls had happened, been refused, and been
/// dropped. Eight cards and no notice is indistinguishable from "eight is
/// all there was".
#[tokio::test]
async fn a_turn_that_overflows_the_cap_tells_the_operator_how_many_were_dropped() {
    use crate::harness::policy::{ApprovalRequest, ApprovalRequestQueue};
    use crate::ports::types::EffectGroup;

    let cap = crate::harness::policy::MAX_APPROVAL_REQUESTS_PER_TURN;
    let over = 5;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());
    for i in 0..(cap + over) {
        requests.push(ApprovalRequest {
            tool: "composio_execute".to_string(),
            reason: "supervised".to_string(),
            effect: Effect {
                kind: "composio_execute".to_string(),
                group: EffectGroup::Send,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                // Distinct payloads, or `push` would dedupe them and the
                // queue would never reach the cap in the first place.
                payload: crate::policy::test_support::composio_unclassified_args_numbered(i),
                agent: None,
                run_id: None,
            },
        });
    }

    let host = ParkingHost::default();
    let notice = brain
        .park_approval_requests(&host)
        .await
        .expect("drain")
        .expect("an overflowing turn has something to tell the operator");

    assert_eq!(host.parked().len(), cap, "the cap still holds");
    assert!(
        notice.contains(&over.to_string()),
        "the operator is told HOW MANY were dropped, not just that some were: {notice}"
    );
    assert!(
        notice.contains(&cap.to_string()),
        "…and what the limit was, so the number means something: {notice}"
    );
}

/// The ordinary turn stays quiet. A notice on every cycle would train the
/// operator to scroll past the one that matters.
#[tokio::test]
async fn a_turn_within_the_cap_raises_no_notice() {
    use crate::harness::policy::{ApprovalRequest, ApprovalRequestQueue};
    use crate::ports::types::EffectGroup;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());
    requests.push(ApprovalRequest {
        tool: "composio_execute".to_string(),
        reason: "supervised".to_string(),
        effect: Effect {
            kind: "composio_execute".to_string(),
            group: EffectGroup::Send,
            amount_usd: None,
            established_thread: false,
            first_time_counterparty: false,
            payload: crate::policy::test_support::composio_send_args(),
            agent: None,
            run_id: None,
        },
    });

    let host = ParkingHost::default();
    assert!(
        brain
            .park_approval_requests(&host)
            .await
            .expect("drain")
            .is_none(),
        "one request, a cap of 8: nothing was dropped and nothing is said"
    );
    assert_eq!(host.parked().len(), 1, "and the request itself still parks");
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

/// One failed park must not take the rest of the batch — or the turn's reply
/// — down with it. `drain` has already emptied the shared queue, so a `?`
/// here would lose every later request forever and abort `run_cycle`,
/// reproducing for the remainder of the batch exactly the silent
/// disappearance this issue fixes.
#[tokio::test]
async fn a_failed_park_does_not_drop_the_rest_of_the_batch() {
    use crate::harness::policy::{ApprovalRequest, ApprovalRequestQueue};
    use crate::ports::types::EffectGroup;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());
    for tool in ["first_tool", "second_tool", "third_tool"] {
        requests.push(ApprovalRequest {
            tool: tool.to_string(),
            reason: "supervised".to_string(),
            effect: Effect {
                kind: tool.to_string(),
                group: EffectGroup::Other,
                amount_usd: None,
                established_thread: false,
                first_time_counterparty: false,
                payload: serde_json::json!({ "tool": tool }),
                agent: None,
                run_id: None,
            },
        });
    }

    let host = FlakyParkingHost::default();
    let notice = brain
        .park_approval_requests(&host)
        .await
        .expect("a park failure is surfaced without aborting the batch")
        .expect("the operator is told a request was not saved");

    // The first park failed; the two after it still reached the operator.
    let parked = host.parked();
    assert_eq!(parked.len(), 2, "the batch continued past the failure");
    assert_eq!(parked[0].kind, "second_tool");
    assert_eq!(parked[1].kind, "third_tool");
    assert!(notice.contains("1 approval request could not be saved"));
    assert!(notice.contains("Ask the agent to request approval again"));
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

/// The arm that made #243 visible: an approved grant re-dispatches its agent
/// with the exact arguments, answers on that agent's channel, and journals
/// the reply.
///
/// Before this arm existed, `ApprovalResolved` fell into `_ => {}`: no turn,
/// no response, and the cycle ended on the "Acknowledged." fallback. The
/// operator approved, read "Acknowledged.", and nothing ran — which looks
/// exactly like success.
///
/// `MockProvider` echoes the user message back, so the reply text IS the
/// instruction the agent received — which is what makes argument fidelity
/// assertable offline.
#[tokio::test]
async fn an_approved_grant_redispatches_its_agent_with_the_exact_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    // Issue #470: a real catalogued send, with the action's own parameters
    // under `arguments` where the tool's schema puts them — so the
    // re-dispatch path this test covers carries a call the classifier can
    // actually read.
    let args = crate::policy::test_support::composio_args_with(
        crate::policy::test_support::COMPOSIO_SEND_SLUG,
        serde_json::json!({ "to": "a@b.test" }),
    );
    requests
        .grants()
        .grant(crate::runtime::grants::GrantedCall {
            approval_id: ApprovalId::new("appr-1"),
            agent: "ceo".into(),
            tool: "composio_execute".into(),
            args: args.clone(),
            at_millis: now_millis(),
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
        });
    let brain = brain_with_queue_and_events(dir.path(), requests, log.clone());

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-1", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    // A real bubble, on the GRANTING agent's channel — not the generic
    // "Acknowledged." fallback and not the operator channel.
    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    assert_eq!(bubble.channel, "ceo");
    assert_ne!(bubble.text, "Acknowledged.");

    // The instruction carried the tool and the arguments VERBATIM. A model
    // that re-issues with drifted arguments re-parks (see the policy tests),
    // so the fidelity of this string is what makes the round-trip land.
    assert!(bubble.text.contains("composio_execute"), "{}", bubble.text);
    assert!(
        bubble.text.contains(&serde_json::to_string(&args).unwrap()),
        "the exact approved arguments must reach the agent: {}",
        bubble.text
    );
    assert!(
        bubble.text.contains("Do not modify them"),
        "{}",
        bubble.text
    );

    // Journaling the reply is no longer this function's job (issue #469):
    // the runtime journals every continuation reply once, in
    // `CompanyRuntime::publish_continuation`, so that the answers of
    // continuations this arm produces nothing for are not lost either. The
    // round trip — reply journaled into the thread the sign-off was raised
    // in, reaching the console's event stream — is covered end to end over
    // the real router by
    // `server::operator::test::a_continuation_answers_in_the_thread_the_sign_off_was_raised_in`.
    assert!(
        no_replies_journaled(&log).await,
        "the brain must not journal the reply a second time; the runtime owns it"
    );
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

#[tokio::test]
async fn an_explicit_approval_continues_without_reissuing_the_request_tool() {
    assert_explicit_decision_continues(Verdict::Approve, "APPROVED").await;
}

#[tokio::test]
async fn an_explicit_denial_also_returns_to_the_requesting_agent() {
    assert_explicit_decision_continues(Verdict::Deny, "DENIED").await;
}

/// A threaded approval continuation must preserve the approval's thread root
/// when it re-dispatches the granted call. The bound agent's observable
/// context proves the target is threaded rather than channel-only.
#[tokio::test]
async fn an_approved_threaded_grant_redispatches_in_its_origin_thread() {
    let dir = tempfile::tempdir().unwrap();
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    let root = crate::ports::types::EventSeq::new(7);
    requests
        .grants()
        .grant(crate::runtime::grants::GrantedCall {
            approval_id: ApprovalId::new("appr-threaded"),
            agent: "ceo".into(),
            tool: "workspace_write".into(),
            args: serde_json::json!({}),
            at_millis: now_millis(),
            origin_thread: Some("general".into()),
            origin_parent: Some(root),
            origin_task: None,
        });
    let base = brain_with_queue_and_events(
        dir.path(),
        requests,
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf())),
    );
    let pool = Arc::new(HarnessPool::new());
    let brain = HarnessBrain::new(pool.clone(), (*base.deps).clone(), record());
    pool.ensure(&record(), &brain.deps).await.expect("ensure");

    brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-threaded", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let agent = pool
        .agents
        .read()
        .await
        .get(&CompanyId::new("acme"))
        .and_then(|roster| roster.iter().find(|agent| agent.agent_id == "ceo"))
        .cloned()
        .expect("the approved turn keeps the agent resident");
    // Issue #1890 I reverses this. It asserted `None` — that an approval's
    // continuation binds to nothing, because it runs unstreamed and "binding
    // is covered by the delegated target".
    //
    // The delegated target does cover the *drain*, which was already bound
    // by `in_thread(grant.origin_parent())`. It never covered the re-issued
    // call itself: that turn ran against whatever history the agent
    // happened to be holding and then published its answer into the origin
    // thread regardless — grounded in one conversation, answering into
    // another. Identity is no longer inferred from the absent stream, so
    // the turn now binds to the conversation the grant recorded.
    assert_eq!(
        *agent.bound_chat.lock().await,
        Some(("general".to_string(), Some(root))),
        "the re-issued call binds to the conversation the approval was raised in"
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

/// Issue #1846 review (Codex #3869725683) — **the regression.** Same
/// fixture as `an_approved_grant_redispatches_its_agent_with_the_exact_arguments`
/// above, but the re-issued call's provider is now out of credits.
///
/// `run_steered_background` runs through the SAME `run_inner` the
/// interactive chat path does, so it parks a re-issue marker for the
/// granting agent exactly as an ordinary paused message would — proven
/// below by reading it straight off `BudgetPauseSet`. Before this fix, the
/// bubble `redispatch_granted_call` built from that outcome carried
/// `outcome.reply` (the budget-paused placeholder text) verbatim, so the
/// operator saw an ordinary-looking reply rather than the runtime's own
/// pause notice.
///
/// Issue #1846 review (Codex #3870562590): the notice it now carries is the
/// NO-RESEND one. The marker asserted below is real but not redeemable —
/// `run_steered_background` parks it with `background: true`, the one shape
/// `redeem_budget_pause` refuses (`src/server/ops/budget_pause.rs`) — so
/// the redeemable prefix would have drawn a CTA that returned 400 on every
/// click. Both prefixes are asserted: matching the new one is only half the
/// contract, since the console branches on the old one.
#[tokio::test]
async fn a_budget_paused_approval_continuation_surfaces_the_notice_and_parks_a_marker() {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    let args = crate::policy::test_support::composio_args_with(
        crate::policy::test_support::COMPOSIO_SEND_SLUG,
        serde_json::json!({ "to": "a@b.test" }),
    );
    requests
        .grants()
        .grant(crate::runtime::grants::GrantedCall {
            approval_id: ApprovalId::new("appr-1"),
            agent: "ceo".into(),
            tool: "composio_execute".into(),
            args: args.clone(),
            at_millis: now_millis(),
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
        });
    let brain = brain_with_queue_and_events_and_budget_exhausted_provider(
        dir.path(),
        requests,
        log.clone(),
    );

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-1", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    assert_eq!(bubble.channel, "ceo");
    assert!(
        bubble
            .text
            .starts_with(BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX),
        "an approval continuation parks a background marker the redeem route refuses, so \
         its notice must carry the non-redeemable prefix — got: {}",
        bubble.text
    );
    assert!(
        !bubble.text.starts_with(BUDGET_PAUSE_NOTICE_PREFIX),
        "the pre-fix defect: this prefix is what the console keys its \"Add credits & \
         resend\" CTA off, and this marker's redeem returns 400: {}",
        bubble.text
    );
    assert!(
        bubble.text.to_ascii_lowercase().contains("add credits"),
        "the actionable ask survives into the notice: {}",
        bubble.text
    );

    // And a re-issue marker really was parked for the granting agent: the
    // notice is non-redeemable because of HOW it was parked (background),
    // not because nothing was parked at all.
    let marker = crate::runtime::grants::budget_pauses_for(&CompanyId::new("acme"))
        .peek("ceo")
        .expect("run_steered_background parks a marker on the same terms run_inner does");
    assert_eq!(marker.agent, "ceo");
}

/// Issue #374: a resolution that minted only a STANDING grant must still
/// re-dispatch the agent.
///
/// This is the feature's happy path, and it was the one real gap in the
/// plan. `redispatch_granted_call` peeked only the single-use set and
/// no-ops silently on a miss — correct for every legitimate miss (a deny, a
/// native effect, a legacy park) and catastrophic here: the operator picks
/// the broader scope, the permission is armed, and the call they were
/// looking at never runs. It would have looked exactly like #243's original
/// bug, one scope over.
#[tokio::test]
async fn a_standing_grant_also_redispatches_its_agent() {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    requests
        .grants()
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("g1"),
            agent: "ceo".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Approve,
            granted_by: crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::User,
                id: "user-1".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: now_millis(),
            expires_at_millis: now_millis() + 60 * 60 * 1000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });
    let brain = brain_with_queue_and_events(dir.path(), requests, log.clone());

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-1", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    assert_eq!(bubble.channel, "ceo");
    assert_ne!(
        bubble.text, "Acknowledged.",
        "a standing grant must re-dispatch, not fall through to the no-op"
    );
    assert!(bubble.text.contains("workspace_write"), "{}", bubble.text);
    // No exact-arguments pin: a standing grant admits any arguments, which
    // is what the operator consented to by choosing this scope. Telling the
    // model to reproduce a specific argument object would make the broad
    // scope behave like the narrow one.
    assert!(
        !bubble.text.contains("Do not modify them"),
        "a standing grant must not pin arguments: {}",
        bubble.text
    );

    // Journaling the reply belongs to the runtime now (issue #469), so the
    // brain must not write a second copy. See
    // `server::operator::test::a_continuation_answers_in_the_thread_the_sign_off_was_raised_in`
    // for the round trip.
    assert!(no_replies_journaled(&log).await);
}

/// A DENIED approval runs no turn. "No" must never re-dispatch anything.
#[tokio::test]
async fn a_denied_approval_redispatches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    // A grant for a DIFFERENT approval is live, to prove the arm keys on the
    // resolved id rather than reaching for whatever is lying around.
    requests
        .grants()
        .grant(crate::runtime::grants::GrantedCall {
            approval_id: ApprovalId::new("appr-other"),
            agent: "ceo".into(),
            tool: "composio_execute".into(),
            args: serde_json::json!({}),
            at_millis: now_millis(),
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
        });
    requests
        .grants()
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("deny-1"),
            agent: "ceo".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Deny,
            granted_by: crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::User,
                id: "user-1".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: now_millis(),
            expires_at_millis: now_millis() + 60_000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });
    let brain = brain_with_queue_and_events(dir.path(), requests, log.clone());

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-1", Verdict::Deny)]),
            &NoopHost,
        )
        .await
        .unwrap();

    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(
        result.channel_responses[0].text, "Acknowledged.",
        "a deny falls through to the fallback, exactly as before #243"
    );
    assert!(
        log.read_from(&CompanyId::new("acme"), crate::ports::EventSeq::new(0), 100)
            .await
            .unwrap()
            .is_empty(),
        "nothing is journaled for a deny"
    );
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

/// The re-run of a card carrying review feedback reads that feedback: the
/// operator's `[reviewer]` note block is part of the turn instruction the
/// fresh dispatch is built from, which is why `apply_review_feedback`
/// appends to the note *before* re-dispatch.
#[test]
fn task_instruction_carries_a_reviewer_note_block() {
    let mut card = card_in_review("card-1");
    card.note = Some("[reviewer] tighten the intro".to_string());
    let instruction = task_instruction(&card);
    assert!(
        instruction.contains("[reviewer] tighten the intro"),
        "the fresh run must see the reviewer's feedback: {instruction}"
    );
    assert!(instruction.starts_with(&format!("Task: {}", card.title)));
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

/// **The reachability assertion.** A test that the drain works when called
/// is not coverage that the drain is reached — and on this path it was not.
///
/// `redispatch_granted_call` runs a full toolbelt turn and claimed publishes
/// only, so a `review_task` the re-issued call made was staged, answered
/// with "the card has moved to done", and destroyed by the next turn's
/// `clear()`. It **drains** rather than refusing, deliberately: `review_task`
/// is a gateable Write effect, so refusing here would make an operator's own
/// approval unspendable — approve, refuse, re-park.
#[tokio::test]
async fn a_granted_redispatch_drains_the_board_work_its_turn_queued() {
    let dir = tempfile::tempdir().unwrap();
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    requests.grants().grant(granted("appr-1", "review_task"));
    let base_url = spawn_model_script(vec![
        ScriptTurn::Call {
            tool: "review_task",
            args: serde_json::json!({ "task_id": "card-1", "decision": "approve" }),
        },
        ScriptTurn::Say("Approved."),
    ])
    .await;
    let brain = brain_over_script(dir.path(), requests, base_url);
    let tasks = brain.deps.tasks.clone().expect("task store");
    tasks
        .upsert(&CompanyId::new("acme"), &card_in_review("card-1"))
        .await
        .expect("seed the card");

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-1", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");
    assert_eq!(result.channel_responses.len(), 1);

    let cards = tasks.list(&CompanyId::new("acme")).await.expect("list");
    assert_eq!(
        cards[0].column,
        crate::ports::tasks::COLUMN_DONE,
        "the approved card must actually move — staging it and returning was the defect"
    );
    assert_eq!(
        brain.deps.delegations.queued(),
        0,
        "and nothing may be left for a later turn's clear() to destroy"
    );
    assert!(
        !brain.deps.delegations.drain_committed(),
        "the claim releases with the re-dispatch turn"
    );
}

/// The #476 nuance: one continuation cycle can run **several** re-dispatch
/// turns, one per batched resolution. The claim is therefore per turn, not
/// per cycle — each re-dispatch owns its own drain window.
///
/// A per-cycle claim would pass a single-approval test and fail here in the
/// worst way: the second turn's staged verdict would ride on the first
/// turn's already-spent window, or the second acquire would clear work the
/// first had not drained yet.
#[tokio::test]
async fn batched_resolutions_each_get_their_own_drain_window() {
    let dir = tempfile::tempdir().unwrap();
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    requests.grants().grant(granted("appr-1", "review_task"));
    requests.grants().grant(granted("appr-2", "review_task"));
    let base_url = spawn_model_script(vec![
        ScriptTurn::Call {
            tool: "review_task",
            args: serde_json::json!({ "task_id": "card-1", "decision": "approve" }),
        },
        ScriptTurn::Say("Approved card-1."),
        ScriptTurn::Call {
            tool: "review_task",
            args: serde_json::json!({ "task_id": "card-2", "decision": "revise" }),
        },
        ScriptTurn::Say("Sent card-2 back."),
    ])
    .await;
    let brain = brain_over_script(dir.path(), requests, base_url);
    let tasks = brain.deps.tasks.clone().expect("task store");
    let company = CompanyId::new("acme");
    for id in ["card-1", "card-2"] {
        tasks
            .upsert(&company, &card_in_review(id))
            .await
            .expect("seed the card");
    }

    let result = brain
        .run_cycle(
            cycle_over(vec![
                approval_resolved("appr-1", Verdict::Approve),
                approval_resolved("appr-2", Verdict::Approve),
            ]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");
    assert_eq!(
        result.channel_responses.len(),
        2,
        "both resolutions re-dispatch"
    );

    let cards = tasks.list(&company).await.expect("list");
    let column = |id: &str| {
        cards
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.column.clone())
            .unwrap_or_else(|| panic!("{id} is on the board"))
    };
    assert_eq!(
        column("card-1"),
        crate::ports::tasks::COLUMN_DONE,
        "the first re-dispatch's verdict must survive the second re-dispatch's claim"
    );
    assert_eq!(
        column("card-2"),
        crate::ports::tasks::COLUMN_TODO,
        "and the second's own verdict lands too"
    );
    assert_eq!(brain.deps.delegations.queued(), 0);
    assert!(!brain.deps.delegations.drain_committed());
}

/// An approved resolution with NO grant behind it is a silent no-op.
///
/// This is the common case, not an edge: a native effect the runtime already
/// executed, a legacy parked effect from before `Effect::agent` existed (it
/// replays as `None` and mints nothing), a grant already consumed, and a
/// grant already swept all land here. Every one of them must keep the exact
/// pre-#243 behaviour rather than manufacturing a turn.
#[tokio::test]
async fn an_approval_with_no_grant_is_a_silent_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let log: Arc<dyn crate::ports::EventLog> =
        Arc::new(crate::store::FsEventLog::new(dir.path().to_path_buf()));
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    let brain = brain_with_queue_and_events(dir.path(), requests, log.clone());

    let result = brain
        .run_cycle(
            cycle_over(vec![approval_resolved("appr-native", Verdict::Approve)]),
            &NoopHost,
        )
        .await
        .unwrap();

    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(result.channel_responses[0].text, "Acknowledged.");
    assert!(
        log.read_from(&CompanyId::new("acme"), crate::ports::EventSeq::new(0), 100)
            .await
            .unwrap()
            .is_empty()
    );
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

/// **A private aside never crosses a desk boundary in a room's answer.**
///
/// `!aside @peer` is journaled as an ordinary `AgentReply` carrying the
/// pair in `audience`, so it lands inside the span an unconverged room's
/// answer is drawn from and can even be its last row. Carried home it would
/// publish a private exchange to a desk that was never in it — and unlike
/// an aside on one's own desk, where every teammate at least sees that it
/// happened, nobody on the asking desk could see anything to audit.
#[tokio::test]
async fn a_rooms_answer_leaves_its_asides_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let brain = hive_test_brain(dir.path());
    let host = ParkingHost::default();
    // The runner's own turn seam is never reached here — `turns_of` only
    // reads the journal — so any outcome does.
    let runner = hive_desk_runner(
        &brain,
        &host,
        crate::harness::built_in::TurnOutcome {
            reply: String::new(),
            steps: Vec::new(),
            hit_iteration_cap: false,
            abnormal_stop: None,
            halted_for_spend: None,
            budget_paused: None,
        },
    );
    // The crate's in-memory journal: `hive_test_brain` wires none, and this
    // reads a journal rather than running a turn.
    let events: Arc<dyn crate::ports::events::EventLog> =
        Arc::new(crate::hivemind::test::MemoryLog::default());
    let company = crate::hivemind::test::MemoryLog::company();

    // The question the room was convened on: every turn of a referred
    // episode is parented to it.
    let root = events
        .append(
            &company,
            CompanyEvent::OperatorMessage {
                text: "cancellations has put a question to this desk".to_string(),
                by: Some(crate::ports::types::Actor {
                    kind: crate::ports::types::ActorKind::Agent,
                    id: "cancellations".to_string(),
                }),
                chat: Some("returns".to_string()),
                parent: None,
                deliverable: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
            },
        )
        .await
        .expect("journal");
    let row = |agent: &str, text: &str, audience: Vec<String>, parent: Option<EventSeq>| {
        CompanyEvent::AgentReply {
            chat_id: "returns".to_string(),
            agent_id: agent.to_string(),
            text: text.to_string(),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
            parent,
            mentions: Vec::new(),
            mention_depth: 0,
            audience,
        }
    };
    let first = events
        .append(
            &company,
            row(
                "exchanges",
                "no refund tool on this seat",
                Vec::new(),
                Some(root),
            ),
        )
        .await
        .expect("journal");
    // A private aside, inside the span and on the same thread.
    events
        .append(
            &company,
            row(
                "refunds",
                "aside @exchanges — do not tell them we are short-staffed",
                vec!["exchanges".to_string()],
                Some(root),
            ),
        )
        .await
        .expect("journal");
    // **Concurrent traffic on the same desk**, in the same span but on no
    // thread of this crossing — the desk's own other work.
    events
        .append(
            &company,
            row(
                "exchanges",
                "unrelated: the Thursday roster is posted",
                Vec::new(),
                None,
            ),
        )
        .await
        .expect("journal");
    let last = events
        .append(
            &company,
            row("refunds", "i hold the refund tool", Vec::new(), Some(root)),
        )
        .await
        .expect("journal");

    // Spelled out: `EpisodeOutcome` has no `Default`, on purpose — an
    // episode that never ran has no ending to report.
    let outcome = crate::hivemind::EpisodeOutcome {
        ending: crate::hivemind::EpisodeEnding::Idle,
        turns: 2,
        first_seq: Some(first),
        last_seq: Some(last),
        report_seq: None,
        violations: Vec::new(),
        failed_turns: 0,
        referrals: crate::hivemind::ReferralLedger::default(),
    };
    let carried = runner
        .turns_of(&events, &company, &outcome, "returns", root)
        .await
        .expect("the room said something");

    assert!(carried.contains("no refund tool on this seat"));
    assert!(carried.contains("i hold the refund tool"));
    assert!(
        !carried.contains("Thursday roster"),
        "the desk's own other work is not this room's answer: {carried}"
    );
    assert!(
        !carried.contains("short-staffed"),
        "an aside is not a turn and does not answer a crossing: {carried}"
    );
}

/// **A budget-paused hive turn is a hard error, not a folded reply.**
///
/// Before this fix `HiveDeskRunner::speak` returned `Ok(outcome.reply)`
/// unconditionally, so `EpisodeDriver` journaled the host's "add credits"
/// placeholder as a genuine `AgentReply` under the member's own identity —
/// indistinguishable, from the transcript alone, from the member actually
/// answering — and the episode never counted the turn as failed.
#[tokio::test]
async fn hive_speak_turns_a_budget_pause_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(&brain, &host, budget_paused_outcome("theorist"));
    let err = runner
        .speak("theorist", "Settle the derivation.")
        .await
        .expect_err("a budget pause must surface as an error, not Ok(reply)");
    assert!(
        err.to_string().contains("theorist"),
        "the error must name the agent so an operator reading it knows who paused: {err}"
    );
}

/// The same terminal state, for a spend halt rather than a budget pause.
#[tokio::test]
async fn hive_speak_turns_a_spend_halt_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(&brain, &host, spend_halted_outcome("theorist"));
    let err = runner
        .speak("theorist", "Settle the derivation.")
        .await
        .expect_err("a spend halt must surface as an error, not Ok(reply)");
    assert!(
        err.to_string().contains("theorist"),
        "the error must name the agent: {err}"
    );
}

/// **The same bug, on the far-desk referral runner.**
///
/// `HiveReferralRunner::refer` shares the exact same shape:
/// `EpisodeReferrals` only ever treated an `Err` result as "the far desk
/// did not answer", so a budget-paused or spend-halted far turn slipped
/// through as though it were a real referral answer and was carried back
/// to the asking desk as content.
#[tokio::test]
async fn hive_refer_turns_a_budget_pause_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(&brain, &host, budget_paused_outcome("sre"));
    let err = runner
        .refer("platform", "sre", "What is the failover budget?")
        .await
        .expect_err("a budget pause on the far desk must surface as an error too");
    assert!(err.to_string().contains("sre"), "{err}");
}

#[tokio::test]
async fn hive_refer_turns_a_spend_halt_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(&brain, &host, spend_halted_outcome("sre"));
    let err = runner
        .refer("platform", "sre", "What is the failover budget?")
        .await
        .expect_err("a spend halt on the far desk must surface as an error too");
    assert!(err.to_string().contains("sre"), "{err}");
}

/// **A pre-dispatch refusal (`abnormal_stop`) is a hard error too.**
///
/// A fail-closed spend gate that cannot read the meter behind a declared
/// cap refuses dispatch before any model call runs and reports the
/// refusal only through `abnormal_stop` — `budget_paused` and
/// `halted_for_spend` both stay `None`, since no turn ran to pause or
/// halt. Without this, `speak` folded the refusal notice as `Ok(reply)`
/// the same way it once did for a budget pause.
#[tokio::test]
async fn hive_speak_turns_an_abnormal_stop_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(
        &brain,
        &host,
        abnormal_stop_outcome("dispatch refused: spend unreadable"),
    );
    let err = runner
        .speak("theorist", "Settle the derivation.")
        .await
        .expect_err("a pre-dispatch refusal must surface as an error, not Ok(reply)");
    assert!(
        err.to_string().contains("theorist"),
        "the error must name the agent: {err}"
    );
}

/// The same terminal state, on the far-desk referral runner.
#[tokio::test]
async fn hive_refer_turns_an_abnormal_stop_into_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(
        &brain,
        &host,
        abnormal_stop_outcome("dispatch refused: spend unreadable"),
    );
    let err = runner
        .refer("platform", "sre", "What is the failover budget?")
        .await
        .expect_err("a pre-dispatch refusal on the far desk must surface as an error too");
    assert!(err.to_string().contains("sre"), "{err}");
}

/// A turn that finishes cleanly is unaffected: `speak`/`refer` still
/// return `Ok(reply)` when neither terminal flag is set.
#[tokio::test]
async fn hive_speak_and_refer_pass_through_an_ordinary_reply() {
    let ok = |text: &str| crate::harness::built_in::TurnOutcome {
        reply: text.to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let brain = hive_test_brain(dir.path());
    let host = NoopHost;
    let runner = hive_desk_runner(&brain, &host, ok("The rollout is ready to stage."));
    assert_eq!(
        runner
            .speak("theorist", "Settle the derivation.")
            .await
            .expect("an ordinary reply is not an error"),
        "The rollout is ready to stage."
    );
    let runner = hive_desk_runner(&brain, &host, ok("The failover budget is $2,000/month."));
    assert_eq!(
        runner
            .refer("platform", "sre", "What is the failover budget?")
            .await
            .expect("an ordinary referral answer is not an error"),
        "The failover budget is $2,000/month."
    );
}

/// **Regression: an approval request a hive turn queues is parked before
/// the episode ends, not only after it.**
///
/// Before this fix, `HiveDeskRunner::speak` never touched
/// `self.deps.approval_requests` — only the *cycle's* single
/// `park_approval_requests(host)` call, after `driver.run(trigger)` had
/// already returned, ever drained it. A member's `request_approval` call
/// mid-episode therefore sat in the internal queue, invisible to
/// `scripts/hive-euler.py`'s concurrent approval pump, for every
/// remaining turn the room took.
///
/// This drives exactly one `speak` call — the queue is populated by
/// `FixedOutcomeTurn` the same way a real `request_approval` refusal
/// would populate it during a turn — and asserts the request already
/// reached `host.park_effect` immediately after that single turn, with no
/// second turn and no `EpisodeDriver` in the picture. Before the fix this
/// assertion fails: nothing is parked until a cycle-level drain that
/// never runs here.
#[tokio::test]
async fn hive_speak_parks_a_queued_approval_before_the_episode_continues() {
    let dir = tempfile::tempdir().unwrap();
    let requests = crate::harness::policy::ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());
    let host = ParkingHost::default();
    let runner = HiveDeskRunner {
        run_turn: Arc::new(FixedOutcomeTurn {
            outcome: crate::harness::built_in::TurnOutcome {
                reply: "blocked, requires approval".to_string(),
                steps: Vec::new(),
                hit_iteration_cap: false,
                abnormal_stop: None,
                halted_for_spend: None,
                budget_paused: None,
            },
            // Mirrors what a supervised `ApprovalPolicy` records when the
            // agent reaches for a gated tool mid-turn: the request lands
            // on the shared queue, and the turn still completes normally.
            approval_requests: Some(requests.clone()),
        }),
        company: CompanyId::new("acme"),
        chat_id: Some("lab".to_string()),
        thread_root: None,
        trigger_seq: None,
        brain: &brain,
        host: &host,
    };

    let reply =
        crate::hivemind::HiveTurnRunner::speak(&runner, "programmer", "Run the computation.")
            .await
            .expect("a turn that only queued an approval request still replies");
    assert_eq!(reply, "blocked, requires approval");

    // The core regression: parked after this ONE turn, not after a whole
    // episode of turns. `FixedOutcomeTurn` queues
    // `MAX_APPROVAL_REQUESTS_PER_TURN + 1` requests per call (mirroring the
    // existing overflow fixtures), so a single `speak` already fills the
    // per-turn cap.
    let parked = host.parked();
    assert_eq!(
        parked.len(),
        crate::harness::policy::MAX_APPROVAL_REQUESTS_PER_TURN,
        "the gated calls from this single turn must already be on the operator's queue"
    );
    assert_eq!(parked[0].kind, "test_tool_0");
    assert_eq!(
        requests.queued(),
        0,
        "the shared queue is drained by the per-turn park, not left for a later cycle-level drain"
    );
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

/// Cancel mid-flight → the card returns to `todo`, the partial reply is
/// DISCARDED, and only the operator cancellation note lands.
#[tokio::test]
async fn steer_cancel_returns_to_todo_and_discards_partial() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks, _) =
        brain_that_steers_itself(dir.path(), "t1", vec![SteerAction::Cancel]);
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", ""))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let moved = only_card(&tasks).await;
    assert_eq!(moved.column, COLUMN_TODO);
    let note = moved.note.expect("note");
    assert!(note.contains("cancelled while in flight"), "{note:?}");
    // The agent's partial reply must NOT be preserved on a cancel.
    assert!(
        !note.contains("did: "),
        "cancel discards the partial: {note:?}"
    );
}

/// Pause mid-flight → the card parks in the new `paused` column and the
/// partial reply is PRESERVED in the note.
#[tokio::test]
async fn steer_pause_parks_in_paused_and_preserves_partial() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, tasks, _) =
        brain_that_steers_itself(dir.path(), "t1", vec![SteerAction::Pause]);
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", ""))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let moved = only_card(&tasks).await;
    assert_eq!(moved.column, "paused");
    let note = moved.note.expect("note");
    assert!(note.contains("[paused]"), "{note:?}");
    assert!(
        note.contains("did: "),
        "pause preserves the partial: {note:?}"
    );
}

/// Redirect on every turn → the run re-runs in-loop carrying the operator
/// instruction, and the per-dispatch redirect cap (3) finalizes it to
/// `in_review` instead of looping forever.
#[tokio::test]
async fn steer_redirect_reruns_and_the_cap_finalizes_to_in_review() {
    let dir = tempfile::tempdir().unwrap();
    let redirect = || SteerAction::Redirect {
        instruction: "focus on the API".to_string(),
    };
    // Steer a redirect on the first several turns; the cap should stop it.
    let (brain, tasks, provider) = brain_that_steers_itself(
        dir.path(),
        "t1",
        vec![redirect(), redirect(), redirect(), redirect()],
    );
    tasks
        .upsert(&CompanyId::new("acme"), &card("t1", ""))
        .await
        .unwrap();

    brain
        .run_cycle(
            request(vec![CompanyEvent::TaskDispatched {
                task_id: "t1".into(),
                run_id: None,
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let moved = only_card(&tasks).await;
    // Redirect budget exhausted → finalized, not looping.
    assert_eq!(moved.column, "in_review");
    let note = moved.note.expect("note");
    // The operator instruction was carried into the rerun, and the reruns
    // echoed it back through the "Operator redirect:" preamble.
    assert!(note.contains("focus on the API"), "{note:?}");
    assert!(
        note.contains("Operator redirect:"),
        "the rerun carried the operator instruction: {note:?}"
    );
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        4,
        "one initial turn plus three reruns"
    );
}

#[tokio::test]
async fn steer_cancelled_delegation_returns_no_bubble() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _, _) = brain_that_steers_itself(dir.path(), "", vec![SteerAction::Cancel]);

    let result = brain
        .run_delegation(
            Delegation::DelegateToDesk {
                desk: "engineering".to_string(),
                instruction: "investigate".to_string(),
            },
            None,
        )
        .await
        .expect("cancellation is handled");

    assert!(
        result.bubble.is_none() && result.desk_reply.is_none(),
        "cancelled delegation must not bubble or relay"
    );
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

#[test]
fn a_triage_request_is_recognised_as_one() {
    let triage = ModelRequest {
        messages: vec![
            tinyinference::message::Message::system(
                crate::harness::triage::system_prompt_for_test(),
            ),
            tinyinference::message::Message::user("hello".to_string()),
        ],
        ..ModelRequest::default()
    };
    assert!(
        is_triage_request(&triage),
        "the fixture must recognise the real prompt, or it silently starts \
         eating scripted turns again"
    );
    let turn = ModelRequest {
        messages: vec![
            tinyinference::message::Message::system("You are the CEO of Acme.".to_string()),
            tinyinference::message::Message::user("ship it".to_string()),
        ],
        ..ModelRequest::default()
    };
    assert!(
        !is_triage_request(&turn),
        "an agent turn is not a classification"
    );
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

#[test]
fn a_selection_request_is_recognised_as_one() {
    let selection = ModelRequest {
        messages: vec![
            tinyinference::message::Message::system(
                crate::harness::selector::system_prompt_for_test(),
            ),
            tinyinference::message::Message::user("who owns login?".to_string()),
        ],
        ..ModelRequest::default()
    };
    assert!(
        is_selection_request(&selection),
        "the fixture must recognise the real prompt, or it silently starts \
         eating scripted turns"
    );
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

/// Issue #1835, the rung itself: an unmentioned message addressed to an
/// `auto` channel is routed to the selector's pick — a member the
/// deterministic fallback (`engineer`, the first member) would never have
/// chosen — and the pick is clamped to the channel.
#[tokio::test]
async fn an_auto_channel_routes_by_the_selectors_pick() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_selects(dir.path(), "chief");
    assert_eq!(
        brain
            .auto_channel_responder(Some("launch"), "which strategy are we running?")
            .await
            .as_deref(),
        Some("chief"),
        "the selection overrides the first-member fallback"
    );
    assert_eq!(
        provider
            .selector_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

/// The worst case of the new rung is the old rung: a pick outside the
/// channel's membership answers `None`, and the caller keeps the
/// deterministic fallback. Revert the clamp in `SelectorVerdict::parse`
/// and this routes a turn to a teammate the channel does not contain.
#[tokio::test]
async fn a_failed_selection_keeps_the_deterministic_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, _provider) = brain_that_selects(dir.path(), "somebody_else");
    assert_eq!(
        brain
            .auto_channel_responder(Some("launch"), "which strategy are we running?")
            .await,
        None,
        "an out-of-membership pick must fall back, never route"
    );
}

/// Issue #1872 (codex P1): the plan-level total-token ceiling gates the
/// **selection**, not only the responder turn it precedes.
///
/// Selection runs before a responder exists, so `total_ceiling_refusal`
/// has no agent to refuse as and never fired for it — meaning a tenant
/// past its hard ceiling could keep paying to route, one selector call per
/// message, after the ceiling that is supposed to permit no model calls at
/// all. Remove the `total_ceiling_spent` arm in `auto_channel_responder`
/// and this spends a call and answers `chief`.
#[tokio::test]
async fn an_exhausted_total_ceiling_routes_without_paying_for_a_selection() {
    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(SpentMeter);
    let plan = crate::harness::capability_budget::CapabilityPlan {
        period: crate::harness::capability_budget::BudgetPeriod::Daily,
        budgets: Default::default(),
        total_budget: Some(10),
    };
    let (brain, provider) =
        brain_that_selects_with(dir.path(), "chief", Some(plan), Some(meter));
    assert_eq!(
        brain
            .auto_channel_responder(Some("launch"), "which strategy are we running?")
            .await,
        None,
        "past the ceiling the deterministic fallback answers, not a selection"
    );
    assert_eq!(
        provider
            .selector_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a company past its hard ceiling must not pay to route"
    );
}

/// The same ceiling, one pass later (codex on #2055): naming a card is a
/// model call with no agent behind it, exactly like a selection, so
/// `total_ceiling_refusal` never fires for it either.
///
/// Without the gate in `MeteredTitler::title` the provider answers and this
/// returns `Some("chief")` — a tenant past its hard ceiling paying once per
/// card opened, forever.
#[tokio::test]
async fn an_exhausted_total_ceiling_names_a_card_without_paying_for_a_title() {
    use crate::ports::tasks::TitleSummariser;

    let dir = tempfile::tempdir().unwrap();
    let meter = Arc::new(SpentMeter);
    let plan = crate::harness::capability_budget::CapabilityPlan {
        period: crate::harness::capability_budget::BudgetPeriod::Daily,
        budgets: Default::default(),
        total_budget: Some(10),
    };
    let (brain, _provider) =
        brain_that_selects_with(dir.path(), "chief", Some(plan), Some(meter));
    let company = brain.record().id.clone();

    assert_eq!(
        brain
            .title_pass(&company)
            .title("can you fix the checkout bug, it keeps dropping orders")
            .await,
        None,
        "past the ceiling the card is named from the request, not by a model"
    );
}

/// Issue #1872 (codex P2): a channel emptied *after* creation.
///
/// `POST …/desks` refuses an empty auto channel, but `DELETE …/team/{id}`
/// can retire its last roster-backed member later. There is then nobody to
/// pick, so this defers to the caller's ladder — the orchestrator answers,
/// as it does for any desk whose members have all gone — and spends
/// nothing doing it. Refusing the deletion instead would mean a teammate
/// you cannot remove because a channel names them.
#[tokio::test]
async fn a_channel_emptied_by_deletion_falls_back_without_paying() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_selects(dir.path(), "chief");
    brain.mutate_record(|r| {
        r.overlay_retired_agents = vec!["engineer".to_string(), "chief".to_string()];
    });
    assert_eq!(
        brain
            .auto_channel_responder(Some("launch"), "who owns the retry logic?")
            .await,
        None,
        "no candidates left: fall back rather than route to a retired teammate"
    );
    assert_eq!(
        provider
            .selector_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
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

/// The short-circuits spend nothing: a lead desk never reaches the
/// selector at all, and a single-member channel is its member without a
/// model call — a pick over one candidate is the fallback with latency.
#[tokio::test]
async fn lead_desks_and_single_member_channels_never_pay_for_selection() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_selects(dir.path(), "chief");
    // The lead desk from `record_with_desk` is not an auto channel.
    assert_eq!(
        brain
            .auto_channel_responder(Some("eng_desk"), "hello")
            .await,
        None
    );
    // Shrink the channel to one member: it answers without the model.
    brain.mutate_record(|r| {
        r.overlay_desks[0].members = vec!["chief".to_string()];
    });
    assert_eq!(
        brain
            .auto_channel_responder(Some("launch"), "hello")
            .await
            .as_deref(),
        Some("chief")
    );
    assert_eq!(
        provider
            .selector_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "neither path may spend a selection call"
    );
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

/// (a) After a `delegate_to_desk`, the operator-facing reply is a SECOND
/// orchestrator turn that relays the teammate's answer — one coherent
/// bubble, not a disconnected sibling.
#[tokio::test]
async fn delegate_to_desk_relays_the_answer_in_a_second_orchestrator_turn() {
    let dir = tempfile::tempdir().unwrap();
    // Invoke 1 (orchestrator) queues a delegate_to_desk; invoke 2 is the desk
    // lead's turn; invoke 3 is the relay turn (queues nothing).
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::DelegateToDesk {
            desk: "eng_desk".to_string(),
            instruction: "diagnose the outage".to_string(),
        })],
    );

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "why is the site down?".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    // The operator sees ONE bubble — the CEO's relay, not a separate teammate
    // sibling bubble.
    assert_eq!(result.channel_responses.len(), 1);
    let bubble = &result.channel_responses[0];
    assert_eq!(bubble.channel, "operator");
    // Three turns ran: orchestrator → desk lead → exactly one relay turn.
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "orchestrator, desk lead, then exactly one relay turn"
    );
    // The relayed bubble carries the teammate's answer (the desk lead echoed
    // its instruction, and the relay prompt embeds that reply under an
    // `engineer replied:` frame) — proving the operator reply is the SECOND
    // turn relaying the teammate, not the pre-delegation first reply.
    assert!(
        bubble.text.contains("engineer replied:")
            && bubble.text.contains("diagnose the outage"),
        "the relay carries the teammate's answer: {:?}",
        bubble.text
    );
    // …and it is the relay turn, whose prompt framed the hand-back.
    assert!(
        bubble.text.contains("Relay their answer"),
        "the operator bubble is the relay turn: {:?}",
        bubble.text
    );
}

/// (b) The relay turn cannot re-delegate: a delegation it queues is
/// discarded, so no further desk turn or relay runs (cost stays bounded to
/// one extra turn).
#[tokio::test]
async fn the_relay_turn_cannot_re_delegate() {
    let dir = tempfile::tempdir().unwrap();
    // Invoke 1 queues a delegation; invoke 3 (the relay) ALSO tries to queue
    // one — which must be discarded, so no fourth/fifth turn runs.
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![
            Some(Delegation::DelegateToDesk {
                desk: "eng_desk".to_string(),
                instruction: "first".to_string(),
            }),
            None, // the desk lead's turn queues nothing
            Some(Delegation::DelegateToDesk {
                desk: "eng_desk".to_string(),
                instruction: "second".to_string(),
            }),
        ],
    );

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "handle it".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    // Exactly three turns: orchestrator, desk lead, relay. The relay's queued
    // delegation was dropped — no fourth (desk-lead) or fifth (relay) turn.
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "the relay turn's delegation is discarded — one extra turn, no loop"
    );
    // The discard actually emptied the queue (not left dirty for next cycle).
    assert_eq!(
        brain.deps.delegations.queued(),
        0,
        "the relay turn's queued delegation was discarded"
    );
    // Still exactly one operator bubble.
    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(result.channel_responses[0].channel, "operator");
}

/// (c) A normal, non-delegating message still produces exactly one turn — the
/// relay path is entered only when a `delegate_to_desk` actually answered.
#[tokio::test]
async fn a_non_delegating_message_runs_exactly_one_turn() {
    let dir = tempfile::tempdir().unwrap();
    // No scripted delegations → the orchestrator answers directly.
    let (brain, provider) = brain_that_delegates(dir.path(), Vec::new());

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "status?".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: Vec::new(),
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "no delegation → a single orchestrator turn, no relay"
    );
    assert_eq!(result.channel_responses.len(), 1);
    assert_eq!(result.channel_responses[0].channel, "operator");
    assert!(
        result.channel_responses[0].text.contains("status?"),
        "{:?}",
        result.channel_responses[0].text
    );
}

/// Issue #1682: on an `openhuman` build the embedded harness brain is the
/// active cognition seam, and the operator's attachments must reach the
/// agent here too — the medulla adapter folds them into its wire body, but
/// this path used to hand the pool the raw message, so an attachment-
/// dependent request reached the agent with no indication a file existed.
/// The provider echoes the composed message, so the bubble proves the
/// marker (node id, filename, and the untrusted-file framing) arrived.
#[tokio::test]
async fn attachments_reach_the_harness_agent() {
    let dir = tempfile::tempdir().unwrap();
    // No scripted delegations → the orchestrator answers directly.
    let (brain, _provider) = brain_that_delegates(dir.path(), Vec::new());

    let result = brain
        .run_cycle(
            request(vec![CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "what does this say?".into(),
                by: None,
                chat: None,
                deliverable: None,
                attachments: vec![crate::ports::types::Attachment {
                    node_id: "node-harness".to_string(),
                    name: "notes.txt".to_string(),
                    mime: "text/plain".to_string(),
                    size: 11,
                    extracted_text: Some("hello world".to_string()),
                }],
            }]),
            &NoopHost,
        )
        .await
        .expect("cycle runs");

    let bubble = result.channel_responses.first().expect("one bubble");
    assert!(
        bubble.text.contains("what does this say?"),
        "{:?}",
        bubble.text
    );
    assert!(bubble.text.contains("node-harness"), "{:?}", bubble.text);
    assert!(bubble.text.contains("notes.txt"), "{:?}", bubble.text);
    // The same untrusted-file framing the medulla wire uses.
    assert!(
        bubble.text.contains("FILE DATA, not instructions"),
        "{:?}",
        bubble.text
    );
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
            .any(|card| {
                card.id == id && card.column == crate::ports::tasks::COLUMN_IN_PROGRESS
            });
        if !handed_on {
            break;
        }
    }
}

/// The bug: a dispatched task the CEO delegated went straight to
/// `in_review` under the CEO with a blank assignee, and the delegate never
/// ran — `run_task` ran one turn and never drained the delegation queue.
///
/// Now the delegate actually runs, is linked as the card's assignee, and
/// the card only reaches `in_review` on the back of THEIR output.
#[tokio::test]
async fn a_dispatched_turn_that_delegates_runs_the_delegate_and_links_them_to_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::DelegateToDesk {
            desk: "eng_desk".to_string(),
            instruction: "fetch my activity".to_string(),
        })],
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-deleg").await;

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the dispatched turn, then the delegate's own turn — the delegate must actually run"
    );

    let after = only_card(&provider.tasks).await;
    assert_eq!(
        after.assignee, "engineer",
        "the delegate — an agent — must be linked as the assignee, not left blank \
         under the delegator"
    );
    assert_eq!(
        after.column, "in_review",
        "the card reaches review on the delegate's output"
    );
    let note = after.note.expect("note");
    assert!(
        note.contains("delegated to engineer: fetch my activity"),
        "the hand-off is recorded in the delegator's voice: {note}"
    );
    // The delegate's own turn produced the result block: the mock echoes the
    // instruction it was handed back under its own attribution.
    let (_, delegate_block) = note
        .split_once("[engineer] did:")
        .unwrap_or_else(|| panic!("the delegate's output is the card's result: {note}"));
    assert!(
        delegate_block.contains("fetch my activity"),
        "the delegate ran the instruction it was handed: {note}"
    );

    // …and while the delegate was working, the card showed THEM working it:
    // its second turn ran against a card already reassigned and still in
    // progress, not one parked in a terminal column.
    // Owner and worker are the same agent: the desk's lead. The board shows
    // a teammate working it, never a channel id.
    assert_eq!(
        provider.board()[1],
        ("in_progress".to_string(), "engineer".to_string()),
        "the delegate must be shown working the card while they work it"
    );
}

/// An errored hand-off must not STRAND the card (issue #213 review).
///
/// `hand_card_over` has already persisted the card as `in_progress`
/// reassigned to the delegate before their turn starts. If that turn then
/// errors and the failure is propagated out of `run_task`, the settle and
/// the final `upsert` are both skipped and the card sits in `in_progress`
/// under the delegate with no result — and nothing re-dispatches it, because
/// `task_enters_in_progress` only edge-fires on the *transition* into that
/// column and the card is already there. Exactly the state issue #204 exists
/// to eliminate, reintroduced through the error path.
///
/// So an errored hand-off takes the same arm an errored turn does: settle
/// `Failed` → `todo`, with the reason on the note.
#[tokio::test]
async fn a_hand_off_whose_delegate_errors_lands_in_todo_not_stranded_in_progress() {
    let dir = tempfile::tempdir().unwrap();
    // Invoke 1 is the dispatched orchestrator turn (queues the hand-off);
    // invoke 2 is the delegate's own turn, which errors.
    let (brain, provider) = brain_that_delegates_with(
        dir.path(),
        vec![vec![Delegation::DelegateToDesk {
            desk: "eng_desk".to_string(),
            instruction: "fetch my activity".to_string(),
        }]],
        TurnFaults {
            fail_from: Some(2),
            ..TurnFaults::default()
        },
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-boom").await;

    let after = only_card(&provider.tasks).await;
    assert_eq!(
        after.column, COLUMN_TODO,
        "an errored hand-off must return the card to To-do, never leave it stranded in \
         progress where nothing will re-dispatch it"
    );
    let note = after.note.expect("note");
    // "dispatch failed", not "hand-off failed": since the async hand-off the
    // delegate errors inside its OWN attempt, so the reason is recorded by
    // that attempt's settle rather than by the hand-off that queued it. The
    // property this test is named for — To-do, never stranded in progress —
    // is asserted above and is unchanged.
    assert!(
        note.contains("dispatch failed:"),
        "the failure reason lands on the note: {note}"
    );
    assert!(
        note.contains("delegated to engineer: fetch my activity"),
        "the hand-off that was attempted is still recorded: {note}"
    );
    // The failure is the assignee's, not the operator's — a cancellation is
    // the only ending attributed to `operator`.
    assert!(
        !note.contains("[operator]"),
        "an errored run is not an operator cancellation: {note}"
    );
}

/// A hand-off an operator cancels mid-flight produced nothing, so the card
/// returns to `todo` reported as the cancellation it actually was.
///
/// The claim is only made because `run_delegation` reports the cancellation
/// as a fact (`DelegationOutcome::cancelled`); a hand-off that ends empty
/// for any other reason no longer reaches this arm at all (issue #213
/// review finding 2).
#[tokio::test]
async fn a_cancelled_hand_off_returns_the_card_to_todo_as_a_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    // Invoke 1 is the dispatched orchestrator turn (queues the hand-off);
    // invoke 2 is the delegate's turn, which cancels itself mid-run.
    let (brain, provider) = brain_that_delegates_with(
        dir.path(),
        vec![vec![Delegation::DelegateToDesk {
            desk: "eng_desk".to_string(),
            instruction: "fetch my activity".to_string(),
        }]],
        TurnFaults {
            cancel_on: vec![2],
            ..TurnFaults::default()
        },
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-cancel").await;

    let after = only_card(&provider.tasks).await;
    assert_eq!(
        after.column, COLUMN_TODO,
        "a cancelled hand-off must not read as finished, and must not strand in progress"
    );
    let note = after.note.expect("note");
    // The wording moved with the async hand-off. A cancelled delegate is now
    // cancelled inside ITS OWN dispatch, so the reason is `run_task`'s
    // steer-cancel wording rather than the sync hand-off's report of a
    // delegate that never produced anything. The property this test is named
    // for — To-do, attributed to the operator — is unchanged.
    assert!(
        note.contains("cancelled while in flight"),
        "the cancellation is reported as the cause: {note}"
    );
    // A cancellation is the operator's act, so the block is theirs — not the
    // delegate's, who never said it.
    assert!(
        note.contains("[operator] cancelled while in flight"),
        "a cancellation is attributed to the operator: {note}"
    );
}

/// A later hand-off that ANSWERS must not be discarded by an earlier one
/// that produced nothing (issue #213 review finding 3).
///
/// Before this, the first hand-off owned the card unconditionally. A first
/// hand-off cancelled mid-flight still produced a `TaskHandoff`, so a second
/// hand-off in the same drain took the "does not own the card" arm: its
/// answer was appended to the note, but the card still settled `Cancelled`
/// -> `todo` off the first. Work that ran and produced output ended up
/// filed under a card marked cancelled.
#[tokio::test]
async fn a_later_answering_hand_off_takes_the_card_over_from_an_earlier_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    // Invoke 1 is the dispatched orchestrator turn, queuing BOTH hand-offs.
    // Invoke 2 is the first delegate's run, cancelled mid-flight; invoke 3
    // is the second delegate's run, which answers.
    let (brain, provider) = brain_that_delegates_with(
        dir.path(),
        vec![vec![
            Delegation::DelegateToDesk {
                desk: "eng_desk".to_string(),
                instruction: "first attempt".to_string(),
            },
            Delegation::DelegateToDesk {
                desk: "eng_desk".to_string(),
                instruction: "second attempt".to_string(),
            },
        ]],
        TurnFaults {
            cancel_on: vec![2],
            ..TurnFaults::default()
        },
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-two-handoffs").await;

    let after = only_card(&provider.tasks).await;
    // The card settles from the ONE hand-off that owns it — cancelled here,
    // so To-do. A second hand-off in the same turn never ran.
    assert_eq!(
        after.column, COLUMN_TODO,
        "the card settles from the hand-off that owns it"
    );
    assert_eq!(
        after.assignee, "engineer",
        "the owner is an agent: the lead of the desk the card was handed to"
    );
    let note = after.note.expect("note");
    // The protection #213 added, reached a stronger way. It used to be
    // "a later hand-off that ANSWERS takes the card from an empty one", so
    // work that ran could never be filed under a cancelled card. Async
    // hand-off removes the race instead of resolving it: the first hand-off
    // owns the card and its delegate is dispatched, so a second one is
    // recorded and NOT started. There is no output to misfile because no
    // second run happened.
    assert!(
        note.contains("also asked eng_desk: second attempt") && note.contains("not started"),
        "the second hand-off is recorded, and visibly did not run: {note}"
    );
    assert!(
        !note.contains("[eng_desk] second attempt")
            && !note.contains("[engineer] second attempt"),
        "a second hand-off must not produce work under a card owned by the first: {note}"
    );
}

/// The compatibility half: a dispatched turn that delegates nothing still
/// runs exactly one turn and settles under the agent that ran it.
///
/// (Deliberately no assertion on `assignee` — linking the *non-delegating*
/// working agent to the card is issue #205's fix, and this test must not
/// pin the blank it leaves behind today.)
#[tokio::test]
async fn a_dispatched_turn_that_delegates_nothing_settles_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(dir.path(), Vec::new());
    dispatch_card(&brain, &provider.tasks.clone(), "t-plain").await;

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "no delegation → one turn, no hand-off"
    );
    let after = only_card(&provider.tasks).await;
    assert_eq!(after.column, "in_review");
    assert!(
        after.note.expect("note").contains("[chief]"),
        "the agent that ran it owns the result"
    );
}

/// A hand-off to a desk nobody leads has nowhere to go: rather than
/// stranding the card in `in_progress` waiting on a delegate that will
/// never run, the delegator's own reply settles it exactly as before.
#[tokio::test]
async fn a_hand_off_to_an_unknown_desk_settles_under_the_delegator() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::DelegateToDesk {
            desk: "ghost".to_string(),
            instruction: "look into it".to_string(),
        })],
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-ghost").await;

    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "an unknown desk has no lead to run a second turn"
    );
    let after = only_card(&provider.tasks).await;
    assert_eq!(
        after.column, "in_review",
        "the card must not strand in progress waiting on a delegate that cannot run"
    );
    assert_ne!(after.assignee, "ghost");
}

/// Issue #272: settling under the delegator is the right *behaviour* (#213
/// chose it so a card is never stranded), but it used to be silent — the
/// board showed a card whose note claimed a hand-off and whose owner was the
/// delegator, with nothing connecting the two. The undeliverable hand-off is
/// now recorded on the card, so an operator reads the fact instead of
/// inferring it from an absence.
#[tokio::test]
async fn a_hand_off_that_cannot_be_delivered_says_so_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::DelegateToDesk {
            desk: "ghost".to_string(),
            instruction: "look into it".to_string(),
        })],
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-loud").await;

    let note = only_card(&provider.tasks).await.note.expect("note");
    assert!(
        note.contains("hand-off to \"ghost\" was not delivered"),
        "the card must name the hand-off that did not happen: {note}"
    );
    assert!(
        note.contains("this card is still with chief"),
        "the card must say who still owns it: {note}"
    );
}

/// Issue #272, the grounded half: the tool refused the invented target, so
/// no `Delegation` was ever queued. The turn is still free to *say* it
/// handed the work off — that is exactly what happened on the live company
/// — so the board records the refusal independently of the turn's account
/// of it. Without this the card settles under the delegator with a note
/// that claims a hand-off and nothing anywhere contradicting it.
#[tokio::test]
async fn a_refused_hand_off_is_recorded_on_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates_with(
        dir.path(),
        vec![Vec::new()],
        TurnFaults {
            refused_on_first: vec!["writer".to_string()],
            ..TurnFaults::default()
        },
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-refused").await;

    let after = only_card(&provider.tasks).await;
    assert_eq!(
        provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a refused hand-off runs no delegate"
    );
    assert_eq!(
        after.column, "in_review",
        "the card still settles under the delegator (#213); it is only no longer silent"
    );
    let note = after.note.expect("note");
    assert!(
        note.contains("hand-off to \"writer\" was not delivered"),
        "the refused target must be named on the card: {note}"
    );
    assert!(
        note.contains("not somewhere this company can hand work to"),
        "the cause must be on the card: {note}"
    );
}

/// The other half of #272's note: a delegation that never had a desk target
/// (a `spawn_task`) must not pick up an undeliverable-hand-off line.
#[tokio::test]
async fn a_spawn_task_never_records_an_undeliverable_hand_off() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::SpawnTask {
            title: "Follow up".to_string(),
            note: None,
            assignee: None,
        })],
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-quiet").await;

    let cards = provider.tasks.list(&CompanyId::new("acme")).await.unwrap();
    let parent = cards.iter().find(|c| c.id == "t-quiet").expect("parent");
    assert!(
        !parent
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("was not delivered"),
        "a spawn_task has no desk target to fail: {:?}",
        parent.note
    );
}

/// A `spawn_task` queued by a *dispatched* turn now opens its card with the
/// dispatched card as its parent — the lineage `run_delegation` could never
/// stamp while the task path did not drain the queue (issue #185's
/// `parent_task_id`).
#[tokio::test]
async fn a_task_spawned_by_a_dispatched_turn_records_its_parent_card() {
    let dir = tempfile::tempdir().unwrap();
    let (brain, provider) = brain_that_delegates(
        dir.path(),
        vec![Some(Delegation::SpawnTask {
            title: "Follow up next week".to_string(),
            note: None,
            assignee: None,
        })],
    );
    dispatch_card(&brain, &provider.tasks.clone(), "t-parent").await;

    let cards = provider.tasks.list(&CompanyId::new("acme")).await.unwrap();
    let spawned = cards
        .iter()
        .find(|c| c.title == "Follow up next week")
        .expect("the spawned card must actually be opened");
    assert_eq!(spawned.column, COLUMN_TODO);
    assert_eq!(
        spawned.parent_task_id.as_deref(),
        Some("t-parent"),
        "a card spawned inside a dispatch remembers the card it came from"
    );
    // The parent still settles on its own turn's reply — spawning follow-up
    // work is not a hand-off.
    let parent = cards.iter().find(|c| c.id == "t-parent").expect("parent");
    assert_eq!(parent.column, "in_review");
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

/// The wiring's whole point: an agent bound to a named harness runs on that
/// harness's engine, and an unbound one stays on the default.
#[tokio::test]
async fn a_bound_agent_runs_on_its_own_lane() {
    let dir = tempfile::tempdir().unwrap();
    let deep = Arc::new(SpyLane {
        label: "deep".to_string(),
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let brain = brain_over_mock_with(dir.path(), two_harness_record()).with_lanes(vec![(
        "deep".to_string(),
        deep.clone() as Arc<dyn crate::runtime::delegation::RunTurn>,
    )]);

    let company = CompanyId::new("acme");
    let out = brain
        .run_turn()
        .run(
            &company,
            "researcher",
            "hi",
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await
        .expect("routes to the deep lane");
    assert_eq!(out.reply, "deep");
    assert_eq!(&*deep.seen.lock().unwrap(), &["researcher".to_string()]);

    // The unbound agent must not reach it — it belongs to the default pool.
    let _ = brain
        .run_turn()
        .run(
            &company,
            "ceo",
            "hi",
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await;
    assert_eq!(
        &*deep.seen.lock().unwrap(),
        &["researcher".to_string()],
        "the default agent must not land on the named lane"
    );
}

/// The named lane is a **real** pool, not a spy: it must build its own
/// roster at boot, or a bound agent's first turn fails `CompanyNotFound`
/// on an empty pool. This is the path the `SpyLane` coverage above cannot
/// reach — a spy forwards every turn, so it would pass whether or not the
/// lane's pool was ever warmed.
#[tokio::test]
async fn a_named_lane_builds_its_roster_at_boot() {
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock_with(dir.path(), two_harness_record());
    // The lane `lanes::build` produces: its own pool over deps narrowed to
    // the agents it serves.
    let mut deep_deps = (*brain.deps).clone();
    deep_deps.serves = Some(std::collections::HashSet::from(["researcher".to_string()]));
    let deep: Arc<dyn crate::runtime::delegation::RunTurn> = Arc::new(HarnessRunTurn::new(
        Arc::new(HarnessPool::new()),
        Arc::new(deep_deps),
    ));
    let brain = brain.with_lanes(vec![("deep".to_string(), deep.clone())]);

    // Boot warm-up: the router warms every lane's engine, each against its
    // own narrowed deps.
    brain
        .run_turn()
        .ensure(&brain.record())
        .await
        .expect("every lane's roster builds");

    // A bound agent's turn now reaches its lane's engine instead of dying
    // with "company not found".
    let out = brain
        .run_turn()
        .run(
            &CompanyId::new("acme"),
            "researcher",
            "hi",
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await
        .expect("the deep lane's roster is built at boot");
    assert!(out.reply.contains("hi"), "{}", out.reply);
}

/// A declared harness this host cannot run fails the turn, naming the
/// harness and the reason. It must never quietly borrow the default lane:
/// that turn would succeed on a model and a credential nobody chose, and
/// the only evidence would be a billing line.
#[tokio::test]
async fn an_unrunnable_harness_fails_rather_than_falling_back() {
    let dir = tempfile::tempdir().unwrap();
    let brain =
        brain_over_mock_with(dir.path(), two_harness_record()).with_unavailable_lanes(vec![(
            "deep".to_string(),
            "this build has no ACP transport wired".to_string(),
        )]);

    let err = brain
        .run_turn()
        .run(
            &CompanyId::new("acme"),
            "researcher",
            "hi",
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await
        .expect_err("must not fall back to the default lane");
    let msg = err.to_string();
    assert!(msg.contains("researcher"), "{msg}");
    assert!(msg.contains("deep"), "{msg}");
    assert!(msg.contains("ACP transport"), "names the fix: {msg}");
}

/// A company declaring no `[[harness]]` keeps exactly the single-lane path:
/// no lanes, no bindings, nothing to consult.
#[tokio::test]
async fn a_company_with_no_harness_block_is_unrouted() {
    let dir = tempfile::tempdir().unwrap();
    let brain = brain_over_mock(dir.path());
    assert!(brain.lanes.is_empty());
    assert!(brain.unavailable.is_empty());
    assert!(brain.bindings.is_empty());
    assert_eq!(brain.default_harness, "default");
}

/// Issue #1244: a company whose *only* declared harness is `kind = "acp"`
/// must not silently run turns on the embedded engine.
///
/// Before the fix, `lanes::build`'s early return for a single declared
/// harness meant nobody ever asked what *kind* that lone harness was — the
/// caller (here, and identically in `RuntimeBuilder`) unconditionally built
/// a `HarnessRunTurn` from the shared pool regardless. This exercises the
/// real `lanes::build` output rather than a hand-simulated one, so a
/// regression in either `lanes::build` or `HarnessBrain::run_turn` fails it.
#[tokio::test]
async fn a_lone_acp_default_harness_does_not_silently_run_embedded() {
    let dir = tempfile::tempdir().unwrap();
    let manifest: CompanyManifest = toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "ceo"
role = "Chief Executive"

[[harness]]
id = "laptop"
kind = "acp"
default = true

[harness.acp]
transport = "local"
agent = "claude"
"#,
    )
    .expect("valid manifest");
    let record = CompanyRecord {
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
    };

    let brain = brain_over_mock_with(dir.path(), record);
    let secrets: Arc<dyn crate::ports::SecretStore> =
        Arc::new(crate::store::FsSecretStore::new(dir.path()));
    let lanes = crate::harness::lanes::build(
        &brain.record(),
        Arc::new(HarnessPool::new()),
        &brain.deps,
        secrets,
        None,
        None,
    );

    assert!(
        lanes.default_engine.is_none(),
        "an acp default harness has no built-in engine to fall back to"
    );
    assert!(
        lanes.unavailable.iter().any(|(id, _)| id == "laptop"),
        "the default harness's own id must carry the unavailable reason: {:?}",
        lanes.unavailable
    );

    // Wire the brain exactly as `RuntimeBuilder` would, and confirm the
    // turn actually fails instead of quietly answering from the embedded
    // `MockProvider`.
    let brain = brain
        .with_lanes(lanes.lanes)
        .with_unavailable_lanes(lanes.unavailable)
        .with_default_engine(lanes.default_engine);

    let err = brain
        .run_turn()
        .run(
            &CompanyId::new("acme"),
            "ceo",
            "hi",
            crate::runtime::delegation::ChatTarget::default(),
        )
        .await
        .expect_err("must not fall back to the embedded engine");
    let msg = err.to_string();
    assert!(msg.contains("ceo"), "{msg}");
    assert!(msg.contains("laptop"), "{msg}");
    assert!(msg.contains("ACP transport"), "names the fix: {msg}");
}

/// Issue #966: a workflow-copilot reply is authored by the copilot.
///
/// This is the assertion the #885 fix was missing on this branch. The bubble
/// is emitted on the **operator** channel, and before this the author field
/// was left `None` — so the journal writer's `channel` fallback stamped
/// `agent_id: "operator"` on a reply an agent had genuinely produced.
///
/// Asserts the author is *not* the channel, rather than only that it equals
/// the constant: the defect's whole shape is the two being conflated, and a
/// test that checked equality alone would still pass if `CONFINED_AGENT_ID`
/// were ever redefined to `"operator"`.
#[test]
fn a_copilot_turn_is_authored_by_the_copilot_not_the_operator_channel() {
    let bubble = confined_bubble(crate::harness::TurnOutcome {
        reply: "here is what that node does".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        // Test fixture, not the ACP fold (PR #1880 review).
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: None,
    });
    assert_eq!(bubble.channel, "operator", "the destination is unchanged");
    assert_eq!(
        bubble.agent.as_deref(),
        Some(crate::ports::CONFINED_AGENT_ID)
    );
    assert_ne!(
        bubble.agent.as_deref(),
        Some(bubble.channel.as_str()),
        "author and destination must not be the same value — that conflation is issue #885"
    );
}

/// Issue #1846 review (Codex #3869277640) — **the regression.** A budget
/// pause from a confined workflow-copilot turn must NOT read as the
/// copilot's own answer.
///
/// Before this fix, `confined_bubble` folded `outcome.reply` — the
/// budget-paused placeholder text, per `classify_turn`'s
/// `AttemptOutcome::BudgetPaused` handling — straight into an ordinary
/// bubble authored by `CONFINED_AGENT_ID`, exactly the #885/#966
/// author-vs-channel conflation this file exists to prevent, just for a
/// pause instead of an authored reply. `confined_turn_bubble` is the
/// fixed boundary: this asserts it routes a paused outcome to
/// `system_notice` (unauthored, `SYSTEM_AUTHOR`) instead.
#[test]
fn a_confined_turns_budget_pause_is_a_system_notice_not_a_copilot_reply() {
    let outcome = crate::harness::TurnOutcome {
        // What `classify_turn`'s `AttemptOutcome::BudgetPaused` arm
        // actually leaves in `reply` — irrelevant to the notice text,
        // which is built fresh from `budget_paused` below, but present
        // here so this fixture matches what `run_confined` really
        // returns rather than an idealised one.
        reply: "Paused — copilot's turn ran out of inference budget/credits.".to_string(),
        steps: Vec::new(),
        hit_iteration_cap: false,
        abnormal_stop: None,
        halted_for_spend: None,
        budget_paused: Some(crate::harness::BudgetPause {
            agent: confine::CONFINED_AGENT_ID.to_string(),
            summary: "Add credits to your account, then resend your message.".to_string(),
        }),
    };

    let bubble = confined_turn_bubble(outcome);

    assert_eq!(
        bubble.agent.as_deref(),
        Some(crate::ports::SYSTEM_AUTHOR),
        "a budget pause is never something the copilot said — it must be unauthored, not \
         attributed to CONFINED_AGENT_ID like an ordinary reply: {:?}",
        bubble.agent
    );
    assert_ne!(
        bubble.agent.as_deref(),
        Some(confine::CONFINED_AGENT_ID),
        "the pre-fix defect: falling through to confined_bubble would attribute the pause \
         notice to the copilot itself"
    );
    // Issue #1846 review (Codex #3870562586): the NO-RESEND prefix. This
    // assertion used to require `BUDGET_PAUSE_NOTICE_PREFIX`, which is
    // precisely what the console keys its "Add credits & resend" button
    // off — and `run_confined` never parks a marker, so that button could
    // only ever 404. Asserting the negative too: the whole defect is the
    // two prefixes being conflated, and a test that only checked the new
    // one would still pass if the redeemable prefix were ever made a
    // prefix of it.
    assert!(
        bubble
            .text
            .starts_with(BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX),
        "a confined pause parks no marker, so it must carry the non-redeemable prefix: {}",
        bubble.text
    );
    assert!(
        !bubble.text.starts_with(BUDGET_PAUSE_NOTICE_PREFIX),
        "the pre-fix defect: this prefix renders an Add-Credits CTA whose GET returns null \
         and whose POST 404s, because CONFINED_AGENT_ID never has a marker: {}",
        bubble.text
    );
    assert!(
        bubble.text.to_ascii_lowercase().contains("add credits"),
        "the actionable ask survives into the notice: {}",
        bubble.text
    );
}

/// Issue #1846 review (Codex #3870562590) — **the regression.** An approval
/// continuation that pauses for credits must not advertise a redeem the
/// server refuses.
///
/// The continuation runs through `run_steered_background`, so `run_inner`
/// parks its marker with `background: true`, and `redeem_budget_pause`
/// rejects exactly that shape with a 400 (`src/server/ops/budget_pause.rs`).
/// Emitting `BUDGET_PAUSE_NOTICE_PREFIX` therefore put a button on screen
/// that reserved the marker, restored it, and failed — every single click.
///
/// Issue #1906: this pins the notice BUILDER only, and its name now says
/// so. It calls `budget_pause_notice_no_resend` directly and asserts the
/// result starts with the constant that function formats with — a
/// tautology over `format!`. Revert the continuation arm at
/// `run_steered_background`'s tail to `budget_pause_notice` and this test
/// still passes, so the name it used to carry — "an approval continuation
/// pause offers no redeem CTA" — promised coverage it does not provide.
/// That coverage is real and lives in
/// `a_budget_paused_approval_continuation_surfaces_the_notice_and_parks_a_marker`,
/// which drives the continuation and reads the bubble it emits. Kept under
/// the honest name anyway: it is the cheap guard on the builder itself,
/// which is what the console branches on.
#[test]
fn the_no_resend_notice_builder_uses_the_non_redeemable_prefix() {
    let pause = crate::harness::BudgetPause {
        agent: "maya".to_string(),
        summary: "Add credits to your account, then start this again.".to_string(),
    };

    let notice = budget_pause_notice_no_resend(&pause);

    assert!(
        notice.starts_with(BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX),
        "{notice}"
    );
    assert!(
        !notice.starts_with(BUDGET_PAUSE_NOTICE_PREFIX),
        "the pre-fix defect: a background-parked marker is refused by the redeem route, so \
         this prefix's CTA can only ever return 400: {notice}"
    );
    assert!(
        notice.to_ascii_lowercase().contains("add credits"),
        "the operator still has to be told the lever: {notice}"
    );
    assert!(
        notice.contains(&pause.summary),
        "the provider's own summary survives into the notice: {notice}"
    );
}

/// The two prefixes must stay genuinely distinct: the console decides
/// whether to render an actionable button by `startsWith`, so if the
/// redeemable prefix were ever edited to become a prefix of the
/// non-redeemable one, every no-resend notice would silently regain the
/// broken CTA. Cheap coupling test, mirrored on the frontend by
/// `budget-pause-notice.test.ts`'s "does not match the NO-RESEND sibling
/// prefix" fixture — which asserts the negative against the real
/// `BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX` string rather than an invented
/// near-miss (issue #1906: the claim was made here before that fixture
/// existed).
#[test]
fn the_redeemable_and_no_resend_prefixes_are_not_prefixes_of_each_other() {
    assert!(
        !BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX.starts_with(BUDGET_PAUSE_NOTICE_PREFIX),
        "a no-resend notice would match `isBudgetPauseNotice` and regain the CTA"
    );
    assert!(
        !BUDGET_PAUSE_NOTICE_PREFIX.starts_with(BUDGET_PAUSE_NOTICE_NO_RESEND_PREFIX),
        "a redeemable notice would stop matching and lose its working CTA"
    );
}

/// Issue #966: a bubble the runtime wrote names a non-agent author.
///
/// Asserts it is not `"operator"` specifically, rather than only that it
/// equals the constant. The whole defect is that a host-authored notice and
/// a reply whose author was overwritten stored the *same* value, so a test
/// that checked equality alone would still pass if `SYSTEM_AUTHOR` were ever
/// redefined to the channel name.
#[test]
fn a_host_authored_notice_is_not_authored_by_the_operator_channel() {
    let bubble = system_notice("Acknowledged.".to_string());
    assert_eq!(bubble.channel, "operator", "the destination is unchanged");
    assert_eq!(bubble.agent.as_deref(), Some(crate::ports::SYSTEM_AUTHOR));
    assert_ne!(
        bubble.agent.as_deref(),
        Some("operator"),
        "a notice must not store the author a destination-overwrite produces"
    );
}

// ── Issue #1861: blockers park instead of settling Failed ───────────────

/// The acceptance case: a dispatch that died on a model id the provider
/// rejects is answerable — somebody can set a real one — so it parks and
/// the card lands `paused` carrying the question, instead of dropping back
/// into To-do indistinguishable from work nobody started.
#[tokio::test]
async fn a_rejected_model_id_parks_a_blocker_rather_than_settling_failed() {
    use crate::harness::policy::ApprovalRequestQueue;
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());

    let reason = "dispatch failed: the model `gpt-nonexistent` does not exist or you do not \
                  have access to it";
    let end = brain.settle_as_blocker_or_failure("t-1", reason, Some("run-1"));

    assert_eq!(end, TaskRunEnd::Blocked);
    assert_eq!(
        lifecycle::landing_column(end),
        crate::ports::tasks::COLUMN_PAUSED,
        "a card with an open question on it has not failed — it is waiting"
    );

    let drained = requests.drain(8);
    assert_eq!(drained.requests.len(), 1, "exactly one question is asked");
    let effect = &drained.requests[0].effect;
    assert_eq!(effect.kind, "blocker.infrastructure");
    assert_eq!(effect.run_id.as_deref(), Some("run-1"));

    let payload: BlockerPayload =
        serde_json::from_value(effect.payload.clone()).expect("the payload round-trips");
    assert_eq!(payload.kind, BlockerKind::Infrastructure);
    assert_eq!(payload.source, BlockerSource::Provider);
    assert_eq!(
        payload.step,
        Some(BlockerStep::Task {
            task_id: "t-1".to_string()
        })
    );
    assert!(
        !payload.needed.trim().is_empty(),
        "a question that does not say what would answer it wastes the asking"
    );
}

/// The conservative default, pinned: a failure the classifier does not
/// recognise keeps today's behaviour exactly and asks nobody. Being wrong
/// in this direction costs a `Failed` that #1865 already surfaces; being
/// wrong the other way spends an operator's attention on a question they
/// cannot answer.
#[tokio::test]
async fn an_unrecognised_failure_still_fails_and_asks_nobody() {
    use crate::harness::policy::ApprovalRequestQueue;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());

    let end =
        brain.settle_as_blocker_or_failure("t-1", "dispatch failed: index out of bounds", None);

    assert_eq!(end, TaskRunEnd::Failed);
    assert_eq!(
        lifecycle::landing_column(end),
        crate::ports::tasks::COLUMN_TODO
    );
    assert!(
        requests.drain(8).requests.is_empty(),
        "an unrecognised failure must not reach the operator as a question"
    );
}

/// Recognising a transient stop is how we know **not** to ask: a rate limit
/// resolves itself, so it settles like any other failure and nothing is
/// parked.
#[tokio::test]
async fn a_rate_limit_settles_without_asking_anybody() {
    use crate::harness::policy::ApprovalRequestQueue;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());

    let end = brain.settle_as_blocker_or_failure(
        "t-1",
        "dispatch failed: hosted inference returned 429: rate limit exceeded",
        None,
    );

    assert_eq!(end, TaskRunEnd::Failed);
    assert!(requests.drain(8).requests.is_empty());
}

/// Approving a blocker must do **nothing** in this issue — the answer is
/// carried back into the stopped turn by #1863, and until then an approve
/// that half-executed something would be worse than one that does not.
///
/// `perform_effect` acts on three things: an `amount_usd` (writes a ledger
/// entry), a `channel`+`text` pair in the payload (sends a message), and
/// the email kind. This pins that a blocker effect carries none of them, so
/// the no-op is a property of the shape rather than a coincidence somebody
/// could break by adding a field.
#[tokio::test]
async fn a_parked_blocker_carries_nothing_an_executor_would_act_on() {
    use crate::harness::policy::ApprovalRequestQueue;

    let dir = tempfile::tempdir().unwrap();
    let requests = ApprovalRequestQueue::default();
    let brain = brain_with_approval_queue(dir.path(), requests.clone());

    brain.settle_as_blocker_or_failure(
        "t-1",
        "tool call failed: could not connect to mcp server `slack`",
        None,
    );

    let drained = requests.drain(8);
    let effect = &drained.requests[0].effect;
    assert!(effect.amount_usd.is_none(), "a question costs nothing");
    assert!(
        effect.payload.get("channel").is_none() && effect.payload.get("text").is_none(),
        "a `channel`+`text` payload would make approving a blocker post a message"
    );
    assert!(
        effect.agent.is_none(),
        "stamping an agent would mint a grant and re-dispatch the turn, which would \
         call the escalation again and park a second time"
    );
}
