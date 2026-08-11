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
use tinyagents::harness::model::{ChatModel, ModelResponse};
use tinyagents::{Result as TaResult, TinyAgentsError};

use super::*;
use crate::company::CompanyManifest;
use crate::ports::runs::{NewRun, RunStatus};
use crate::ports::types::CompanyId;

// ---------------------------------------------------------------------------
// A scripted model
// ---------------------------------------------------------------------------

/// A model that answers with a canned string (or fails), counts its calls, and —
/// optionally — mutates the board mid-call to simulate an operator moving the
/// card out from under the pass.
struct ScriptedModel {
    reply: Option<String>,
    calls: AtomicUsize,
    /// When set, the model moves the card to To-do on invoke, before answering —
    /// the operator's drag landing while the pass is waiting on the model.
    move_card: StdMutex<Option<(Weak<CompanyRuntime>, String)>>,
}

impl ScriptedModel {
    fn replying(reply: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            reply: Some(reply.into()),
            calls: AtomicUsize::new(0),
            move_card: StdMutex::new(None),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            reply: None,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
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
        match &self.reply {
            Some(reply) => Ok(ModelResponse::assistant(reply.clone())),
            None => Err(TinyAgentsError::Model("the brain is down".to_string())),
        }
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
