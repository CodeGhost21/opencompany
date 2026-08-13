//! Tests for the workflow builder pass (issue #580).
//!
//! Two tiers, the same split the planning station uses. The **unit** tier covers
//! the pure decisions — the parse, the graph-vs-not-automatable resolution, the
//! host-assigned id, the spec → `RawWorkflow` conversion — because a wrong answer
//! there is silent. The **pass** tier runs the real [`run_workflow_build_pass`]
//! against a real [`CompanyRuntime`] with a real store and a scripted model,
//! because the things most likely to be wrong — that a proposal lands In Review,
//! that a bad answer returns the card to To-do with no proposal, that the attempt
//! row settles, and that an operator's move wins — are properties of the whole
//! pass.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::json;
use tinyagents::harness::model::{ChatModel, ModelResponse};
use tinyagents::{Result as TaResult, TinyAgentsError};

use super::*;
use crate::company::CompanyManifest;
use crate::ports::runs::{NewRun, RunStatus};
use crate::ports::types::CompanyId;

// ---------------------------------------------------------------------------
// A scripted model
// ---------------------------------------------------------------------------

/// A model that answers with a canned script (or fails), counts its calls, and —
/// optionally — mutates the board mid-call to simulate an operator moving the
/// card out from under the pass.
///
/// The script is a sequence: the Nth call returns the Nth reply, and once the
/// script is exhausted it repeats the last reply. A single-reply model therefore
/// answers every call the same (the card-builder shape), while a multi-reply
/// script drives the create-time copilot's draft→correct loop (issue #813): a
/// `bad → good` script proves the retry recovers, a single `bad` proves a second
/// failure folds to not-automatable.
struct ScriptedModel {
    replies: Vec<String>,
    /// When true the model errors instead of answering — the brain being down.
    fail: bool,
    calls: AtomicUsize,
    /// When set, the model moves the card to To-do on invoke, before answering —
    /// the operator's drag landing while the pass is waiting on the model.
    move_card: StdMutex<Option<(Weak<CompanyRuntime>, String)>>,
}

impl ScriptedModel {
    fn replying(reply: impl Into<String>) -> Arc<Self> {
        Self::scripting(vec![reply.into()])
    }

    /// A model that answers each call with the next reply in `replies`, repeating
    /// the last once the script runs out.
    fn scripting(replies: Vec<String>) -> Arc<Self> {
        assert!(
            !replies.is_empty(),
            "a scripted model needs at least one reply"
        );
        Arc::new(Self {
            replies,
            fail: false,
            calls: AtomicUsize::new(0),
            move_card: StdMutex::new(None),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            replies: Vec::new(),
            fail: true,
            calls: AtomicUsize::new(0),
            move_card: StdMutex::new(None),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    async fn invoke(&self, _state: &(), request: ModelRequest) -> TaResult<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            request.tools.is_empty(),
            "a builder pass must expose NO tools — a tool here is a loop, and a loop is a dispatch"
        );
        // Simulate an operator drag arriving while the model is thinking.
        let mover = self.move_card.lock().unwrap().clone();
        if let Some((weak, task_id)) = mover
            && let Some(runtime) = weak.upgrade()
        {
            let mut card = runtime
                .tasks()
                .list(runtime.id())
                .await
                .unwrap()
                .into_iter()
                .find(|t| t.id == task_id)
                .unwrap();
            card.column = COLUMN_TODO.to_string();
            card.updated_at_millis += 1;
            runtime.tasks().upsert(runtime.id(), &card).await.unwrap();
        }
        if self.fail {
            return Err(TinyAgentsError::Model("the brain is down".to_string()));
        }
        let reply = self.replies[index.min(self.replies.len() - 1)].clone();
        Ok(ModelResponse::assistant(reply))
    }
}

impl HarnessModel for ScriptedModel {
    fn telemetry_provider_id(&self) -> String {
        "managed".to_string()
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const MANIFEST: &str = r#"
[company]
name = "Acme"

[[agent]]
id = "maya"
role = "Writer"
tools = ["docs", "web"]

[policy]
mode = "full"

[tools]
allow = ["docs", "web"]
"#;

fn manifest() -> CompanyManifest {
    toml::from_str(MANIFEST).expect("the fixture manifest parses")
}

/// A valid answer: a two-node scheduled graph whose agent is on the roster.
const VALID_GRAPH: &str = r#"```json
{
  "automatable": true,
  "summary": "Email the weekly digest every Monday",
  "workflow": {
    "id": "the-model-should-not-pick-this",
    "name": "Weekly digest",
    "description": "Draft and send the weekly digest.",
    "nodes": [
      { "id": "start", "kind": "trigger", "name": "Every Monday", "schedule": "0 9 * * 1" },
      { "id": "draft", "kind": "agent", "name": "Draft it", "agent": "maya", "summary": "write the digest" }
    ],
    "edges": [{ "from": "start", "to": "draft" }]
  }
}
```"#;

// ---------------------------------------------------------------------------
// Unit tier
// ---------------------------------------------------------------------------

/// Fenced or narrated JSON both parse; prose is a failure, not a guess.
#[test]
fn a_fenced_or_narrated_answer_parses_and_prose_does_not() {
    let fenced = parse_draft(VALID_GRAPH).expect("a fenced answer parses");
    assert_eq!(fenced.automatable, Some(true));
    let narrated = parse_draft("Sure!\n{\"automatable\":false,\"reason\":\"one-off\"}\nok")
        .expect("a narrated answer parses");
    assert_eq!(narrated.reason, "one-off");

    assert!(parse_draft("I think we should just do it once.").is_none());
    assert!(parse_draft("").is_none());
    assert!(parse_draft("}{").is_none());
}

/// A graph present (and not explicitly refused) is a graph to build; anything
/// else — an explicit `automatable:false`, an empty graph, a missing one — is
/// not-automatable, carrying the model's reason when it gave one.
#[test]
fn the_outcome_resolves_graph_versus_not_automatable() {
    let graph = parse_draft(VALID_GRAPH).unwrap().into_outcome();
    match graph {
        BuildOutcome::Graph { summary, spec } => {
            assert!(summary.contains("digest"));
            assert_eq!(spec.nodes.len(), 2);
        }
        BuildOutcome::NotAutomatable(_) => panic!("a valid graph must build"),
    }

    let refused = parse_draft(r#"{"automatable":false,"reason":"only runs once"}"#)
        .unwrap()
        .into_outcome();
    assert!(matches!(refused, BuildOutcome::NotAutomatable(r) if r == "only runs once"));

    // A graph AND an explicit no → the model is taken at its explicit word.
    let both =
        parse_draft(r#"{"automatable":false,"workflow":{"nodes":[{"id":"t","kind":"trigger"}]}}"#)
            .unwrap()
            .into_outcome();
    assert!(matches!(both, BuildOutcome::NotAutomatable(_)));

    // No workflow, no reason → a truthful default reason rather than an empty one.
    let empty = parse_draft(r#"{"automatable":true}"#)
        .unwrap()
        .into_outcome();
    assert!(matches!(empty, BuildOutcome::NotAutomatable(r) if !r.is_empty()));
}

/// The host assigns a safe, unique id — slugged from the name, deduped, and
/// never the model's — so the model can never doom a proposal with a colliding
/// or unsafe stem.
#[test]
fn the_host_assigns_a_safe_unique_id() {
    let mut existing = HashSet::new();
    assert_eq!(
        safe_workflow_id("Weekly Digest!", "card", &existing),
        "weekly-digest"
    );
    existing.insert("weekly-digest".to_string());
    assert_eq!(
        safe_workflow_id("Weekly Digest!", "card", &existing),
        "weekly-digest-2"
    );
    // An empty/symbol-only name falls back to the card title, then to a constant.
    assert_eq!(safe_workflow_id("", "My Card", &existing), "my-card");
    assert_eq!(safe_workflow_id("!!!", "!!!", &existing), "workflow");
}

/// A large plan is bounded before it reaches the prompt: the step and
/// prerequisite counts are capped and each step's free text is truncated, so an
/// oversized plan can't run up the input tokens the pass meters (issue #580).
#[test]
fn a_large_plan_is_bounded_for_the_prompt() {
    use crate::ports::tasks::TaskPlan;
    let steps: Vec<_> = (0..40)
        .map(|_| serde_json::json!({ "title": "t".repeat(600), "detail": "d".repeat(600) }))
        .collect();
    let prereqs: Vec<_> = (0..40)
        .map(|_| serde_json::json!({ "kind": "connection", "name": "gh", "status": "satisfied", "note": "" }))
        .collect();
    let plan: TaskPlan = serde_json::from_value(serde_json::json!({
        "description": "x".repeat(2_000),
        "steps": steps,
        "prerequisites": prereqs,
        "risks": [],
        "verification": "v".repeat(2_000),
        "scope": "everything",
        "plannedAtMillis": 1,
    }))
    .unwrap();

    let bounded = bounded_plan(plan);
    assert_eq!(bounded.steps.len(), MAX_PLAN_STEPS, "step count is capped");
    assert_eq!(
        bounded.prerequisites.len(),
        MAX_PLAN_PREREQS,
        "prerequisite count is capped"
    );
    // `cap` appends a one-char ellipsis when it truncates.
    assert!(bounded.steps[0].detail.chars().count() <= MAX_STEP_DETAIL_CHARS + 1);
    assert!(bounded.steps[0].title.chars().count() <= MAX_SUMMARY_CHARS + 1);
    assert!(bounded.description.chars().count() <= MAX_REASON_CHARS + 1);
    assert!(bounded.verification.chars().count() <= MAX_REASON_CHARS + 1);
}

/// The host dedups the name like the id, case-insensitively (matching the create
/// path's uniqueness check), so a clash settles here instead of at apply.
#[test]
fn the_host_dedups_a_clashing_name() {
    let existing = vec!["Weekly Digest".to_string()];
    // A fresh name is returned untouched.
    assert_eq!(
        safe_workflow_name("Monthly Report", &existing),
        "Monthly Report"
    );
    // A clash — case-insensitively — gets a suffix that clears the check.
    assert_eq!(
        safe_workflow_name("weekly digest", &existing),
        "weekly digest 2"
    );
    // The next suffix skips a taken one.
    let existing = vec!["Digest".to_string(), "Digest 2".to_string()];
    assert_eq!(safe_workflow_name("Digest", &existing), "Digest 3");
    // An empty name is left for the create path to refuse on its own terms.
    assert_eq!(safe_workflow_name("   ", &existing), "   ");
}

/// The stored `ops` round-trips to the exact `RawWorkflow` the create path will
/// see — the host-authority conversion, with the model's config JSON becoming
/// node config.
#[test]
fn a_spec_rebuilds_the_expected_raw_workflow() {
    let spec: WorkflowGraphSpec = serde_json::from_value(serde_json::json!({
        "id": "weekly-digest",
        "name": "Weekly digest",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * 1" },
            { "id": "draft", "kind": "agent", "name": "Draft", "agent": "maya" }
        ],
        "edges": [{ "from": "start", "to": "draft" }]
    }))
    .unwrap();
    let raw = raw_workflow_from_spec(&spec).expect("a well-formed spec converts");
    assert_eq!(raw.id, "weekly-digest");
    assert_eq!(raw.nodes.len(), 2);
    assert_eq!(raw.nodes[0].schedule.as_deref(), Some("0 9 * * 1"));
    assert_eq!(raw.nodes[1].agent.as_deref(), Some("maya"));
    assert_eq!(raw.edges.len(), 1);
}

// ---------------------------------------------------------------------------
// Pass tier
// ---------------------------------------------------------------------------

async fn runtime_with(model: Arc<ScriptedModel>) -> (tempfile::TempDir, Arc<CompanyRuntime>) {
    let home = tempfile::Builder::new()
        .prefix("opencompany-builder-")
        .tempdir()
        .expect("tempdir");
    let mut runtime = crate::runtime::RuntimeBuilder::new(home.path().to_path_buf(), manifest())
        .with_id(CompanyId::new("acme"))
        .build()
        .await
        .expect("runtime");
    runtime.set_builder(Arc::new(WorkflowBuilder::new(model, "chat-v1")));
    (home, Arc::new(runtime))
}

/// A `workflow`-deliverable card sitting In Progress, with an optional plan.
fn card(id: &str, plan: Option<crate::ports::tasks::TaskPlan>) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        title: "Automate the weekly digest".to_string(),
        note: Some("It should go out every Monday morning.".to_string()),
        column: COLUMN_IN_PROGRESS.to_string(),
        priority: "medium".to_string(),
        assignee: "maya".to_string(),
        updated_at_millis: 7,
        origin_chat_id: None,
        parent_task_id: None,
        output: None,
        plan,
        deliverable: TaskDeliverable::Workflow,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
    }
}

async fn read(runtime: &Arc<CompanyRuntime>, id: &str) -> TaskRecord {
    runtime
        .tasks()
        .list(runtime.id())
        .await
        .expect("board")
        .into_iter()
        .find(|t| t.id == id)
        .expect("the card exists")
}

/// Mints the attempt row the dispatch edge would, so the test can read its
/// settle status back.
async fn open_run(runtime: &Arc<CompanyRuntime>, task_id: &str) -> String {
    runtime
        .runs()
        .create_run(
            runtime.id(),
            NewRun {
                id: crate::ports::generate_id(),
                task_id: task_id.to_string(),
                agent_id: "maya".to_string(),
            },
        )
        .await
        .expect("mint the attempt row")
        .id
}

async fn run_status(runtime: &Arc<CompanyRuntime>, run_id: &str) -> RunStatus {
    runtime
        .runs()
        .get_run(runtime.id(), run_id)
        .await
        .expect("read")
        .expect("the attempt row exists")
        .status
}

/// The happy path: a valid graph lands a proposal In Review, the card carries it,
/// the attempt settles Succeeded, and the stored `ops` carries the host-assigned
/// id (not the model's) and the schedule.
#[tokio::test]
async fn a_valid_graph_lands_a_proposal_in_review() {
    let model = ScriptedModel::replying(VALID_GRAPH);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-1", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-1").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-1".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-1").await;
    assert_eq!(after.column, COLUMN_IN_REVIEW);
    let proposal = after
        .workflow_proposal
        .expect("the proposal is on the card");
    assert!(proposal.summary.contains("digest"));
    assert_eq!(
        proposal.run_id, run_id,
        "the proposal links to the build attempt"
    );
    // The host owns the id; the model's suggestion is ignored, and the schedule
    // survives in the stored ops the apply route will rebuild from.
    let spec: WorkflowGraphSpec = serde_json::from_value(proposal.ops).unwrap();
    assert_eq!(spec.id, "weekly-digest");
    assert_eq!(spec.nodes[0].schedule.as_deref(), Some("0 9 * * 1"));
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Succeeded);
    assert_eq!(model.calls(), 1, "one card, one model call");
}

/// A card with no plan still builds — from its title and note.
#[tokio::test]
async fn a_card_with_no_plan_builds_from_title_and_note() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying(VALID_GRAPH)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-2", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-2").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-2".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-2").await;
    assert_eq!(after.column, COLUMN_IN_REVIEW);
    assert!(after.workflow_proposal.is_some());
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Succeeded);
}

/// A not-automatable answer returns the card to To-do with the reason and no
/// proposal (decision D2c); the attempt settles Failed.
#[tokio::test]
async fn a_not_automatable_answer_returns_the_card_to_todo() {
    let reply = r#"{"automatable":false,"reason":"this only ever runs once"}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-3", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-3").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-3".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-3").await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(
        after.workflow_proposal.is_none(),
        "no proposal on a not-automatable card"
    );
    assert!(after.note.unwrap().contains("done once"));
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Failed);
}

/// An unparseable answer returns the card to To-do with no proposal; the attempt
/// settles Failed. Prose or nothing — a graph guessed from prose is exactly the
/// broken graph.
#[tokio::test]
async fn an_unparseable_answer_returns_to_todo_with_no_proposal() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying("I'd just do this by hand.")).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-4", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-4").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-4".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-4").await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(after.workflow_proposal.is_none());
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Failed);
}

/// A model error returns the card to To-do and settles the attempt Failed —
/// building could not reach the model, so nothing was proposed.
#[tokio::test]
async fn a_model_error_returns_to_todo() {
    let (_home, runtime) = runtime_with(ScriptedModel::failing()).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-5", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-5").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-5".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-5").await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(after.workflow_proposal.is_none());
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Failed);
}

/// A graph the create path would refuse — an `agent` node naming a teammate not
/// on the roster — never reaches In Review: the courtesy validation catches it,
/// the card returns to To-do naming the error, no proposal, attempt Failed.
#[tokio::test]
async fn a_graph_that_would_be_refused_never_reaches_in_review() {
    let reply = r#"{"automatable":true,"summary":"do it","workflow":{"name":"Bad",
        "nodes":[{"id":"start","kind":"trigger","name":"Start"},
                 {"id":"draft","kind":"agent","name":"Draft","agent":"ghost"}],
        "edges":[{"from":"start","to":"draft"}]}}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-6", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-6").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-6".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-6").await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(
        after.workflow_proposal.is_none(),
        "a doomed proposal must not reach In Review"
    );
    assert!(
        after.note.unwrap().contains("ghost"),
        "the roster error is named on the card"
    );
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Failed);
}

/// A graph carrying a node kind outside `BUILDER_NODE_KINDS` — one the prompt
/// never taught the model to shape and nothing downstream validates (here an
/// `http_request` with an attacker-influenceable URL) — settles to To-do before
/// the courtesy pass, with no proposal and the attempt Failed. The host owns the
/// kind vocabulary, not the model (issue #580).
#[tokio::test]
async fn an_out_of_vocabulary_kind_settles_to_todo() {
    let reply = r#"{"automatable":true,"summary":"call a url","workflow":{"name":"Reach out",
        "nodes":[{"id":"start","kind":"trigger","name":"Start"},
                 {"id":"call","kind":"http_request","name":"Call",
                  "config":{"url":"http://attacker.example/x","method":"GET"}}],
        "edges":[{"from":"start","to":"call"}]}}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-9", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-9").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-9".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-9").await;
    assert_eq!(after.column, COLUMN_TODO);
    assert!(
        after.workflow_proposal.is_none(),
        "an unsupported kind must not reach In Review"
    );
    assert!(
        after.note.unwrap().contains("http_request"),
        "the offending kind is named on the card"
    );
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Failed);
}

/// The model does not get a vote on approval gating: whatever `requires_approval`
/// it emits — `true` on one node, `false` on another — the host strips before the
/// proposal is stored, so a builder-authored node inherits the platform default
/// (#460) rather than the model's choice. Both are dropped in the stored `ops`.
#[tokio::test]
async fn the_host_strips_model_chosen_approval_gating() {
    let reply = r#"{"automatable":true,"summary":"gated","workflow":{"name":"Gated",
        "nodes":[{"id":"start","kind":"trigger","name":"Start","requires_approval":true},
                 {"id":"draft","kind":"agent","name":"Draft","agent":"maya","requires_approval":false}],
        "edges":[{"from":"start","to":"draft"}]}}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-approval", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-approval").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-approval".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-approval").await;
    assert_eq!(after.column, COLUMN_IN_REVIEW);
    let spec: WorkflowGraphSpec =
        serde_json::from_value(after.workflow_proposal.expect("proposal").ops).unwrap();
    assert!(
        spec.nodes.iter().all(|n| n.requires_approval.is_none()),
        "the host drops the model's requires_approval on every node (true and false alike)"
    );
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Succeeded);
}

/// An operator moving the card out from under the pass wins: the pass discards
/// its result (no proposal, the card stays where the operator put it) and the
/// attempt settles Cancelled — the tokens stay metered because they were spent.
#[tokio::test]
async fn an_operator_move_mid_build_is_discarded() {
    let model = ScriptedModel::replying(VALID_GRAPH);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    runtime
        .tasks()
        .upsert(runtime.id(), &card("t-7", None))
        .await
        .unwrap();
    let run_id = open_run(&runtime, "t-7").await;
    // The model moves the card to To-do while it "thinks".
    *model.move_card.lock().unwrap() = Some((Arc::downgrade(&runtime), "t-7".to_string()));

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-7".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-7").await;
    assert_eq!(after.column, COLUMN_TODO, "the operator's move wins");
    assert!(
        after.workflow_proposal.is_none(),
        "the pass's result is discarded"
    );
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Cancelled);
}

// ---------------------------------------------------------------------------
// Create-time copilot (issue #753)
// ---------------------------------------------------------------------------

/// A drafter answer whose graph carries a model-chosen id and per-node approval
/// gating — both of which the host overrides — plus a real roster agent.
const DESC_GRAPH: &str = r#"{"automatable":true,"summary":"email the weekly digest",
    "workflow":{"id":"the-model-should-not-pick-this","name":"Weekly digest",
        "nodes":[{"id":"start","kind":"trigger","name":"Every Monday","schedule":"0 9 * * 1","requires_approval":true},
                 {"id":"draft","kind":"agent","name":"Draft","agent":"maya","requires_approval":false}],
        "edges":[{"from":"start","to":"draft"}]}}"#;

/// Seeds a real overlay workflow through the create path, so the drafter's host
/// authority has something to dedup an id and name against.
async fn seed_workflow(runtime: &Arc<CompanyRuntime>, id: &str, name: &str) {
    let spec: WorkflowGraphSpec = serde_json::from_value(serde_json::json!({
        "id": id,
        "name": name,
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [{ "from": "start", "to": "done" }]
    }))
    .unwrap();
    let raw = raw_workflow_from_spec(&spec).unwrap();
    crate::company::create_company_workflow(
        runtime.id(),
        runtime.source_dir(),
        runtime.store(),
        Some(runtime.events()),
        raw,
    )
    .await
    .expect("seed workflow");
}

/// The happy path: an operator's description drafts a graph, and the host owns
/// the id, the display name and the approval gating — the model's choices for
/// all three are overridden — while the schedule it authored survives. The
/// drafted id/name dedup against a workflow that already exists.
#[tokio::test]
async fn a_description_drafts_a_graph_under_host_authority() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying(DESC_GRAPH)).await;
    seed_workflow(&runtime, "weekly-digest", "Weekly digest").await;

    let outcome = draft_workflow_from_description(&runtime, "email the weekly digest every Monday")
        .await
        .expect("the drafter runs");
    let (summary, spec) = match outcome {
        DescriptionDraftOutcome::Graph { summary, spec, .. } => (summary, spec),
        DescriptionDraftOutcome::NotAutomatable(reason) => panic!("expected a graph: {reason}"),
    };
    assert!(summary.contains("digest"));
    // The host mints the id (ignoring the model's) and dedups it against the seed.
    assert_eq!(spec.id, "weekly-digest-2");
    // …and the display name, case-insensitively, the same way.
    assert_eq!(spec.name, "Weekly digest 2");
    // Approval gating is the host's: both the model's `true` and `false` are gone.
    assert!(spec.nodes.iter().all(|n| n.requires_approval.is_none()));
    // The schedule the model authored survives into the drafted spec.
    assert_eq!(spec.nodes[0].schedule.as_deref(), Some("0 9 * * 1"));
}

/// A not-automatable answer passes its reason straight through — the copilot
/// never forces a workflow the model judged a one-off.
#[tokio::test]
async fn a_not_automatable_description_passes_the_reason_through() {
    let reply = r#"{"automatable":false,"reason":"this only ever runs once"}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    let outcome = draft_workflow_from_description(&runtime, "do a one-off thing")
        .await
        .unwrap();
    match outcome {
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            assert!(reason.contains("done once"), "reason: {reason}");
        }
        DescriptionDraftOutcome::Graph { .. } => panic!("expected not-automatable"),
    }
}

/// An unparseable model answer folds to not-automatable rather than a 500 — a
/// graph guessed from prose is exactly the broken graph.
#[tokio::test]
async fn an_unparseable_description_answer_is_not_automatable() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying("I'd just do this by hand.")).await;
    let outcome = draft_workflow_from_description(&runtime, "x")
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        DescriptionDraftOutcome::NotAutomatable(_)
    ));
}

/// A node kind outside `DESCRIPTION_NODE_KINDS` — one the copilot prompt never
/// taught (`http_request`) — is refused by name before the courtesy pass, the
/// same host-owns-the-vocabulary rule the card builder applies.
#[tokio::test]
async fn a_description_kind_outside_the_vocabulary_is_refused_by_name() {
    let reply = r#"{"automatable":true,"summary":"call a url","workflow":{"name":"Reach out",
        "nodes":[{"id":"start","kind":"trigger","name":"Start"},
                 {"id":"call","kind":"http_request","name":"Call",
                  "config":{"url":"http://attacker.example/x","method":"GET"}}],
        "edges":[{"from":"start","to":"call"}]}}"#;
    let (_home, runtime) = runtime_with(ScriptedModel::replying(reply)).await;
    let outcome = draft_workflow_from_description(&runtime, "call a url")
        .await
        .unwrap();
    match outcome {
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            assert!(reason.contains("http_request"), "reason: {reason}");
        }
        DescriptionDraftOutcome::Graph { .. } => panic!("http_request must be refused"),
    }
}

/// A `tool_call` node whose slug the company grants drafts cleanly; one whose
/// namespace is ungranted is refused with `validate_tool_call_node`'s own
/// message, so the copilot's grounding and courtesy validation stay in lockstep.
/// The fixture grants `web` (not `code`), so `web_fetch` passes and `csv_export`
/// does not.
#[tokio::test]
async fn a_granted_tool_call_drafts_and_an_ungranted_one_is_refused() {
    let granted = r#"{"automatable":true,"summary":"fetch a page","workflow":{"name":"Fetcher",
        "nodes":[{"id":"start","kind":"trigger","name":"Start"},
                 {"id":"fetch","kind":"tool_call","name":"Fetch",
                  "config":{"slug":"web_fetch","args":{"url":"https://example.com"}}}],
        "edges":[{"from":"start","to":"fetch"}]}}"#;
    let (_h1, runtime) = runtime_with(ScriptedModel::replying(granted)).await;
    let outcome = draft_workflow_from_description(&runtime, "fetch a page")
        .await
        .unwrap();
    match outcome {
        DescriptionDraftOutcome::Graph { spec, .. } => {
            assert!(spec.nodes.iter().any(|n| n.kind == "tool_call"));
        }
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            panic!("a granted tool_call must draft: {reason}")
        }
    }

    let ungranted = r#"{"automatable":true,"summary":"export rows","workflow":{"name":"Exporter",
        "nodes":[{"id":"start","kind":"trigger","name":"Start"},
                 {"id":"exp","kind":"tool_call","name":"Export","config":{"slug":"csv_export"}}],
        "edges":[{"from":"start","to":"exp"}]}}"#;
    let (_h2, runtime2) = runtime_with(ScriptedModel::replying(ungranted)).await;
    let outcome2 = draft_workflow_from_description(&runtime2, "export the rows")
        .await
        .unwrap();
    match outcome2 {
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            assert!(reason.contains("does not grant"), "reason: {reason}");
        }
        DescriptionDraftOutcome::Graph { .. } => panic!("an ungranted tool_call must be refused"),
    }
}

/// The prompt grounds the model in the company's real state: the operator's
/// description verbatim, the roster ids, the existing workflow names, and the
/// granted tool slugs — and the system prompt carries the `tool_call` rule the
/// card builder has no need for.
#[tokio::test]
async fn the_description_prompt_renders_the_company_state_verbatim() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying(DESC_GRAPH)).await;
    seed_workflow(&runtime, "existing-one", "Existing One").await;
    let company = gather_company_evidence(&runtime).await.unwrap();
    let slugs = crate::company::workflow_callable_tool_slugs(&company.record);

    let description = "email the weekly digest every Monday morning";
    let prompt = description_evidence_prompt(&company, &slugs, &[], description);
    assert!(
        prompt.contains(description),
        "the description appears verbatim"
    );
    assert!(prompt.contains("`maya`"), "the roster id is rendered");
    assert!(
        prompt.contains("Existing One"),
        "existing names are rendered"
    );
    assert!(
        prompt.contains("`web_fetch`"),
        "granted tool slugs are rendered: {prompt}"
    );
    // Issue #813: the tool line carries the honest capability + required args, not
    // a bare slug — so the model does not reach for a tool that cannot do the job.
    assert!(
        prompt.contains("cannot search for a URL"),
        "the web_fetch capability line is rendered: {prompt}"
    );
    assert!(
        prompt.contains("(args: url)"),
        "web_fetch's required arg is rendered: {prompt}"
    );
    let system = description_system_prompt();
    assert!(
        system.contains("config.slug"),
        "the copilot system prompt teaches the tool_call rule"
    );
    // Issue #813: the system prompt states the delivery invariant and shows an
    // `output` node with a destination in the schema example.
    assert!(
        system.contains("an `agent` node cannot send"),
        "the delivery invariant is stated: {system}"
    );
    assert!(
        system.contains("\"kind\": \"output\"") && system.contains("\"destination\""),
        "the schema example includes an output node with a destination: {system}"
    );
    assert!(
        system.contains("copied EXACTLY"),
        "the roster-copy rule is stated: {system}"
    );
}

#[tokio::test]
async fn the_description_prompt_excludes_capability_filtered_tools() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying(DESC_GRAPH)).await;
    let company = gather_company_evidence(&runtime).await.unwrap();
    let mut record = company.record.clone();
    record.manifest.tools.allow.push("search".to_string());
    // This is the resolved result a capability plan supplies to the live
    // wiring resolver: a plan that filters `web` must remove every web slug
    // from prompt grounding, even when the company grants the namespace.
    let capability_filter =
        crate::harness::toolbelt::CapabilityFilter::DenyNamespaces(["web"].into_iter().collect());
    let wired: std::collections::BTreeSet<&'static str> =
        crate::workflows::caps::WORKFLOW_TOOL_NAMESPACES
            .into_iter()
            .filter(|namespace| {
                !matches!(
                    &capability_filter,
                    crate::harness::toolbelt::CapabilityFilter::DenyNamespaces(denied)
                        if denied.contains(namespace)
                )
            })
            .collect();
    let effective = crate::company::workflow_effective_tool_slugs(&record, Some(&wired));
    let unwired = crate::company::workflow_granted_but_unwired_tool_slugs(&record, Some(&wired));
    assert!(!effective.iter().any(|slug| slug == "web_fetch"));
    assert!(effective.iter().any(|slug| slug == "web_search"));
    assert!(unwired.iter().any(|slug| slug == "web_fetch"));

    let evidence = CompanyEvidence { record, ..company };
    let prompt = description_evidence_prompt(&evidence, &effective, &unwired, "search the web");
    assert!(!prompt.contains("web_fetch —"));
    assert!(prompt.contains("granted but not wired"));
    assert!(prompt.contains("web_fetch"));
    assert!(prompt.contains("if the task needs one, say so"));
}

/// A company with no teammates and no granted tools renders the guiding lines
/// that keep the model from authoring an `agent` or `tool_call` node it cannot
/// ground.
#[tokio::test]
async fn the_description_prompt_names_an_empty_roster_and_toolset() {
    let (_home, runtime) = runtime_with(ScriptedModel::replying(DESC_GRAPH)).await;
    let base = gather_company_evidence(&runtime).await.unwrap();
    // Reuse the gathered record but blank the roster / names for the render.
    let empty = CompanyEvidence {
        roster: Vec::new(),
        existing_names: Vec::new(),
        existing_ids: HashSet::new(),
        ..base
    };
    let prompt = description_evidence_prompt(&empty, &[], &[], "do the thing");
    assert!(prompt.contains("no teammates"), "{prompt}");
    assert!(prompt.contains("no callable tools are wired"), "{prompt}");
    assert!(prompt.contains("(none yet)"), "{prompt}");
}

// ---------------------------------------------------------------------------
// Grounding & gates (issue #813) — unit tier over the pure helpers
// ---------------------------------------------------------------------------

/// A roster teammate for the pure-helper units.
fn roster_entry(id: &str, role: &str, name: Option<&str>) -> RosterEntry {
    RosterEntry {
        id: id.to_string(),
        role: role.to_string(),
        name: name.map(str::to_string),
        description: None,
    }
}

/// A `WorkflowGraphSpec` from a JSON literal.
fn spec_from(value: serde_json::Value) -> WorkflowGraphSpec {
    serde_json::from_value(value).expect("the spec parses")
}

/// The normalizer collapses `-`, `_` and whitespace runs so a role, an id and a
/// name spelled three ways compare equal — and nothing fuzzier.
#[test]
fn normalize_label_collapses_separators_only() {
    assert_eq!(normalize_label("QA Engineer"), "qa engineer");
    assert_eq!(normalize_label("qa_engineer"), "qa engineer");
    assert_eq!(normalize_label("  Qa--Engineer  "), "qa engineer");
    // Different words never collapse together.
    assert_ne!(normalize_label("writer"), normalize_label("rewriter"));
}

/// Delivery detection stays conservative: a request to deliver to the operator
/// or a concrete target is a signal; the business activity "we email customers"
/// is not (no address, no #channel, no verb aimed at the operator).
#[test]
fn delivery_signals_are_conservative() {
    assert!(!delivery_signals("email me the digest every monday").is_empty());
    assert!(!delivery_signals("post the summary to #ops").is_empty());
    assert!(!delivery_signals("send the report to jo@acme.com").is_empty());
    assert!(delivery_signals("we email customers a weekly newsletter").is_empty());
    assert!(delivery_signals("summarize the week's work").is_empty());
    // A numeric ticket/issue reference is not a #channel (leading-digit guard).
    assert!(delivery_signals("summarize ticket #4521 each friday").is_empty());
    // "send used" is not the whole word "send us" (whole-word verb match).
    assert!(delivery_signals("send used parts to the warehouse").is_empty());
}

/// (a) The resolver rewrites a role-named agent to its roster id and records a
/// note; the rewrite is exact-normalized, never fuzzy.
#[test]
fn the_resolver_rewrites_a_role_named_agent_and_notes_it() {
    let roster = vec![roster_entry("qa_engineer", "QA Engineer", None)];
    let mut spec = spec_from(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "T" },
            { "id": "a", "kind": "agent", "name": "Test it", "agent": "QA Engineer" }
        ],
        "edges": []
    }));
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    resolve_agent_ids(&mut spec, &roster, &mut notes, &mut errors);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(spec.nodes[1].agent.as_deref(), Some("qa_engineer"));
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("qa_engineer"), "{notes:?}");
}

/// (a) An agent id that matches nothing on the roster is a gate error that NAMES
/// the roster ids — proving the old silent fold (issue #813) is dead. An already
/// valid id is left untouched.
#[test]
fn the_resolver_names_the_roster_on_an_unknown_agent() {
    let roster = vec![
        roster_entry("qa_engineer", "QA Engineer", None),
        roster_entry("ceo", "Chief Executive", None),
    ];
    let mut spec = spec_from(json!({
        "nodes": [
            { "id": "a", "kind": "agent", "name": "X", "agent": "019fcbc3cb55-nope" },
            { "id": "b", "kind": "agent", "name": "Y", "agent": "ceo" }
        ],
        "edges": []
    }));
    let mut notes = Vec::new();
    let mut errors = Vec::new();
    resolve_agent_ids(&mut spec, &roster, &mut notes, &mut errors);
    assert_eq!(errors.len(), 1, "only the unknown agent errors: {errors:?}");
    assert!(
        errors[0].contains("qa_engineer") && errors[0].contains("ceo"),
        "the roster is named so the model can self-correct: {errors:?}"
    );
    // The already-valid `ceo` node is untouched.
    assert_eq!(spec.nodes[1].agent.as_deref(), Some("ceo"));
}

/// (b) The delivery gate fires when a delivery is asked for and no `output` node
/// carries a destination; it is silent with one, and silent absent a signal.
#[test]
fn the_delivery_gate_fires_only_when_delivery_is_asked_and_missing() {
    let no_output = spec_from(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "T" },
            { "id": "a", "kind": "agent", "name": "Draft", "agent": "maya" }
        ],
        "edges": []
    }));
    let mut fired = Vec::new();
    delivery_gate(&no_output, "email me the digest", &mut fired);
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert!(fired[0].contains("output"), "{fired:?}");

    let with_output = spec_from(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "T" },
            { "id": "o", "kind": "output", "name": "Send", "destination": { "kind": "owner" } }
        ],
        "edges": []
    }));
    let mut satisfied = Vec::new();
    delivery_gate(&with_output, "email me the digest", &mut satisfied);
    assert!(satisfied.is_empty(), "{satisfied:?}");

    let mut no_signal = Vec::new();
    delivery_gate(&no_output, "summarize the week", &mut no_signal);
    assert!(no_signal.is_empty(), "{no_signal:?}");
}

/// (c) The wrong-but-real arm flags a draft that uses a real teammate the request
/// did not name while ignoring the one it did; it is silent when the draft uses
/// the named teammate.
#[test]
fn the_wrong_but_real_arm_flags_the_unnamed_teammate() {
    let roster = vec![
        roster_entry("qa_engineer", "QA Engineer", None),
        roster_entry("ceo", "Chief Executive", None),
    ];
    let uses_ceo = spec_from(json!({
        "nodes": [{ "id": "a", "kind": "agent", "name": "Do it", "agent": "ceo" }],
        "edges": []
    }));
    let mut errors = Vec::new();
    wrong_but_real_agent_gate(
        &uses_ceo,
        "have the qa engineer run the tests",
        &roster,
        &mut errors,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("qa_engineer"), "{errors:?}");

    let uses_qa = spec_from(json!({
        "nodes": [{ "id": "a", "kind": "agent", "name": "Do it", "agent": "qa_engineer" }],
        "edges": []
    }));
    let mut ok = Vec::new();
    wrong_but_real_agent_gate(
        &uses_qa,
        "have the qa engineer run the tests",
        &roster,
        &mut ok,
    );
    assert!(ok.is_empty(), "{ok:?}");
}

// ---------------------------------------------------------------------------
// Grounding, gates & the retry loop (issue #813) — pass tier
// ---------------------------------------------------------------------------

/// A description that names a teammate by ROLE drafts a graph with the resolver's
/// id rewrite and a note explaining it — one model call, no retry (the fixture
/// roster has `maya`, role "Writer").
#[tokio::test]
async fn a_role_named_agent_is_resolved_end_to_end_with_a_note() {
    let reply = r#"{"automatable":true,"summary":"draft it","workflow":{"name":"Draft",
        "nodes":[{"id":"t","kind":"trigger","name":"Start"},
                 {"id":"a","kind":"agent","name":"Write","agent":"Writer"}],
        "edges":[{"from":"t","to":"a"}]}}"#;
    let model = ScriptedModel::replying(reply);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let outcome = draft_workflow_from_description(&runtime, "have the writer draft an update")
        .await
        .unwrap();
    match outcome {
        DescriptionDraftOutcome::Graph { spec, notes, .. } => {
            assert_eq!(spec.nodes[1].agent.as_deref(), Some("maya"));
            assert!(notes.iter().any(|n| n.contains("maya")), "{notes:?}");
        }
        DescriptionDraftOutcome::NotAutomatable(reason) => panic!("expected a graph: {reason}"),
    }
    assert_eq!(model.calls(), 1, "a clean resolve needs no retry");
}

/// An agent id that resolves to nothing folds — after one corrective retry — to
/// not-automatable whose reason NAMES the roster (proving the silent fold is
/// gone). Two model calls, both metered.
#[tokio::test]
async fn an_unresolvable_agent_folds_to_not_automatable_naming_the_roster() {
    let reply = r#"{"automatable":true,"summary":"x","workflow":{"name":"Bad",
        "nodes":[{"id":"t","kind":"trigger","name":"Start"},
                 {"id":"a","kind":"agent","name":"Do","agent":"019fcbc3cb55-nope"}],
        "edges":[{"from":"t","to":"a"}]}}"#;
    let model = ScriptedModel::replying(reply);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let outcome = draft_workflow_from_description(&runtime, "do the thing")
        .await
        .unwrap();
    match outcome {
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            assert!(reason.contains("maya"), "the roster is named: {reason}");
        }
        DescriptionDraftOutcome::Graph { .. } => panic!("an unknown agent must not draft"),
    }
    assert_eq!(
        model.calls(),
        2,
        "one draft, one corrective retry — both metered"
    );
}

/// The draft→correct loop recovers: a first answer that fails the roster gate,
/// then a good one, yields a graph — two model calls (both metered).
#[tokio::test]
async fn the_retry_recovers_from_a_correctable_first_answer() {
    let bad = r#"{"automatable":true,"summary":"x","workflow":{"name":"Bad",
        "nodes":[{"id":"t","kind":"trigger","name":"Start"},
                 {"id":"a","kind":"agent","name":"Do","agent":"ghost"}],
        "edges":[{"from":"t","to":"a"}]}}"#;
    let good = r#"{"automatable":true,"summary":"fixed","workflow":{"name":"Good",
        "nodes":[{"id":"t","kind":"trigger","name":"Start"},
                 {"id":"a","kind":"agent","name":"Do","agent":"maya"}],
        "edges":[{"from":"t","to":"a"}]}}"#;
    let model = ScriptedModel::scripting(vec![bad.to_string(), good.to_string()]);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let outcome = draft_workflow_from_description(&runtime, "do the thing")
        .await
        .unwrap();
    assert!(
        matches!(outcome, DescriptionDraftOutcome::Graph { .. }),
        "the retry recovers a correctable draft"
    );
    assert_eq!(
        model.calls(),
        2,
        "one bad draft, one good retry — both metered"
    );
}

/// The delivery gate is enforced end-to-end: an "email me" request whose draft
/// tries to deliver from an agent summary (no output node) is refused, and — bad
/// on both attempts — folds to not-automatable naming the missing output node.
#[tokio::test]
async fn the_delivery_gate_is_enforced_end_to_end() {
    let reply = r#"{"automatable":true,"summary":"digest","workflow":{"name":"Digest",
        "nodes":[{"id":"t","kind":"trigger","name":"Monday","schedule":"0 9 * * 1"},
                 {"id":"a","kind":"agent","name":"Draft and email","agent":"maya",
                  "summary":"email the digest to the owner"}],
        "edges":[{"from":"t","to":"a"}]}}"#;
    let model = ScriptedModel::replying(reply);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let outcome =
        draft_workflow_from_description(&runtime, "email me the weekly digest every monday")
            .await
            .unwrap();
    match outcome {
        DescriptionDraftOutcome::NotAutomatable(reason) => {
            assert!(
                reason.contains("output"),
                "names the missing output node: {reason}"
            );
        }
        DescriptionDraftOutcome::Graph { .. } => {
            panic!("a delivery with no output node must be refused")
        }
    }
    assert_eq!(model.calls(), 2);
}

/// The same "email me" request draws no gate when the draft actually ends in an
/// `output` node with a destination — the gate is about missing delivery, not
/// about the word "email".
#[tokio::test]
async fn a_delivery_with_an_output_node_drafts_cleanly() {
    let reply = r#"{"automatable":true,"summary":"digest","workflow":{"name":"Digest",
        "nodes":[{"id":"t","kind":"trigger","name":"Monday","schedule":"0 9 * * 1"},
                 {"id":"a","kind":"agent","name":"Draft","agent":"maya"},
                 {"id":"o","kind":"output","name":"Send","destination":{"kind":"owner"}}],
        "edges":[{"from":"t","to":"a"},{"from":"a","to":"o"}]}}"#;
    let model = ScriptedModel::replying(reply);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let outcome = draft_workflow_from_description(&runtime, "email me the weekly digest")
        .await
        .unwrap();
    assert!(
        matches!(outcome, DescriptionDraftOutcome::Graph { .. }),
        "a correct delivery graph drafts"
    );
    assert_eq!(model.calls(), 1, "a correct delivery graph needs no retry");
}

/// The card entering the pass is not the assignee's dispatch: no artifact, no
/// delegation — only a proposal or a return. A `once` deliverable is never routed
/// here, which the runtime's dispatch branch enforces; this pins that the builder
/// itself refuses a card that is not a `workflow` card even if called directly
/// (a rebuild race, or a card flipped back to once mid-flight).
#[tokio::test]
async fn a_once_card_is_not_built() {
    let model = ScriptedModel::replying(VALID_GRAPH);
    let (_home, runtime) = runtime_with(Arc::clone(&model)).await;
    let mut once = card("t-8", None);
    once.deliverable = TaskDeliverable::Once;
    runtime.tasks().upsert(runtime.id(), &once).await.unwrap();
    let run_id = open_run(&runtime, "t-8").await;

    run_workflow_build_pass(
        Arc::clone(&runtime),
        "t-8".to_string(),
        Some(run_id.clone()),
    )
    .await;

    let after = read(&runtime, "t-8").await;
    assert!(after.workflow_proposal.is_none());
    assert_eq!(
        after.column, COLUMN_IN_PROGRESS,
        "the card is left untouched"
    );
    assert_eq!(model.calls(), 0, "no model call for a non-workflow card");
    assert_eq!(run_status(&runtime, &run_id).await, RunStatus::Cancelled);
}
