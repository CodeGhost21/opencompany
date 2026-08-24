//! The run-observability read: what a company's agents actually did.
//!
//! Answers the question the REST run routes cannot: *given a workflow run, what
//! did each of its agent nodes do, step by step?* That join did not exist until
//! a workflow `agent` node started minting an attempt row — before it, a node's
//! turn had neither a card nor a conversation, so `RunStore` could not name it.
//!
//! # Why this is GraphQL and the timeline is REST
//!
//! `GET {scope}/runs/{id}` stays exactly as it is: it is shipping, tested, and
//! its shape is deliberately the console's `TimelineEntry` contract. This
//! surface exists for the *joined* read — run → attempts → steps → detail in one
//! request — which over REST would be one round trip per node and a client-side
//! assembly of the result.
//!
//! # The deep half
//!
//! [`RunStepGql::deep`] is unredacted: raw tool arguments, raw output, model
//! reasoning. It resolves through the same company scope every other field here
//! does, and it is `None` for a host that keeps no deep trace and for any step
//! that produced none. See [`crate::ports::deep_trace`] for what that store
//! holds and why it is separate.

use std::collections::HashMap;
use std::sync::Arc;

use async_graphql::{ID, Object, SimpleObject};

use crate::company::runtime::CompanyRuntime;
use crate::ports::deep_trace::TurnStepDetail;
use crate::ports::runs::{RunFilter, RunRecord, RunStepRecord};

/// Token and cost totals for one attempt.
#[derive(SimpleObject, Default)]
#[graphql(name = "RunUsage")]
pub struct RunUsageGql {
    /// Input tokens.
    pub input_tokens: f64,
    /// Output tokens.
    pub output_tokens: f64,
    /// Input tokens served from the provider's cache.
    pub cached_input_tokens: f64,
    /// Cost in USD.
    pub cost_usd: f64,
}

/// The unredacted companion of one step. **Carries secrets by construction.**
#[derive(SimpleObject)]
#[graphql(name = "DeepStepDetail")]
pub struct DeepStepDetailGql {
    /// Model reasoning for a thinking step.
    pub reasoning: Option<String>,
    /// The tool's arguments as the model emitted them, unredacted.
    pub arguments: Option<String>,
    /// The tool's raw output, before it was reduced to a shape.
    pub output: Option<String>,
    /// The harness's own contextual label.
    pub display_detail: Option<String>,
    /// Which pass of the tool loop this step belongs to.
    pub iteration: Option<i32>,
    /// Whether the store clipped any field above to its cap.
    pub clipped: bool,
}

impl From<TurnStepDetail> for DeepStepDetailGql {
    fn from(d: TurnStepDetail) -> Self {
        Self {
            reasoning: d.reasoning,
            arguments: d.arguments,
            output: d.output,
            display_detail: d.display_detail,
            iteration: d.iteration.map(|i| i as i32),
            clipped: d.clipped,
        }
    }
}

/// One step of an attempt's trace.
#[derive(SimpleObject)]
#[graphql(name = "RunStep")]
pub struct RunStepGql {
    /// The step's ordinal within its run.
    pub seq: i32,
    /// When it was recorded.
    pub at_millis: f64,
    /// `tool_call` | `thinking` | `note`.
    pub kind: String,
    /// `ok` | `error` | `running` | `awaiting_approval`.
    pub status: String,
    /// The display label — the tool's name, or "Thinking".
    pub label: String,
    /// Arguments, **through the host redactor**. Safe to render anywhere.
    pub detail: Option<String>,
    /// A summary or shape of the result — never a remote body.
    pub result: Option<String>,
    /// The typed failure class, when the step failed.
    pub failure: Option<String>,
    /// Whether the harness truncated the result before we saw it.
    pub truncated: bool,
    /// Wall-clock duration.
    pub elapsed_ms: Option<f64>,
    /// The unredacted half. `None` when this host keeps no deep trace, and when
    /// the step produced none.
    pub deep: Option<DeepStepDetailGql>,
}

/// One attempt at work — a card dispatch, a chat turn, or a workflow node.
pub struct AgentRunGql {
    record: RunRecord,
    steps: Vec<RunStepRecord>,
    details: HashMap<u32, TurnStepDetail>,
}

#[Object(name = "AgentRun")]
impl AgentRunGql {
    /// The attempt id.
    async fn id(&self) -> ID {
        ID(self.record.id.clone())
    }

    /// The teammate that ran it.
    async fn agent_id(&self) -> String {
        self.record.agent_id.clone()
    }

    /// 1-based attempt ordinal at its card.
    async fn attempt(&self) -> i32 {
        self.record.attempt as i32
    }

    /// `pending` | `running` | `waiting_approval` | `paused` | `succeeded` |
    /// `failed` | `cancelled`.
    async fn status(&self) -> String {
        self.record.status.as_str().to_string()
    }

    /// `active` | `parked` | `terminal` — read this rather than inferring a
    /// phase from timestamps.
    async fn phase(&self) -> String {
        format!("{:?}", self.record.status.phase()).to_lowercase()
    }

    /// The card this attempted, when it attempted one.
    async fn task_id(&self) -> Option<ID> {
        self.record.task_id.clone().map(ID)
    }

    /// The conversation it belongs to, when one raised it.
    async fn chat_id(&self) -> Option<ID> {
        self.record.chat_id.clone().map(ID)
    }

    /// The workflow run whose node spawned it.
    async fn workflow_run_id(&self) -> Option<ID> {
        self.record.workflow_run_id.clone().map(ID)
    }

    /// The graph node within that run.
    async fn node_id(&self) -> Option<ID> {
        self.record.node_id.clone().map(ID)
    }

    /// When the row was opened.
    async fn created_at_millis(&self) -> f64 {
        self.record.created_at_millis as f64
    }

    /// When it began running.
    async fn started_at_millis(&self) -> Option<f64> {
        self.record.started_at_millis.map(|v| v as f64)
    }

    /// When it settled. `None` while it is still going.
    async fn finished_at_millis(&self) -> Option<f64> {
        self.record.finished_at_millis.map(|v| v as f64)
    }

    /// Why it failed, when it did.
    async fn error(&self) -> Option<String> {
        self.record.error.clone()
    }

    /// Token and cost totals. Provisional until the attempt settles — they are
    /// written by the settle, not accumulated on the row.
    async fn usage(&self) -> RunUsageGql {
        RunUsageGql {
            input_tokens: self.record.usage.input as f64,
            output_tokens: self.record.usage.output as f64,
            cached_input_tokens: self.record.usage.cached_input as f64,
            cost_usd: self.record.usage.cost_usd,
        }
    }

    /// The settled step count.
    ///
    /// **Null while the attempt is live**, deliberately: `step_count` is written
    /// by the settle, so returning the stored `0` for a running attempt would be
    /// a lie that a client cannot detect. A live reader counts `steps` instead,
    /// and this being `null` is what tells it to.
    async fn step_count(&self) -> Option<i32> {
        self.record
            .status
            .is_terminal()
            .then_some(self.record.step_count as i32)
    }

    /// The step trace, oldest first.
    async fn steps(&self) -> Vec<RunStepGql> {
        self.steps
            .iter()
            .map(|record| {
                let step = &record.step;
                RunStepGql {
                    seq: record.step_seq as i32,
                    at_millis: record.at_millis as f64,
                    kind: format!("{:?}", step.kind).to_lowercase(),
                    status: match step.status {
                        crate::ports::types::TurnStepStatus::Ok => "ok",
                        crate::ports::types::TurnStepStatus::Error => "error",
                        crate::ports::types::TurnStepStatus::Running => "running",
                        crate::ports::types::TurnStepStatus::AwaitingApproval => {
                            "awaiting_approval"
                        }
                    }
                    .to_string(),
                    label: step.label.clone(),
                    detail: step.detail.clone(),
                    result: step.result.clone(),
                    failure: step.failure.map(|f| format!("{f:?}").to_lowercase()),
                    truncated: step.truncated,
                    elapsed_ms: step.elapsed_ms.map(|v| v as f64),
                    deep: self
                        .details
                        .get(&record.step_seq)
                        .cloned()
                        .map(DeepStepDetailGql::from),
                }
            })
            .collect()
    }
}

/// Loads one attempt with its trace and, when the host keeps one, its deep half.
async fn load(runtime: &Arc<CompanyRuntime>, record: RunRecord) -> AgentRunGql {
    let steps = runtime
        .runs()
        .list_run_steps(runtime.id(), &record.id)
        .await
        .unwrap_or_default();
    // A missing deep store, or a read that fails, degrades to "no deep half"
    // rather than failing the query: the scrubbed trace is the answer, and the
    // unredacted companion is the bonus.
    let details = runtime
        .deep_trace()
        .list_step_details(runtime.id(), &record.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|d| (d.step_seq, d.detail))
        .collect();
    AgentRunGql {
        record,
        steps,
        details,
    }
}

/// `Company.agentRuns` — attempts, newest first, optionally narrowed.
pub(crate) async fn resolve_runs(
    runtime: &Arc<CompanyRuntime>,
    task_id: Option<String>,
    workflow_run_id: Option<String>,
    limit: i32,
) -> async_graphql::Result<Vec<AgentRunGql>> {
    let filter = RunFilter {
        task_id,
        workflow_run_id,
        statuses: Vec::new(),
        limit: Some(limit.clamp(1, 200) as usize),
    };
    let rows = runtime.runs().list_runs(runtime.id(), &filter).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(load(runtime, row).await);
    }
    Ok(out)
}

/// `Company.agentRun` — one attempt by id, or null.
pub(crate) async fn resolve_run(
    runtime: &Arc<CompanyRuntime>,
    id: String,
) -> async_graphql::Result<Option<AgentRunGql>> {
    let Some(record) = runtime.runs().get_run(runtime.id(), &id).await? else {
        return Ok(None);
    };
    Ok(Some(load(runtime, record).await))
}
