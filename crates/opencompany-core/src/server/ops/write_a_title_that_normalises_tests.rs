//! Integration tests for the `ops` write plane: tasks, memory, workspace,
//! skills, team, inbox-read, and desk chat — exercised end-to-end over the
//! router against a real fs-backed company.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::company::steer::{InflightEntry, InflightKind};
use crate::ports::facts::{FactKind, FactRecord};
use crate::ports::tasks::{TaskRecord, TaskTitle};
use crate::ports::types::{CompanyId, CompanyRecord, CompressedTrace, ContextChunk};
use crate::runtime::RuntimeBuilder;
use crate::runtime::journal::{ApprovalConversation, TaskLink};
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("opencompany-ops-")
        .tempdir()
        .expect("tempdir")
}

fn manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
    )
    .unwrap()
}

/// The sorted node names in a workspace tree body.
///
/// A freshly-built company is no longer an empty tree: boot scaffolds the
/// reserved `agents/` and `desks/` roots (issue #551), so the tests below name
/// what they expect rather than counting to zero. Nothing is provisioned
/// *inside* them — a member folder is minted when that agent or desk first
/// produces something.
fn provisioned_names(tree: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = tree
        .as_array()
        .expect("the tree read is an array")
        .iter()
        .map(|node| node["name"].as_str().unwrap_or_default().to_string())
        .collect();
    names.sort();
    names
}

async fn state_with_company(home: &std::path::Path) -> AppState {
    state_with_quota(home, crate::runtime::WorkspaceQuota::default()).await
}

/// [`state_with_company`], with the workspace held to `quota`.
///
/// Parameterised rather than duplicated so the one test that needs a non-default
/// `[workspace] max_blob_mb` (issue #647) exercises the same wiring every other
/// test here does, instead of a second harness that could drift from it.
async fn state_with_quota(
    home: &std::path::Path,
    quota: crate::runtime::WorkspaceQuota,
) -> AppState {
    state_with(home, quota, None).await
}

/// [`state_with_company`], with the workspace tree served by `workspace`
/// (issue #759).
///
/// The `fs` backend refuses to create two sibling nodes with one name
/// (`reject_path_collision`, issue #665), so the raced tree the repair route
/// exists to fix cannot be built through it. sqlite and mongodb — the backends
/// hosted tenants run, and the reason the state exists at all — accept it, and
/// this swaps in a double that behaves the same way. Everything else about the
/// harness is unchanged, so the route under test is the one the console calls.
async fn state_with_workspace(
    home: &std::path::Path,
    workspace: std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>,
) -> AppState {
    state_with(
        home,
        crate::runtime::WorkspaceQuota::default(),
        Some(workspace),
    )
    .await
}

async fn state_with(
    home: &std::path::Path,
    quota: crate::runtime::WorkspaceQuota,
    workspace: Option<std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>>,
) -> AppState {
    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest(),
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
    let mut builder = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(id.clone())
        .with_workspace_quota(quota);
    if let Some(workspace) = workspace {
        builder = builder.with_workspace(workspace);
    }
    let runtime = builder.build().await.unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    // Every route needs a principal now; the harness signs in as an admin so
    // tests keep asserting write behavior rather than auth.
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// The repo's `companies/` directory, whose bundles' `skills/` are the skill
/// registry — the same directory the serve path derives `skills_root` from.
fn repo_skills_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../companies")
}

/// Like [`state_with_company`], but with the repo's shipped bundles wired in as
/// the skill registry, so registry reads and server-authoritative installs
/// resolve against real documents instead of degrading to the empty-registry
/// fallback.
async fn state_with_registry(home: &std::path::Path) -> AppState {
    // `with_skills_root` consumes and returns the state, so the registered
    // company and seeded admin move along with it.
    state_with_company(home)
        .await
        .with_skills_root(repo_skills_root())
}

/// The operator deltas persisted for `acme` — the durable rows behind the API.
async fn persisted_skills(state: &AppState) -> Vec<crate::ports::skills_state::SkillState> {
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("company");
    runtime.skills().list(runtime.id()).await.expect("deltas")
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_auth(state, method, uri, body, None).await
}

async fn send_auth(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    } else {
        // No explicit credential: sign in as the harness admin. Every route
        // needs a principal now, so an unauthenticated request would only ever
        // assert 401 rather than the behavior under test.
        request = request.header("cookie", crate::server::test_support::fixed_cookie("acme"));
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// A headline that normalises away is refused, not persisted (coderabbit on
/// #2055).
///
/// The `"""` bug's sibling, one layer out. That one was a *model* reply peeling
/// to a lone quote; this is a person typing punctuation into the title field.
/// The junk test that catches the first deliberately does not apply here — a
/// person who names a card `🚀` means it — so the boundary that persists the
/// card is what has to refuse a name with nothing in it, on both routes that
/// can set one.
#[tokio::test]
async fn a_title_that_normalises_to_nothing_is_refused_on_create_and_rename() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Non-blank on the wire, and nothing left after the sentence-punctuation
    // strip — so a length check on the raw input passes it through.
    for junk in ["...", "!!!", " . . . "] {
        let (status, _body) = send(
            &state,
            "POST",
            "/api/v1/company/tasks",
            Some(json!({ "title": junk })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "create accepted {junk:?}");
    }

    // A symbol a person plainly meant is still a title, and still lands.
    let (status, rocket) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({ "title": "🚀" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rocket["title"], "🚀");
    let id = rocket["id"].as_str().unwrap().to_string();

    // …and the rename route refuses the same junk rather than blanking it.
    let (status, _body) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({ "title": "..." })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    let still = board
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str());
    assert_eq!(
        still.map(|t| &t["title"]),
        Some(&json!("🚀")),
        "a refused rename leaves the card named as it was"
    );
}

#[tokio::test]
async fn tasks_crud_round_trips_under_both_scopes() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Create via the single-company alias.
    let (status, task) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Q2 brief", "priority": "high"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(task["title"], "Q2 brief");
    // Issue #206/#301: manual entry lands in To-do, the board's one intake lane
    // — which reads as `pending`, its phase, since issue #1512.
    assert_eq!(task["column"], "pending");
    let id = task["id"].as_str().unwrap().to_string();

    // Drag (PATCH column) via the {id} scope.
    let (status, moved) = send(
        &state,
        "PATCH",
        &format!("/api/v1/companies/acme/tasks/{id}"),
        Some(json!({"column": "done"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(moved["column"], "done");

    // List (GET) reflects the write — the board the console reads.
    let (status, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = board.as_array().expect("array of cards");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id);
    assert_eq!(rows[0]["column"], "done");

    // Delete.
    let (status, _) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // Second delete is a 404.
    let (status, _) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// #205: a card may only be assigned to somebody the company actually has.
/// Before this the board's free-text Assignee field accepted anything, the bad
/// value was persisted verbatim, and dispatch silently handed the work to the
/// orchestrator instead.
#[tokio::test]
async fn task_writes_reject_an_off_roster_assignee() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Fetch my activity", "assignee": "Shane"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.to_string().contains("Shane"),
        "the refusal must name what was typed: {body}"
    );

    // A roster teammate is fine, matched case-insensitively…
    let (status, task) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Q2 brief", "assignee": "CEO"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = task["id"].as_str().unwrap().to_string();

    // …and so is blank — an unassigned card is not an error.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Unowned"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The same rule on PATCH, and the rejected patch leaves the card untouched.
    let (status, _) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({"title": "Renamed", "assignee": "Shane"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    let card = board
        .as_array()
        .expect("board")
        .iter()
        .find(|c| c["id"] == json!(id))
        .expect("the card survives a rejected patch")
        .clone();
    assert_eq!(
        card["assignee"], "ceo",
        "the typed key is stored as the canonical roster id"
    );
    assert_eq!(
        card["title"], "Q2 brief",
        "a rejected patch must not persist the fields it did apply"
    );
}

/// #205: a column the board does not render is refused too. A typo'd
/// `in-progress` used to be persisted verbatim, hiding the card from every
/// rendered column *and* — since only the exact literal `in_progress`
/// edge-fires a dispatch — silently never running it.
#[tokio::test]
async fn task_writes_reject_a_column_the_board_cannot_render() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Typo'd", "column": "in-progress"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.to_string().contains("working"),
        "the refusal must list the columns that do exist: {body}"
    );

    let (status, task) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Fine", "column": "paused"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = task["id"].as_str().unwrap().to_string();

    let (status, _) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({"column": "reviewing"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    // The refused patch left the card where it was: paused, which reads as the
    // `working` phase with `paused` named as the stage (issue #1512).
    let card = &board.as_array().expect("board")[0];
    assert_eq!(card["column"], "working");
    assert_eq!(card["stage"], "paused");
}

/// #334: `in_review → done` is a move the write boundary accepts, and the one
/// the board's drag actually sends.
///
/// QA reported that a card "cannot be moved out of In review" — the drop did
/// nothing and said nothing, which cannot distinguish a host refusing the write
/// from a console that never sent it. It was the console (the drop was missing
/// the mostly off-window last column, and every miss was silent), but nothing
/// pinned the host's half of that answer. This does: both columns are in
/// `BOARD_COLUMNS`, the transition is special-cased nowhere, and `done` is
/// terminal — the card lands there and no dispatch fires behind it.
#[tokio::test]
async fn a_card_moves_from_in_review_to_done() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, seeded) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Invoice March retainer", "column": "in_review"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let id = seeded["id"].as_str().unwrap().to_string();

    // Exactly the body a drag onto Done sends.
    let (status, moved) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({"column": "done"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the board's drag PATCH must be accepted: {moved}"
    );
    assert_eq!(moved["column"], crate::ports::tasks::COLUMN_DONE);

    // What the board reads back on its next poll, not just what the echo said —
    // a card that snaps back is the shape of the original report.
    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    let card = board
        .as_array()
        .expect("board")
        .iter()
        .find(|c| c["id"] == json!(id))
        .expect("the card is still on the board");
    assert_eq!(card["column"], crate::ports::tasks::COLUMN_DONE);
}

/// Issue #206: `POST …/tasks` defaults a new card to To-do — the board's one
/// manual-entry column — while an explicit `column` still wins, so the
/// lifecycle paths that place a card themselves are untouched.
///
/// Issue #301 kept the default and reshaped what "explicit" may say: `planning`
/// is now a column (inert, but the write boundary must accept it before §4's
/// auto-advance starts writing it), and the removed `backlog` pool is refused.
#[tokio::test]
async fn created_tasks_default_to_the_todo_column() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, defaulted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "queued work"})),
    )
    .await;
    assert_eq!(defaulted["column"], crate::ledger::board::PHASE_PENDING);

    // An explicit column is still honored verbatim — `spawn_task`, the
    // orchestrator's `revise`, and a failed run all place their own card.
    let (status, explicit) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "being planned", "column": "planning"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(explicit["column"], crate::ledger::board::PHASE_WORKING);
    assert_eq!(explicit["stage"], crate::ports::tasks::COLUMN_PLANNING);

    // Issue #301: `backlog` is gone from the board, so a client still writing
    // it is refused rather than persisting a card nothing renders. Legacy data
    // heals silently on read (`ports::tasks`), but a *write* fails loudly — the
    // error names the set that replaced it.
    let (status, refused) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "parked", "column": "backlog"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused.to_string().contains("pending"),
        "the refusal must name the columns that replaced it: {refused}"
    );

    // …and the same on a drag, so a stale console cannot move a card into the
    // removed column either.
    let id = defaulted["id"].as_str().unwrap().to_string();
    let (status, _) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({"column": "backlog"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Issue #246: `POST …/tasks` carries the thread a card was opened from.
///
/// `TaskRecord.origin_chat_id` has existed since #151 and the tool-spawn path
/// stamped it, but this handler hardcoded `None` and no DTO projected it — so a
/// card opened from a conversation had no way back to it, and #151's
/// "answer where you were asked" post-back could never fire for anything the
/// REST surface created. Both halves are checked here: the write keeps it, and
/// **both** reads (the board list and task detail) hand it back.
#[tokio::test]
async fn a_task_created_from_a_thread_remembers_and_reads_back_that_thread() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Ship the brief", "originChatId": "strategy"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["originChatId"], "strategy");
    let id = created["id"].as_str().unwrap().to_string();

    // The board read (what the console lists) carries it…
    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    assert_eq!(board.as_array().unwrap()[0]["originChatId"], "strategy");

    // …and so does task detail, which is where an operator asks "where did
    // this come from?".
    let (status, detail) = send(&state, "GET", &format!("/api/v1/company/tasks/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["task"]["originChatId"], "strategy");

    // A card created without a thread — the board's `+` button — omits the key
    // entirely rather than sending null, so the pre-#246 wire shape is
    // unchanged for every card that has no conversation behind it.
    let (_, plain) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Typed on the board"})),
    )
    .await;
    assert!(
        plain.get("originChatId").is_none(),
        "a card with no originating thread must not grow the key: {plain}"
    );

    // A blank thread id is normalised away rather than persisted as a thread
    // that matches nothing.
    let (_, blank) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({"title": "Blank origin", "originChatId": "   "})),
    )
    .await;
    assert!(blank.get("originChatId").is_none(), "{blank}");
}

/// A card raised inside a thread carries the thread root on both the task
/// detail and board wire responses, not only on the stored `TaskRecord`.
///
/// `originParent` is stamped by the tool-spawn path rather than the REST
/// create body, so the record is seeded directly here.
#[tokio::test]
async fn a_task_detail_response_carries_the_thread_root_of_a_threaded_origin() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();

    runtime
        .tasks()
        .upsert(
            &company,
            &TaskRecord {
                id: "threaded".into(),
                title: TaskTitle::authored("Ship the brief"),
                note: None,
                column: crate::ports::tasks::COLUMN_TODO.into(),
                priority: "medium".into(),
                assignee: String::new(),
                updated_at_millis: 1,
                origin: crate::ports::tasks::TaskOrigin::new(
                    Some("strategy".to_string()),
                    Some(crate::ports::types::EventSeq::new(41)),
                ),
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
            },
        )
        .await
        .unwrap();

    let (status, detail) = send(&state, "GET", "/api/v1/company/tasks/threaded", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["task"]["originChatId"], "strategy");
    assert_eq!(detail["task"]["originParent"], 41, "{detail}");

    let (_, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    let card = board
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "threaded")
        .unwrap();
    assert_eq!(card["originParent"], 41, "{card}");
}
