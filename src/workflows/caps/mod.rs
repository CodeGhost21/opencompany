//! The tinyflows [`Capabilities`] bundle for a company workflow run.
//!
//! tinyflows is host-agnostic: every outside-world effect is a trait the host
//! implements. This module supplies that bundle for an OpenCompany run.
//!
//! Wired capabilities (P1):
//!
//! * **agent** ([`HarnessAgentRunner`]) — an `agent` node (config `agent_ref` =
//!   a roster teammate id) routes to the company's
//!   [`HarnessPool`](crate::harness::HarnessPool), so the step runs on the same
//!   live openhuman agent as chat/task dispatch — inheriting its persona, model,
//!   [`OcMemory`](crate::harness::memory), approval policy, and cost metering.
//! * **tool_call** ([`WorkflowToolInvoker`](tools::WorkflowToolInvoker)) — a
//!   `tool_call` node executes a real Cell A toolbelt tool (`shell` / `code` /
//!   `web`, plus the metered `search` family behind an explicit `search` grant)
//!   scoped to a dedicated per-company workflow workspace, fail-closed on the
//!   company's `[tools].allow` grants.
//! * **http_request** ([`GuardedHttpClient`](http::GuardedHttpClient)) — an
//!   `http_request` node routes through OpenHuman's `HttpRequestTool` so every
//!   request (and redirect) passes the upstream `url_guard` SSRF check.
//! * **state** ([`CompanyStateStore`](state::CompanyStateStore)) — durable
//!   per-run key/value over the [`SecretStore`](crate::ports::SecretStore) seam.
//!   No tinyflows node OpenCompany emits consumes it yet; it is deliberate
//!   contract-plumbing a later phase (P3) consumes.
//!
//! Wired in P2:
//!
//! * **sub_workflow** ([`StoreWorkflowResolver`](resolver::StoreWorkflowResolver))
//!   — a `sub_workflow` node referencing a child by `workflow_id` resolves it
//!   from the union of the company's seed `workflows/` directory
//!   ([`HarnessDeps::workflow_source_dir`](crate::harness::HarnessDeps)) and the
//!   record's runtime-authored graph bodies (full validation + a static cycle
//!   guard). A platform-provisioned tenant has no source directory, so every
//!   child it owns resolves from the record.
//!
//! Still **not wired**: the bare-completion `LlmProvider` fallback and `code`
//! nodes. They are explicit stubs that return a clear capability error rather
//! than a silent no-op, so a workflow that reaches one fails loudly; a workflow
//! that never reaches one is unaffected.
//!
//! Also not wired, and for a different reason: **memory**, which tinyflows 0.6
//! added with the #499 pin bump. The other two are unbuilt; this one is
//! *undecided*. A `MemoryProvider` would give a workflow read and **write**
//! access to agent memory, and which scopes a workflow may touch has not been
//! settled — so it is left `None` until it is, and
//! [`the_memory_capability_is_left_unwired_on_purpose`](tests) pins that so the
//! answer has to be given rather than defaulted into.

mod dry_run;
mod http;
mod resolver;
mod state;
mod tools;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tinyflows::caps::{
    AgentRunner, Capabilities, CodeLanguage, CodeRunner, HttpClient, LlmProvider, StateStore,
    ToolInvoker, WorkflowResolver,
};
use tinyflows::error::{EngineError, Result as TfResult};

use crate::harness::policy::{ApprovalScope, MAX_APPROVAL_REQUESTS_PER_TURN, PolicyMode};
use crate::harness::{HarnessDeps, HarnessPool, toolbelt};
use crate::ports::types::{CompanyId, CompanyRecord};

use self::http::GuardedHttpClient;
use self::resolver::StoreWorkflowResolver;
use self::state::{CompanyStateStore, NoopState};
pub(crate) use self::tools::WORKFLOW_TOOL_NAMESPACES;
use self::tools::WorkflowToolInvoker;

/// The four effectful capability slots [`build_capabilities`] chooses by mode:
/// `tool_call`, `http_request`, `state`, and the optional `agent` runner. The
/// dry and live branches each build one of these; the read-only `resolver` and
/// the always-stub `llm`/`code`/`memory` slots are assembled outside it.
type EffectSlots = (
    Arc<dyn ToolInvoker>,
    Arc<dyn HttpClient>,
    Arc<dyn StateStore>,
    Option<Arc<dyn AgentRunner>>,
);

/// What one run needs the capability bundle to know about *itself*.
///
/// Bundled rather than passed as five more parameters (issue #638 added the
/// fifth and tipped `build_capabilities` over clippy's arity limit). They
/// genuinely travel together — every one is scoped to this run and meaningless
/// without the others — so a struct is the honest shape rather than a way of
/// making the lint quiet.
pub struct RunContext<'a> {
    /// The workflow being run.
    pub workflow_id: &'a str,
    /// This run's id (issue #395), the key its approvals are stamped with.
    pub run_id: &'a str,
    /// The operator's topic for this run (issue #154), threaded to the agent
    /// capability so a node's turn carries what was actually asked.
    pub run_request: Option<String>,
    /// Issue #542: stub every effectful slot and journal nothing.
    pub dry_run: bool,
    /// Where an agent node leaves an operator-facing notice (issue #638).
    pub notices: RunNotices,
}

/// Assembles the [`Capabilities`] bundle for a run of `workflow_id`.
///
/// `record` carries everything the outside-world capabilities need: the company
/// id, the `[policy].mode` (the exec-security autonomy tier), the `[tools].allow`
/// grants (the fail-closed `tool_call` gate), and the `[tools].web_allowed_domains`
/// SSRF allowlist. The tool_call / http_request capabilities are scoped to a
/// dedicated per-run workflow workspace
/// (`{workspace_root}/{company}/_workflow/{workflow}/{run}/workspace`) — the
/// `_` prefix keeps it from ever colliding with a roster agent's own workspace
/// directory.
///
/// `pool`/`deps` are shared with the rest of the harness surface — the roster the
/// agent nodes address is the one already resident in `pool`.
///
/// `run_request` is the operator's topic for this run (issue #154), threaded to
/// the agent capability so every agent node's turn message carries what was
/// actually asked, not just the node's authored instruction.
///
/// `dry_run` (issue #542) selects the **mode**, one assembly point so the two
/// bundles cannot drift. When `true`, every *effectful* slot is a stub from
/// [`dry_run`]: the agent echoes with no inference, `tool_call` keeps its
/// fail-closed grant check but executes nothing, `http_request` sends nothing,
/// and `state` is [`NoopState`] rather than the durable
/// [`CompanyStateStore`](state::CompanyStateStore). The read-only `resolver`
/// stays real in both modes, so a `sub_workflow` child runs under this same
/// bundle and a dry run propagates into it. The per-run workspace, exec-security
/// policy and search backend are not built at all for a dry run — nothing needs
/// them. Because every effect is stubbed, a future node kind cannot reach a real
/// effect through a dry bundle: the engine only calls what is on the bundle.
pub async fn build_capabilities(
    pool: Arc<HarnessPool>,
    deps: HarnessDeps,
    record: &CompanyRecord,
    run: RunContext<'_>,
) -> Capabilities {
    let RunContext {
        workflow_id,
        run_id,
        run_request,
        dry_run,
        notices,
    } = run;
    let company = record.id.clone();
    // Issue #562: the tier actually in force — the operator's console override
    // when one is set, the manifest's otherwise. Reading `manifest.policy` here
    // would leave a workflow run on the shipped tier while the roster ran on the
    // operator's, which is the disagreement `effective_policy` exists to prevent.
    let mode = PolicyMode::parse(&record.effective_policy().mode);
    let grants = record.manifest.tools.allow.clone();

    // sub_workflow-by-id resolves children from the union of the company's seed
    // `workflows/` directory and the record's runtime-authored bodies — so a
    // platform tenant with no source dir still resolves the workflows it
    // created (issue #168). Read before `deps` may move into the agent runner.
    // REAL in both modes: it is a read, and a dry sub_workflow child runs under
    // this same (dry) bundle, so dry propagates rather than stopping here.
    // Issue #617: a child's nodes never reach the gate pass, so the resolver
    // carries the policy in order to *say* which of its calls were never
    // offered for approval. `None` for a dry run — nothing executes, so there
    // is nothing to disclose, and the resolver behaves exactly as before.
    let audit = (!dry_run).then(|| self::resolver::ChildCallAudit {
        policy: record.manifest.policy.clone(),
        run_id: run_id.to_string(),
        events: deps.events.clone(),
    });
    let resolver: Arc<dyn WorkflowResolver> = Arc::new(StoreWorkflowResolver::new(
        deps.workflow_source_dir.clone(),
        deps.store.clone(),
        company.clone(),
        workflow_id.to_string(),
        audit,
    ));

    // The four effectful slots, chosen by mode at this one point.
    let (tools, http, state, agent): EffectSlots = if dry_run {
        // DRY: stub every effect. No workspace mkdir, no exec-security, no pool
        // routing, no secret store. The grant check is KEPT (pure) so an
        // ungranted `tool_call` refuses identically; state is the inert no-op so
        // a dry run cannot persist either.
        tracing::debug!(
            company = %company,
            workflow = workflow_id,
            "workflow: building DRY capability bundle — no real effects will run"
        );
        (
            Arc::new(dry_run::DryRunTools::new(grants)),
            Arc::new(dry_run::DryRunHttp),
            Arc::new(NoopState),
            Some(Arc::new(dry_run::DryRunAgent)),
        )
    } else {
        let workflow_ws = workflow_workspace(&deps.workspace_root, &company, workflow_id, run_id);
        if let Err(err) = tokio::fs::create_dir_all(&workflow_ws).await {
            tracing::warn!(
                company = %company,
                workspace = %workflow_ws.display(),
                %err,
                "workflow: could not create the per-run workspace"
            );
        }

        // ONE exec-security policy shared by the tool_call toolbelt and the
        // http_request client, sandboxed to the workflow workspace with the
        // company's autonomy tier — exactly the shape a roster agent's exec
        // tools get.
        let exec_security = Arc::new(toolbelt::exec_security(&workflow_ws, mode));
        let web_allowed_domains = record.manifest.tools.web_allowed_domains.clone();

        // The metered `search` family is threaded through the invoker the same
        // way `build_agent` wires it onto a roster agent — explicit `search`
        // grant + managed backend, fail-closed. Read `deps.search` / `deps.meter`
        // here, before `deps` moves into `HarnessAgentRunner` below. The agent
        // label names the run so a search sample is attributed to the workflow,
        // not a chat turn.
        let search_metering = crate::harness::search::SearchMetering {
            company: company.clone(),
            agent: format!("workflow:{workflow_id}"),
            meter: deps.meter.clone(),
        };
        let tools = WorkflowToolInvoker::new(
            exec_security.clone(),
            &workflow_ws,
            web_allowed_domains.clone(),
            grants,
            &deps.capabilities,
            deps.search.as_ref(),
            search_metering,
        );
        let http = GuardedHttpClient::new(exec_security, web_allowed_domains);

        // Durable run state over the per-company secret store, namespaced by
        // workflow id. `None` (default/tests) keeps the inert no-op with a
        // warning — no node OpenCompany emits reads state in P1, so this never
        // blocks a run.
        let state: Arc<dyn StateStore> = match &deps.secrets {
            Some(secrets) => Arc::new(CompanyStateStore::new(
                secrets.clone(),
                company.clone(),
                workflow_id.to_string(),
            )),
            None => {
                tracing::warn!(
                    company = %company,
                    workflow = workflow_id,
                    "workflow: no secret store wired; run state is a no-op (deliberate — no P1 node uses it)"
                );
                Arc::new(NoopState)
            }
        };

        // `deps` moves in last — the borrows above (`deps.capabilities`,
        // `deps.search`, `deps.meter`, `deps.secrets`, `deps.workspace_root`)
        // are all done by here.
        let agent: Arc<dyn AgentRunner> = Arc::new(HarnessAgentRunner::new(
            pool,
            deps,
            company.clone(),
            run_id.to_string(),
            run_request,
            notices,
        ));
        (Arc::new(tools), Arc::new(http), state, Some(agent))
    };

    Capabilities {
        llm: Arc::new(UnwiredLlm),
        tools,
        http,
        code: Arc::new(UnwiredCode),
        state,
        resolver,
        agent,
        // New in tinyflows 0.6, which arrived with the #499 pin bump. Left
        // unwired deliberately rather than pointed at the company's context
        // store: a `memory` node would then read and WRITE agent memory on
        // behalf of a workflow, and which scopes a workflow may touch is a
        // policy question this repo has not answered — `remember`/`forget`
        // especially. `None` fails such a node at run time with a capability
        // error, which is the honest answer until that decision is made, and
        // no company manifest can currently produce one (`NodeKind` has no
        // `memory` variant on our side).
        memory: None,
    }
}

/// Builds a traversal-safe workspace path unique to one workflow execution.
fn workflow_workspace(
    root: &std::path::Path,
    company: &CompanyId,
    workflow_id: &str,
    run_id: &str,
) -> std::path::PathBuf {
    root.join(company.as_ref())
        .join("_workflow")
        .join(hex_segment(workflow_id))
        .join(hex_segment(run_id))
        .join("workspace")
}

/// Encodes an arbitrary identifier as one safe, reversible path segment.
fn hex_segment(value: &str) -> String {
    use std::fmt::Write;
    value
        .as_bytes()
        .iter()
        .fold(String::with_capacity(value.len() * 2), |mut out, byte| {
            write!(out, "{byte:02x}").expect("writing to String cannot fail");
            out
        })
}

/// A tinyflows [`AgentRunner`] that executes an `agent` node on the company's
/// [`HarnessPool`].
///
/// The engine calls [`run_agent`](AgentRunner::run_agent) with the node's
/// resolved config as `request` and the (trusted) `agent_ref` as the roster
/// teammate id. This extracts the turn message from the request and runs it
/// through [`HarnessPool::run`], which meters the turn's cost through `deps` — so
/// a workflow step and a chat turn account identically.
///
/// # It claims neither the publish queue nor the delegation queue — on purpose
///
/// A node's turn carries the whole toolbelt, so an orchestrator-tier `agent_ref`
/// can reach `review_task`, `assign_task`, `spawn_task` and `delegate_to_desk`,
/// and a granted one can reach `publish_artifact`. Nothing here drains either
/// queue: a workflow run has no board card behind it and no conversation to
/// surface a delegation's answer into — the same absence that makes
/// [`park_gated_calls`](HarnessAgentRunner::park_gated_calls) record approvals
/// explicitly unlinked.
///
/// So this path takes **no claim**, and that is the decision rather than an
/// omission. What the agent gets is an honest in-turn refusal it can report —
/// *"the card was NOT reviewed"* — instead of what it got before #453: a
/// success receipt saying the card had moved to done, followed by the next
/// turn's `clear()` destroying the delegation. Silent destruction replaced by a
/// visible refusal.
///
/// Wiring a drain here would be a real feature (a workflow node that can move
/// the board), and it is deliberately not this issue: it needs a decision about
/// which card a node's `spawn_task` parents to and where a hand-off's reply
/// goes. Until then, refusing is the truthful answer. Mirrors how this path
/// already takes no [`PublishClaim`](crate::harness::publish::PublishClaim).
pub struct HarnessAgentRunner {
    pool: Arc<HarnessPool>,
    deps: HarnessDeps,
    company: CompanyId,
    /// The run these agent nodes belong to (issue #395), stamped onto every
    /// approval this node's turn parks so the Approvals page can say which
    /// workflow run is waiting on the operator.
    run_id: String,
    /// What the operator asked for on this run (issue #154), when they supplied
    /// it. A node's `prompt` is authored into the graph and is the same on every
    /// run, so without this the run's topic never reaches the teammate doing the
    /// work — the agent would run, find no subject, and ask for one.
    run_request: Option<String>,
    /// Where this node leaves an operator-facing notice (issue #638).
    notices: RunNotices,
}

/// Where an agent node leaves a notice for the operator (issue #638).
///
/// A shared handle rather than a return value because there is nowhere to
/// return it to: `AgentRunner::run_agent` hands the engine a `Value` that
/// becomes the node's output, and a system notice is emphatically not node
/// output — it would ride into a downstream `=item` binding and into the run's
/// persisted output snapshot. So the notice goes sideways, out to the runner
/// that owns the run, and lands on [`WorkflowRun::notices`].
///
/// Cheap to clone; every clone appends to the same list, which is what lets one
/// run's several agent nodes each contribute.
#[derive(Clone, Default)]
pub struct RunNotices {
    inner: Arc<std::sync::Mutex<Vec<String>>>,
}

impl RunNotices {
    /// Records one notice.
    pub fn push(&self, notice: String) {
        self.inner
            .lock()
            .expect("run notices poisoned")
            .push(notice);
    }

    /// Takes everything recorded so far, leaving the collector empty.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.inner.lock().expect("run notices poisoned"))
    }
}

impl HarnessAgentRunner {
    /// Builds a runner over an already-populated pool for `company`, carrying
    /// the run's id (issue #395) and the operator's run request (issue #154)
    /// when one was supplied.
    pub fn new(
        pool: Arc<HarnessPool>,
        deps: HarnessDeps,
        company: CompanyId,
        run_id: String,
        run_request: Option<String>,
        notices: RunNotices,
    ) -> Self {
        Self {
            pool,
            deps,
            company,
            run_id,
            run_request,
            notices,
        }
    }

    /// Parks every approval-gated tool call this node's turn just recorded
    /// (issue #395) — the drain the workflow path never had.
    ///
    /// # The hole this closes
    ///
    /// [`ApprovalPolicy`](crate::harness::policy::ApprovalPolicy) is installed
    /// pool-wide on every roster agent with the shared
    /// [`ApprovalRequestQueue`](crate::harness::policy::ApprovalRequestQueue),
    /// so a gated tool call inside a workflow agent node **was** recorded. But
    /// the only drain in the codebase is
    /// [`park_approval_requests`](crate::harness::HarnessBrain), which lives
    /// inside `run_cycle` and needs a
    /// [`CycleHost`](crate::ports::brain::CycleHost). This path —
    /// `run_agent` → [`HarnessPool::run_background`] → `run_inner` — never goes
    /// near a cycle, so nothing drained, and the next chat cycle's
    /// [`clear`](crate::harness::policy::ApprovalRequestQueue::clear) threw the
    /// request away. The queue's own doc names this case. The leak was
    /// prevented; the parking was never added. That is why the Approvals page
    /// stayed "All clear" through a run an operator watched get gated.
    ///
    /// # Scope, not boundary (issue #439)
    ///
    /// This block used to describe a boundary index: `from` taken before the
    /// turn, only the tail above it claimed, because the queue was shared with
    /// whatever chat cycle happened to be running and `drain` would have taken
    /// that cycle's entries and cleared the rest.
    ///
    /// It also said, accurately, that this **narrowed** the race rather than
    /// eliminating it — a chat turn pushing while this node ran landed above
    /// the boundary and was parked here with this run's id on it — and that the
    /// real fix was a per-run queue, deferred out of #395.
    ///
    /// That is now done, though **not** in the shape that sentence predicted.
    /// One queue per run is unbuildable: `ApprovalPolicy` is installed by
    /// `build_roster` inside a fingerprint-cached, per-company
    /// `HarnessPool::ensure` with no run id in scope, and is then sealed into
    /// the vendored agent with no setter, so there is nowhere to hand a
    /// per-run queue *to*. The separation is in the key instead — the run takes
    /// an [`ApprovalScope::Run`](crate::harness::policy::ApprovalScope) claim
    /// and pushes route into its own bucket — which yields the same property
    /// the issue asked for: a turn sees only its own requests.
    ///
    /// It also closes a race the boundary never addressed. Two workflow runs
    /// overlap (they are spawned, not under the cycle lock), and both took a
    /// boundary against the same vector, so the later `split_off` swallowed the
    /// earlier run's tail. Scopes are disjoint, so that cannot happen.
    ///
    /// # Never fails the node
    ///
    /// A park that errors is logged per entry and the loop continues. The turn
    /// already happened, the model was already told it was refused, and failing
    /// the node here would discard a completed turn's work over a queue write.
    /// Same stance `park_approval_requests` takes, for the same reason.
    ///
    /// # A run cancelled mid-turn parks nothing, deliberately
    ///
    /// Stopping a run drops the engine future *mid-await* (issue #383), which
    /// takes this call with it — so a call the policy had already gated is
    /// discarded rather than parked. That is the intended outcome, not a
    /// residual leak: an operator who stopped a run is not asking to be asked
    /// about the work they stopped. It is the same judgement `cancelled_run`
    /// makes in reporting no `pending_approvals`, and the same one
    /// `park_pending_gates` makes in skipping a cancelled run.
    ///
    /// Issue #439 made this **cleaner, and no longer anyone else's business**.
    /// The discard used to be performed by the next chat cycle's `clear`, which
    /// only worked because the queue was shared — the cancelled run's leftovers
    /// were sitting in the cycle's way. Now the claim's `Drop` takes them as the
    /// dropped future unwinds, so the entries never outlive the run and no
    /// other turn has to sweep up after it.
    async fn park_gated_calls(&self) {
        let queue = &self.deps.approval_requests;
        // Issue #242's stamp. The `from` is now 0 because the scope *is* the
        // entitlement: every entry in this bucket was pushed by this run's own
        // turn, so there is no prefix belonging to anyone else to skip past.
        // That is what #439 bought — the boundary index encoded a guess about
        // who wrote what, and the scope encodes the fact.
        queue.stamp_run(0, &self.run_id);
        // The discard count comes off the drain itself (issue #561): `drain`
        // caps and drops the remainder in one step, so by the time it returns,
        // how many went is not recoverable from what came back. Reading it
        // here is what keeps the overflow warning below reachable — without a
        // count, a run that flooded the gate looks identical to one that did
        // not.
        let drained = queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN);
        let notice = drained.overflow_notice();
        let discarded = drained.discarded;
        let requests = drained.requests;
        if requests.is_empty() {
            return;
        }

        // Issue #638: told to the operator, not only logged. Raised BEFORE the
        // parking guard below, and that ordering is a fix in itself — the guard
        // `return`s, so on a runtime with no approvals gate the overflow was
        // not even reaching the log. The notice is about calls that were
        // *discarded*, which is true whether or not the survivors could be
        // parked; if anything it matters more when they could not.
        if let Some(notice) = notice {
            // `overflow_notice` rather than a sentence of our own: the wording
            // lives on `DrainedRequests` (#561) precisely so the chat path and
            // this one cannot tell an operator the same thing two ways.
            self.notices.push(notice);
        }
        if discarded > 0 {
            tracing::warn!(
                company = %self.company,
                run_id = %self.run_id,
                discarded,
                "workflow agent node: more gated tool calls than one run may park; the excess \
                 was discarded"
            );
        }

        let Some(parking) = self
            .deps
            .delivery
            .as_ref()
            .and_then(|delivery| delivery.parking.as_ref())
        else {
            // Loud: these requests are already off the queue and are the only
            // trace of calls the operator will never be asked about.
            tracing::error!(
                company = %self.company,
                run_id = %self.run_id,
                requests = requests.len(),
                "workflow agent node: gated tool calls could NOT be parked — this runtime has \
                 no approvals queue wired; the operator will not be asked about them"
            );
            return;
        };

        for request in requests {
            // The delivery precedent: a workflow run has no board card behind it
            // and no conversation to raise the request in, so it is recorded
            // explicitly unlinked (#333) and stays Approvals-page-only (#379).
            match parking
                .park_and_journal(
                    &self.company,
                    request.effect,
                    crate::runtime::journal::TaskLink::Unlinked,
                    None,
                )
                .await
            {
                Ok(approval_id) => tracing::info!(
                    company = %self.company,
                    run_id = %self.run_id,
                    tool = %request.tool,
                    approval_id = %approval_id,
                    "workflow agent node: parked a gated tool call for operator approval"
                ),
                Err(err) => tracing::error!(
                    company = %self.company,
                    run_id = %self.run_id,
                    tool = %request.tool,
                    %err,
                    "workflow agent node: failed to park a gated tool call; the operator will \
                     not be asked about it"
                ),
            }
        }
    }
}

#[async_trait]
impl AgentRunner for HarnessAgentRunner {
    async fn run_agent(
        &self,
        agent_ref: &str,
        request: Value,
        _conn: Option<&str>,
    ) -> TfResult<Value> {
        let message =
            compose_turn_message(&message_from_request(&request), self.run_request.as_deref());
        tracing::debug!(
            company = %self.company,
            agent = agent_ref,
            "workflow agent node: routing to harness pool"
        );
        // Issue #439: this run's own approval scope, replacing #395's boundary
        // index. The index was only ever a narrowing — it was taken against a
        // vector any concurrent turn could append to, so a chat cycle pushing
        // inside the window landed above the boundary and was parked here with
        // this run's id on it, and two concurrent runs each took a boundary
        // against the same vector so the later `split_off` swallowed the
        // earlier one's tail. A scope removes both by construction: nothing
        // else can write into this bucket.
        let claim = self
            .deps
            .approval_requests
            .claim(ApprovalScope::Run(self.run_id.clone()));
        let outcome = claim
            .scoped(
                self.pool
                    .run_background(&self.company, agent_ref, &message, &self.deps),
            )
            .await;
        // Drained on BOTH arms, deliberately. A turn that errored may still have
        // had a tool call gated before it failed, and that request is just as
        // real — dropping the claim without parking would discard it, which is
        // the exact disappearance this issue is about.
        //
        // Inside the scope, so the drain reads this run's bucket rather than
        // whatever `Unscoped` happens to hold.
        claim.scoped(self.park_gated_calls()).await;
        let outcome = outcome
            .map_err(|e| EngineError::Capability(format!("harness agent '{agent_ref}': {e}")))?;
        // Mirror the engine's `{ json, text, raw }` envelope shape: expose the
        // reply as `text` so a downstream `=item.text` binding resolves. A
        // workflow node carries no chat bubble, so the turn's steps are dropped
        // here (they surface only on operator/desk chat replies).
        Ok(json!({ "text": outcome.reply, "agent_ref": agent_ref }))
    }
}

/// Extracts the turn message from an agent node's resolved config: the `prompt`
/// string when present (what [`translate`](crate::workflows::translate) writes),
/// else the `input`/`message` string, else the whole request serialized.
fn message_from_request(request: &Value) -> String {
    for key in ["prompt", "input", "message"] {
        if let Some(text) = request.get(key).and_then(Value::as_str) {
            return text.to_string();
        }
    }
    request.to_string()
}

/// Combines a node's authored instruction with the operator's run request
/// (issue #154).
///
/// A node's `prompt` is baked into the graph, so it is identical on every run —
/// it says *what this step does*, never *what was asked this time*. Before this,
/// the run's topic stopped at the trigger node and the agent had no subject to
/// work on, which is what made a run end with the agent asking the operator for
/// a topic they had no field to supply.
///
/// The instruction stays first so the node's job still leads; the request is
/// appended under a labelled heading so a teammate can tell the standing
/// instruction from this run's subject. A blank or whitespace-only request is
/// treated as absent, leaving the message byte-identical to the previous
/// behaviour — runs that supply no topic are unchanged.
fn compose_turn_message(instruction: &str, run_request: Option<&str>) -> String {
    let request = run_request.map(str::trim).filter(|r| !r.is_empty());
    match request {
        Some(request) => {
            let instruction = instruction.trim();
            if instruction.is_empty() {
                return request.to_string();
            }
            format!("{instruction}\n\nRequest for this run:\n{request}")
        }
        None => instruction.to_string(),
    }
}

/// Extracts a human-readable run request from the trigger input (issue #154).
///
/// The console posts `{"request": "…"}`, but the run endpoint accepts an
/// arbitrary JSON trigger payload, so this also accepts a bare string and the
/// nearby key spellings a hand-written call or an older client may use. Anything
/// else (an object with no recognised key, a number, `null`) yields `None` and
/// the run proceeds exactly as it did before — the topic is an addition, not a
/// new requirement.
pub(super) fn run_request_text(input: &Value) -> Option<String> {
    let text = match input {
        Value::String(s) => s.as_str(),
        Value::Object(_) => ["request", "input", "topic", "message", "text"]
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))?,
        _ => return None,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

/// The bare-completion fallback. An `agent` node with no `agent_ref` would land
/// here; [`translate`](crate::workflows::translate) always sets `agent_ref` for a
/// roster agent, so reaching this means an agent node with no teammate assigned.
struct UnwiredLlm;

#[async_trait]
impl LlmProvider for UnwiredLlm {
    async fn complete(&self, _request: Value, _conn: Option<&str>) -> TfResult<Value> {
        Err(EngineError::Capability(
            "workflow agent node has no roster agent; bare LLM completion is not wired for \
             company workflows"
                .to_string(),
        ))
    }
}

/// `code` nodes are not part of the OpenCompany model and never emitted by
/// translation; wired to an error for completeness.
struct UnwiredCode;

#[async_trait]
impl CodeRunner for UnwiredCode {
    async fn run(&self, _language: CodeLanguage, _source: &str, _input: Value) -> TfResult<Value> {
        Err(EngineError::Capability(
            "code execution is not supported for company workflows".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #638: a node that gates more calls than the cap allows leaves the
    /// operator a **notice**, not only a log line.
    ///
    /// Asserted on `RunNotices` — the value that becomes `WorkflowRun::notices`
    /// and then the journaled outcome the history panel reads — rather than on
    /// a log, which is what the issue asks for and what the chat path already
    /// had via #561.
    #[tokio::test]
    async fn an_overflowing_node_leaves_the_operator_a_notice() {
        let over = MAX_APPROVAL_REQUESTS_PER_TURN + 3;
        let (notices, queue) = overflowing_runner_notices(over, true).await;

        assert_eq!(notices.len(), 1, "one notice for one overflow: {notices:?}");
        let notice = &notices[0];
        assert!(
            notice.contains(&format!("at most {MAX_APPROVAL_REQUESTS_PER_TURN}")),
            "it must quote the cap that did the discarding: {notice}"
        );
        assert!(notice.contains('3'), "…and how many went past it: {notice}");
        assert_eq!(
            queue.drain(MAX_APPROVAL_REQUESTS_PER_TURN).requests.len(),
            0,
            "the drain already emptied this run's scope"
        );
    }

    /// The ordering fix that rides with it. The `parking`-is-`None` guard
    /// `return`s, and it used to sit **above** the overflow branch — so on a
    /// runtime with no approvals gate the discard was not even reaching the
    /// log, let alone the operator.
    ///
    /// That is the worst case, not a corner: the survivors could not be parked
    /// either, so the notice is the *only* thing the operator can be told.
    #[tokio::test]
    async fn the_notice_survives_a_runtime_with_no_approvals_gate() {
        let over = MAX_APPROVAL_REQUESTS_PER_TURN + 2;
        let (notices, _) = overflowing_runner_notices(over, false).await;
        assert_eq!(
            notices.len(),
            1,
            "no gate to park into is exactly when the operator most needs telling: {notices:?}"
        );
    }

    /// A node that stayed under the cap says nothing — the notice must be the
    /// exception, not a line on every run.
    #[tokio::test]
    async fn a_node_within_the_cap_raises_no_notice() {
        let (notices, _) = overflowing_runner_notices(MAX_APPROVAL_REQUESTS_PER_TURN, true).await;
        assert!(notices.is_empty(), "nothing was discarded: {notices:?}");
    }

    /// Queues `count` gated calls in a run's scope, drains them through
    /// `park_gated_calls`, and returns whatever the run was told.
    ///
    /// `with_gate` selects whether a `parking` sink is wired, which is the axis
    /// the guard-order test needs.
    async fn overflowing_runner_notices(
        count: usize,
        with_gate: bool,
    ) -> (Vec<String>, crate::harness::policy::ApprovalRequestQueue) {
        use crate::harness::policy::{ApprovalRequest, ApprovalScope};
        use crate::ports::types::{Effect, EffectGroup};

        let dir = tempfile::Builder::new()
            .prefix("oc-638-")
            .tempdir()
            .expect("tempdir");
        let (mut deps, _journal) =
            crate::workflows::gated_tool_turn_test::deps(String::new(), dir.path());
        if !with_gate {
            deps.delivery = None;
        }
        let queue = deps.approval_requests.clone();
        let notices = RunNotices::default();
        let runner = HarnessAgentRunner::new(
            Arc::new(HarnessPool::new()),
            deps,
            CompanyId::new("acme"),
            "run-1".to_string(),
            None,
            notices.clone(),
        );

        // Pushed inside the run's own scope, exactly as its turn would.
        let claim = queue.claim(ApprovalScope::Run("run-1".to_string()));
        claim
            .scoped(async {
                for i in 0..count {
                    queue.push(ApprovalRequest {
                        tool: "shell".to_string(),
                        reason: "gated".to_string(),
                        effect: Effect {
                            kind: "shell".to_string(),
                            group: EffectGroup::Other,
                            amount_usd: None,
                            established_thread: false,
                            first_time_counterparty: false,
                            payload: json!({ "n": i }),
                            agent: Some("ceo".to_string()),
                            run_id: None,
                        },
                    });
                }
            })
            .await;
        claim.scoped(runner.park_gated_calls()).await;
        (notices.take(), queue)
    }

    #[test]
    fn message_prefers_prompt_then_input_then_message() {
        assert_eq!(
            message_from_request(&json!({ "prompt": "P", "input": "I" })),
            "P"
        );
        assert_eq!(message_from_request(&json!({ "input": "I" })), "I");
        assert_eq!(message_from_request(&json!({ "message": "M" })), "M");
    }

    // ── Issue #154: the operator's run request reaches the agent ──

    #[test]
    fn run_request_is_appended_under_a_labelled_heading() {
        let out = compose_turn_message("Draft the launch post.", Some("dark mode for iOS"));
        // The node's standing instruction still leads.
        assert!(out.starts_with("Draft the launch post."), "{out}");
        // …and this run's subject is distinguishable from it.
        assert!(out.contains("Request for this run:"), "{out}");
        assert!(out.contains("dark mode for iOS"), "{out}");
    }

    #[test]
    fn a_run_with_no_request_is_byte_identical_to_the_old_message() {
        // The guarantee that makes this safe to land: runs that supply no topic
        // must behave exactly as they did before.
        for empty in [None, Some(""), Some("   "), Some("\n\t ")] {
            assert_eq!(
                compose_turn_message("Draft the launch post.", empty),
                "Draft the launch post.",
                "empty request {empty:?} must not alter the message"
            );
        }
    }

    #[test]
    fn a_request_with_no_instruction_stands_on_its_own() {
        // No dangling heading when the node carries no usable instruction.
        assert_eq!(
            compose_turn_message("", Some("ship dark mode")),
            "ship dark mode"
        );
        assert_eq!(
            compose_turn_message("   ", Some("ship dark mode")),
            "ship dark mode"
        );
    }

    #[test]
    fn run_request_text_reads_the_console_payload_and_a_bare_string() {
        assert_eq!(
            run_request_text(&json!({ "request": "dark mode" })).as_deref(),
            Some("dark mode")
        );
        assert_eq!(
            run_request_text(&json!("dark mode")).as_deref(),
            Some("dark mode")
        );
        // Tolerated spellings from a hand-written call or an older client.
        for key in ["input", "topic", "message", "text"] {
            let mut payload = serde_json::Map::new();
            payload.insert(key.to_string(), json!("dark mode"));
            assert_eq!(
                run_request_text(&Value::Object(payload)).as_deref(),
                Some("dark mode"),
                "key {key} should be accepted"
            );
        }
        // Trimmed.
        assert_eq!(
            run_request_text(&json!({ "request": "  dark mode  " })).as_deref(),
            Some("dark mode")
        );
    }

    #[test]
    fn run_request_text_is_none_for_payloads_that_carry_no_topic() {
        // These are the shapes an existing caller already sends — none may start
        // injecting a topic into agent messages.
        for payload in [
            json!({}),
            json!(null),
            json!(42),
            json!({ "request": "" }),
            json!({ "request": "   " }),
            json!({ "unrelated": "value" }),
            json!({ "request": 7 }),
            json!(["dark mode"]),
        ] {
            assert_eq!(
                run_request_text(&payload),
                None,
                "payload {payload} must carry no topic"
            );
        }
    }

    #[test]
    fn message_falls_back_to_serialized_request() {
        // No known string key: fall back to the serialized object.
        let out = message_from_request(&json!({ "agent_ref": "x" }));
        assert!(out.contains("agent_ref"));
    }

    #[test]
    fn workflow_workspace_is_unique_per_run_and_traversal_safe() {
        let root = std::path::Path::new("/tmp/workspaces");
        let company = CompanyId::new("acme");
        let first = workflow_workspace(root, &company, "../billing", "run:1");
        let second = workflow_workspace(root, &company, "../billing", "run:2");

        assert_ne!(first, second);
        assert!(first.starts_with(root.join("acme").join("_workflow")));
        assert!(!first.to_string_lossy().contains("../billing"));
        assert_eq!(
            first.file_name().and_then(|part| part.to_str()),
            Some("workspace")
        );
    }

    /// Issue #499. tinyflows 0.6 added `Capabilities::memory`, and this pins the
    /// answer we gave it.
    ///
    /// `None` is a decision, not an omission — see the comment at the field. A
    /// `MemoryProvider` here would let a workflow read and *write* agent memory
    /// (`remember`/`forget` are on the trait), and which scopes a workflow may
    /// touch is a policy question this repo has not answered. Until it is,
    /// unwired is the honest state: a `memory` node fails with a capability
    /// error rather than quietly writing somewhere nobody authorised.
    ///
    /// So this test is here to make wiring it a *deliberate* act. Whoever
    /// changes it has to change this line too, which is where they will find the
    /// question they need to answer first.
    #[tokio::test]
    async fn the_memory_capability_is_left_unwired_on_purpose() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No endpoint is spawned: `build_capabilities` assembles a struct of
        // handles and never calls the provider, so a base URL that answers
        // nothing is sufficient and keeps this off the network.
        let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
            "http://127.0.0.1:1/unused".to_string(),
            dir.path(),
        );
        let record = crate::workflows::gated_tool_turn_test::record();

        let caps = build_capabilities(
            Arc::new(HarnessPool::new()),
            deps,
            &record,
            RunContext {
                workflow_id: "wf",
                run_id: "run:1",
                run_request: None,
                dry_run: false,
                notices: RunNotices::default(),
            },
        )
        .await;

        assert!(
            caps.memory.is_none(),
            "wiring `Capabilities::memory` gives workflows read AND write access \
             to agent memory — settle which scopes a workflow may touch before \
             changing this, and say so at the field"
        );
        // The neighbouring optional capability IS wired, so this is a statement
        // about `memory` specifically rather than about the bundle being empty.
        assert!(
            caps.agent.is_some(),
            "agent capability should still be wired"
        );
    }

    /// Issue #542 — T9: a dry bundle wires the effect STUBS (agent / tools / http
    /// all echo with the `dry_run` marker) and the inert `NoopState`, while the
    /// read-only resolver stays real. Pinned behaviourally through the marker, so
    /// a future refactor that quietly wired a real effect into a dry bundle fails
    /// here.
    #[tokio::test]
    async fn a_dry_bundle_wires_stubs_and_noop_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (deps, _journal) = crate::workflows::gated_tool_turn_test::deps(
            "http://127.0.0.1:1/unused".to_string(),
            dir.path(),
        );
        let record = crate::workflows::gated_tool_turn_test::record();

        let caps = build_capabilities(
            Arc::new(HarnessPool::new()),
            deps,
            &record,
            RunContext {
                workflow_id: "wf",
                run_id: "run:1",
                run_request: None,
                dry_run: true,
                notices: RunNotices::default(),
            },
        )
        .await;

        // http: the stub echoes without sending, carrying the marker.
        let http_out = caps
            .http
            .request(json!({ "url": "http://127.0.0.1:9/" }), None)
            .await
            .expect("dry http never fails");
        assert_eq!(
            http_out["dry_run"],
            json!(true),
            "http slot should be the dry stub"
        );

        // agent: the stub echoes with no pool routing.
        let agent = caps.agent.as_ref().expect("agent stub is wired");
        let agent_out = agent
            .run_agent("ceo", json!({ "prompt": "hi" }), None)
            .await
            .expect("dry agent never fails");
        assert_eq!(
            agent_out["dry_run"],
            json!(true),
            "agent slot should be the dry stub"
        );

        // state: NoopState — a load reads None and a store is dropped.
        assert_eq!(caps.state.load("k").await.expect("noop load"), None);
        caps.state.store("k", json!(1)).await.expect("noop store");
        assert_eq!(
            caps.state.load("k").await.expect("noop load"),
            None,
            "dry state must be the inert NoopState, never durable"
        );
    }
}
