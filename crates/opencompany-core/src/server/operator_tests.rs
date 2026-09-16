/// The wire shape the console binds to.
///
/// `fold_asides` is worthless if the field reaches the browser under a
/// different name, and `tsc` cannot catch that: the DTO is Rust, the
/// interface is hand-written TypeScript, and nothing checks one against the
/// other. This is that check.
#[test]
fn the_folded_aside_reaches_the_wire_as_camel_case() {
    use crate::server::chat_history::{AsideConversation, AsideLine};

    let aside = AsideConversation {
        members: vec!["exchanges".to_owned(), "refunds".to_owned()],
        lines: vec![AsideLine {
            author_id: "exchanges".to_owned(),
            text: "the difference is -$16.63".to_owned(),
        }],
    };
    let dto = super::AsideConversationDto {
        members: aside.members,
        lines: aside
            .lines
            .into_iter()
            .map(|line| super::AsideLineDto {
                author_id: line.author_id,
                text: line.text,
            })
            .collect(),
    };
    let wire = serde_json::to_value(&dto).expect("the DTO serializes");

    assert!(
        wire.get("members").is_some(),
        "author first, then who they addressed: {wire}"
    );
    let line = &wire.get("lines").and_then(|l| l.as_array()).expect("lines")[0];
    assert_eq!(
        line.get("authorId").and_then(|a| a.as_str()),
        Some("exchanges"),
        "camelCase, as the console reads it: {wire}"
    );
    assert_eq!(
        line.get("text").and_then(|t| t.as_str()),
        Some("the difference is -$16.63"),
        "and the marker head never reaches the browser"
    );
}

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;
use crate::company::CompanyManifest;
use crate::ports::tasks::TaskTitle;
use crate::ports::types::CompanyRecord;
use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("opencompany-http-")
        .tempdir()
        .expect("tempdir")
}

fn manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
}

async fn state_with_company(home: &std::path::Path, lifecycle: &str) -> AppState {
    build_state(home, lifecycle, AppConfig::default()).await
}

async fn build_state(home: &std::path::Path, lifecycle: &str, config: AppConfig) -> AppState {
    build_state_with_brain(home, lifecycle, config, None).await
}

/// [`build_state`], optionally swapping the runtime's cognition. The
/// approval-continuation tests need a brain they can stall mid-turn.
async fn build_state_with_brain(
    home: &std::path::Path,
    lifecycle: &str,
    config: AppConfig,
    brain: Option<Arc<dyn crate::ports::brain::Brain>>,
) -> AppState {
    build_state_with_brain_and_manifest(home, lifecycle, config, brain, manifest()).await
}

/// [`build_state_with_brain`], with the company manifest chosen by the
/// caller — the approval **deadline** lives in `[policy]`, so a test about
/// what a past-deadline card answers has to be able to set it (issue #1449).
async fn build_state_with_brain_and_manifest(
    home: &std::path::Path,
    lifecycle: &str,
    config: AppConfig,
    brain: Option<Arc<dyn crate::ports::brain::Brain>>,
    manifest: CompanyManifest,
) -> AppState {
    // Pre-seed a record so the builder preserves the requested lifecycle.
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: lifecycle.to_string(),
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

    let mut builder = RuntimeBuilder::new(home.to_path_buf(), manifest).with_id(id.clone());
    if let Some(brain) = brain {
        builder = builder.with_brain(brain);
    }
    let runtime = builder.build().await.unwrap();
    let state = AppState::new(config);
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// A run store that refuses every verb — the persistence layer mid-outage.
/// `accept_chat_turn` treats a refused row best-effort, so this store is
/// what probes the other half of that promise: the turn still runs and the
/// request still gets an answer, it just cannot be a pollable `202`.
struct FailingRunStore;

#[async_trait::async_trait]
impl crate::ports::runs::RunStore for FailingRunStore {
    async fn create_run(
        &self,
        _company: &CompanyId,
        _spec: crate::ports::runs::NewRun,
    ) -> crate::Result<crate::ports::runs::RunRecord> {
        Err(OpenCompanyError::InvalidRequest(
            "run store offline".to_string(),
        ))
    }
    async fn get_run(
        &self,
        _company: &CompanyId,
        _id: &str,
    ) -> crate::Result<Option<crate::ports::runs::RunRecord>> {
        Ok(None)
    }
    async fn put_run(
        &self,
        _company: &CompanyId,
        _run: &crate::ports::runs::RunRecord,
    ) -> crate::Result<()> {
        Err(OpenCompanyError::InvalidRequest(
            "run store offline".to_string(),
        ))
    }
    async fn list_runs(
        &self,
        _company: &CompanyId,
        _filter: &crate::ports::runs::RunFilter,
    ) -> crate::Result<Vec<crate::ports::runs::RunRecord>> {
        Ok(Vec::new())
    }
    async fn append_run_step(
        &self,
        _company: &CompanyId,
        _step: &crate::ports::runs::RunStepRecord,
    ) -> crate::Result<()> {
        Err(OpenCompanyError::InvalidRequest(
            "run store offline".to_string(),
        ))
    }
    async fn list_run_steps(
        &self,
        _company: &CompanyId,
        _run_id: &str,
    ) -> crate::Result<Vec<crate::ports::runs::RunStepRecord>> {
        Ok(Vec::new())
    }
}

/// [`state_with_company`] with the run store swapped for one that refuses
/// every verb — the setup for the rowless-turn tests.
async fn state_with_failing_runs(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(id.clone())
        .with_runs(Arc::new(FailingRunStore))
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

#[tokio::test]
async fn chat_returns_echoed_response() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                // Issue #1725: not "hi". A bare pleasantry is answered by
                // the runtime without a turn, so the echo brain — which is
                // what this asserts is wired up — never sees it.
                .body(Body::from(r#"{"text":"ship the landing page"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["responses"][0]["text"],
        "You said: ship the landing page"
    );
    assert_eq!(value["responses"][0]["channel"], "operator");
}

/// An actionable operator chat opens exactly one task card on the dashboard
/// (deterministic, independent of the brain's own `spawn_task`), and a
/// greeting opens none. Runs on the default echo brain, so it proves the
/// handler-level wiring, not model behaviour.
///
/// Issue #576: that card now lands in **Planning**, not To-do. The request
/// carries `fixed_cookie`, so a signed-in person is behind it — which is
/// what the promotion is conditional on.
///
/// `tasks.len() == 1` is doing real work here beyond "a card was opened":
/// the card is created *directly* in `planning` by a single `upsert_task`,
/// so a second card, or a card that arrived via To-do and was promoted,
/// would both show up here.
#[tokio::test]
async fn actionable_chat_opens_a_planning_task_card() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let chat = |text: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/company/chat")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"text":{}}}"#,
                serde_json::json!(text)
            )))
            .unwrap()
    };

    // Actionable → one Planning card, titled from the ask.
    let r = app
        .clone()
        .oneshot(chat("build the landing page"))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1, "an actionable ask opens one card");
    assert_eq!(
        tasks[0].column,
        crate::ports::tasks::COLUMN_PLANNING,
        "issue #576: the prompt box promotes its own card, with no drag"
    );
    assert_eq!(tasks[0].priority, "medium");
    assert_eq!(tasks[0].title, "Build the landing page");

    // Greeting → no new card.
    let r = app.oneshot(chat("thanks!")).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1, "a greeting must not open a card");
}

/// Issue #1725, through the route an operator actually hits: "hi" comes
/// back answered, with no card, no steps, and no turn behind it.
///
/// The unit-level proof that the brain is not called lives in
/// `runtime::cycle`'s `a_bare_greeting_answers_without_calling_the_brain`,
/// where a counting brain can be injected. This one pins that the chat
/// handler reaches that path at all — the two are separate failures, and a
/// correct fast path nothing routes to leaves the bug where it was.
///
/// The echo brain answers `"You said: <text>"`, so the assertion below is
/// also the evidence: a canned greeting means the brain never ran.
#[tokio::test]
async fn a_bare_greeting_is_answered_without_a_turn() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["responses"][0]["text"],
        crate::company::task_intent::SmallTalk::Hello.reply(),
        "a greeting is answered by the runtime, not by a turn"
    );
    // The console showed "1 step" for a greeting on staging. There is no
    // step to show, so the field is omitted entirely.
    assert!(
        value["responses"][0]["steps"].is_null(),
        "no tool ran: {}",
        value["responses"][0]
    );
    assert!(
        runtime.tasks().list(&id).await.unwrap().is_empty(),
        "a greeting opens no card"
    );
}

// ── Issue #982: the card goes to whoever was addressed ──────────────────

/// A roster with three teammates and one desk, so a chat can be addressed
/// to something that exists.
///
/// The ids are the ones the smoke that found #982 used, and the roles are
/// deliberately distinct words, so a test can address one teammate in a
/// message whose text points at another.
fn roster_manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "product_manager"
role = "Product Manager"

[[agent]]
id = "backend_engineer"
role = "Backend Engineer"

[[agent]]
id = "designer"
role = "Designer"

[[group_chat]]
id = "engineering"
name = "Engineering"
members = ["backend_engineer"]

[policy]
mode = "full"
"#,
    )
    .unwrap()
}

/// [`state_with_company`] over [`roster_manifest`]. Written out rather than
/// threaded through the shared builders above, which several other suites
/// call with the roster-less fixture.
async fn state_with_roster(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: roster_manifest(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), roster_manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// [`roster_manifest`] plus a **memberless** desk — one that exists on the
/// roster but has nobody seated on it, the `EmptyDesk` shape `mention_context`
/// still has to canonicalize.
fn memberless_desk_manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "product_manager"
role = "Product Manager"

[[agent]]
id = "backend_engineer"
role = "Backend Engineer"

[[group_chat]]
id = "engineering"
name = "Engineering"
members = ["backend_engineer"]

[[group_chat]]
id = "sales"
name = "Sales"
members = []

[policy]
mode = "full"
"#,
    )
    .unwrap()
}

/// [`roster_manifest`] plus a desk **literally named** `dm:engineering`,
/// beside the ordinary `engineering` desk — the shape `mention_context`
/// must resolve **as sent** instead of stripping the `dm:` prefix away.
fn dm_prefixed_desk_manifest() -> CompanyManifest {
    toml::from_str(
        r#"
[company]
name = "Acme"

[[agent]]
id = "product_manager"
role = "Product Manager"

[[agent]]
id = "backend_engineer"
role = "Backend Engineer"

[[group_chat]]
id = "engineering"
name = "Engineering"
members = ["backend_engineer"]

[[group_chat]]
id = "dm:engineering"
name = "Dm Engineering"
members = ["backend_engineer"]

[policy]
mode = "full"
"#,
    )
    .unwrap()
}

/// [`state_with_roster`] over [`memberless_desk_manifest`].
async fn state_with_memberless_desk(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: memberless_desk_manifest(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), memberless_desk_manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// [`state_with_roster`] over [`dm_prefixed_desk_manifest`].
async fn state_with_dm_prefixed_desk(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: dm_prefixed_desk_manifest(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), dm_prefixed_desk_manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

/// One chat request, optionally addressed to a thread.
fn chat_to(text: &str, chat: Option<&str>) -> Request<Body> {
    chat_in_thread(text, chat, None)
}

/// The same send, typed inside a thread — `parent` is the root the console
/// sends when the operator answers in an open thread (#1890 B).
fn chat_in_thread(text: &str, chat: Option<&str>, parent: Option<u64>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/company/chat")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "text": text,
                "chat": chat,
                // A string, like every other message id on this API — the
                // field's own note says so, and a number is a 422.
                "parent": parent.map(|seq| seq.to_string()),
            })
            .to_string(),
        ))
        .unwrap()
}

/// The message the smoke sent: an actionable ask whose *text* points at one
/// teammate, addressed to a different one.
const CROSSED: &str = "build the backend deployment pipeline";

/// The card a chat opens is handed to the teammate the operator addressed.
///
/// The fixture is the whole test. `CROSSED` names *backend* work and is
/// addressed to the **product manager**, so the two candidate answers are
/// distinguishable: pre-fix this card was born blank and the planning pass
/// filled it from a content match of the title against teammate roles —
/// which is exactly the wrong answer here. A message whose text and
/// addressee agree would pass on pre-fix code and prove nothing; that is
/// what the two "right" rows of the issue's table were.
#[tokio::test]
async fn chat_addressed_to_a_teammate_assigns_that_teammate() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    assert!(
        matches!(
            crate::company::task_intent::triage_message(CROSSED),
            crate::company::task_intent::MessageTriage::Track(_)
        ),
        "fixture must be a message the handler cards, or this proves nothing"
    );

    let r = app
        .oneshot(chat_to(CROSSED, Some("product_manager")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1, "an actionable ask opens one card");
    assert_eq!(
        tasks[0].assignee, "product_manager",
        "the card belongs to the teammate the operator addressed"
    );
    assert_ne!(
        tasks[0].assignee, "backend_engineer",
        "…and not to whoever the message text happens to name"
    );
}

/// A desk-addressed chat is assigned to the **desk**, not to its lead.
///
/// Writing the lead would erase the desk from the board the moment the card
/// was created — the invariant `AssigneeResolution::canonical` holds for
/// every other write site (issue #214), now held here too.
#[tokio::test]
async fn chat_addressed_to_a_desk_assigns_the_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let r = app
        .oneshot(chat_to(CROSSED, Some("engineering")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].assignee, "engineering",
        "picking a desk IS the operator's routing decision"
    );
}

/// Everything that addresses nobody in particular still opens a blank card:
/// no thread at all, the empty string, the console's legacy fallback desk
/// id, and the default "General" desk this company does not have.
///
/// This pins the direction of the change — *more* cards are operator-chosen,
/// none fewer — and it is the clause that keeps the orchestrator's own queue
/// working: a blank assignee is what hands a card to it.
#[tokio::test]
async fn an_unaddressed_chat_leaves_the_card_unassigned() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    for thread in [None, Some(""), Some("main"), Some(DEFAULT_DESK)] {
        let r = app.clone().oneshot(chat_to(CROSSED, thread)).await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "thread {thread:?}");
    }

    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 4, "one card per message: {tasks:?}");
    for card in &tasks {
        assert_eq!(
            card.assignee, "",
            "an unaddressed message leaves the card for the orchestrator"
        );
    }
}

/// A thread key that names nothing on the roster is not an error: the card
/// is opened, unassigned, exactly as it was before this route resolved
/// anything. A chat must never 400 — and must never lose its card — over who
/// it was addressed to.
#[tokio::test]
async fn an_unknown_addressee_leaves_the_card_unassigned() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let r = app
        .oneshot(chat_to(CROSSED, Some("nobody_by_that_name")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK, "an unknown thread is not a 400");

    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1, "…and the card is still opened");
    assert_eq!(tasks[0].assignee, "", "…with nobody guessed onto it");
}

/// The card remembers the thread it was opened from, so the marker that says
/// it settled lands back in the conversation that asked for the work.
///
/// `origin_chat_id` is the field issue #151 added for exactly this, and the
/// console already renders the marker in whatever channel it names — the
/// route was simply never filling it in. An unaddressed message still opens
/// a card with no origin, which is every card this route opened before.
#[tokio::test]
async fn a_chat_card_remembers_the_thread_it_was_opened_from() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let r = app
        .clone()
        .oneshot(chat_to(CROSSED, Some("dm:designer")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(
        tasks[0].origin_chat_id(),
        Some("dm:designer"),
        "the thread as the console addressed it"
    );

    let r = app
        .oneshot(chat_to("draft the investor update", None))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    let unaddressed = tasks
        .iter()
        .find(|c| c.title == "Draft the investor update")
        .expect("the second card");
    // No desk, therefore no conversation and no thread inside one. Before
    // #1890 step 5 this card carried a thread root beside no desk — the
    // drifted pair — and the root was inert: `relay_reply` posts back
    // through the desk, so a root with nothing to post into named nothing.
    // `TaskOrigin` cannot hold that state, so it is simply absent now.
    //
    // Restoring a real origin here means stamping the General desk the
    // route already folds this message into, which is a behaviour change
    // and not this one.
    assert_eq!(
        unaddressed.origin_chat_id(),
        None,
        "an unaddressed message has no conversation to answer in"
    );
    assert_eq!(
        unaddressed.origin_parent(),
        None,
        "and therefore no thread inside one either"
    );

    // The addressed card, found by title rather than by index: the two are
    // listed together from here on, and this assertion is about the one
    // that has a desk.
    let addressed = tasks
        .iter()
        .find(|c| c.origin_chat_id() == Some("dm:designer"))
        .expect("the addressed card");
    // Reversed once #1890 D landed alongside B, and the reversal is the
    // point. B alone read the message's own `parent`, so a card raised from
    // a channel-level question recorded no thread — right while a thread
    // was only ever something an operator opened by hand.
    //
    // D changed what a thread is: an answer parents to the message that
    // opened the exchange, so that question is a root. A card raised from
    // it belongs to the thread it just started, and recording `None` here
    // would put the settle marker in the channel while the answer to the
    // same message sat in a thread — the split B exists to prevent.
    assert!(
        addressed.origin_parent().is_some(),
        "a channel-level question is itself the thread its card was raised in",
    );
}

/// Issue #1890 D part 1: every answer threads under the message that
/// opened the exchange.
///
/// The two arms are the whole rule, and the second is the change: before
/// it, an answer to an unthreaded question was journaled unparented, so the
/// only threads that existed were ones an operator opened by hand.
#[test]
fn an_answer_threads_under_the_message_that_opened_the_exchange() {
    let message = EventSeq::new(41);
    // Not in a thread: the exchange becomes one, rooted at the question.
    assert_eq!(reply_thread(None, message), Some(message));
    // Already in one: the same root, so a follow-up does not open a thread
    // of its own — N messages in a thread is one topic, not N.
    let root = EventSeq::new(7);
    assert_eq!(reply_thread(Some(root), message), Some(root));
    // Never `None`: uniform is what keeps `parent` out of the hands of race
    // timing, since `parent` is permanent and presentation is not.
    assert!(reply_thread(None, message).is_some());
    assert!(reply_thread(Some(root), message).is_some());
}

/// Issue #1890 B: the card remembers **which thread** inside that channel.
///
/// The channel alone was never enough — a channel holds any number of live
/// threads, and a settle filed against the channel surfaces in none of
/// them. A message's own `parent` IS its root (a reply is parented to its
/// question's parent, never to the question), so the route reads it
/// straight off the send with no walk.
#[tokio::test]
async fn a_chat_card_remembers_the_thread_inside_the_channel() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let r = app
        .oneshot(chat_in_thread(CROSSED, Some("dm:designer"), Some(41)))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks[0].origin_chat_id(), Some("dm:designer"));
    assert_eq!(
        tasks[0].origin_parent(),
        Some(crate::ports::types::EventSeq::new(41)),
        "the root the operator was answering in",
    );
}

/// The console mints a DM channel id as `dm:<teammate-id>`, and that form is
/// documented as a valid channel key — so it has to address the teammate
/// here as well as in the responder lookup.
#[tokio::test]
async fn a_console_dm_channel_id_addresses_the_teammate() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_roster(&home).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    let r = app
        .oneshot(chat_to(CROSSED, Some("dm:designer")))
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);

    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, "designer");
}

/// Issue #845: an explicit "Build me the workflow" opens a card even when
/// the triage would have opened nothing.
///
/// The composer's toggle was consulted *only* on the card-opening branch,
/// and that branch is gated on the triage. So a `workflow` request the
/// classifier read as a question or as chatter dropped the choice on the
/// floor: no card, therefore no builder pass, therefore nothing built — and
/// no error either, because a conversational reply came back as though the
/// message had been handled.
///
/// Both halves are pinned here: the same text opens nothing as a `once`
/// message and opens a `workflow` card when the operator asked for one.
#[tokio::test]
async fn an_explicit_workflow_request_opens_a_card_the_triage_declined() {
    use crate::ports::tasks::TaskDeliverable;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    // A question by construction — `is_question` fires on the wh-opener, so
    // the triage answers `Answer` and the card branch declines.
    let text = "what would a weekly AEO audit of the blog even look like?";
    assert!(
        matches!(
            crate::company::task_intent::triage_message(text),
            crate::company::task_intent::MessageTriage::Answer
        ),
        "fixture must be one the triage declines to card, or this proves nothing"
    );

    let chat = |deliverable: Option<&str>| {
        let body = match deliverable {
            Some(d) => format!(
                r#"{{"text":{},"deliverable":"{d}"}}"#,
                serde_json::json!(text)
            ),
            None => format!(r#"{{"text":{}}}"#, serde_json::json!(text)),
        };
        Request::builder()
            .method("POST")
            .uri("/api/v1/company/chat")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };

    // `once` (and no choice at all): unchanged — the triage still decides.
    let r = app.clone().oneshot(chat(None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let r = app.clone().oneshot(chat(Some("once"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        runtime.tasks().list(&id).await.unwrap().is_empty(),
        "a `once` question must still open nothing"
    );

    // `workflow`: the operator's explicit choice outranks the classifier.
    let r = app.oneshot(chat(Some("workflow"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 1, "the workflow choice must open its card");
    assert_eq!(
        tasks[0].deliverable,
        TaskDeliverable::Workflow,
        "and it must be the deliverable that routes it to the builder pass"
    );
    // Titled through `to_title`, exactly as a `Track` card would have been.
    assert_eq!(
        tasks[0].title,
        crate::company::task_intent::to_title(text),
        "a bypassed card must be titled byte-for-byte as a tracked one"
    );
}

/// A task store that lists cleanly but refuses every write — the board
/// persistence layer mid-outage, for CHAT-021.
struct FailingTaskUpsert;

#[async_trait::async_trait]
impl crate::ports::tasks::TaskStore for FailingTaskUpsert {
    async fn list(
        &self,
        _company: &CompanyId,
    ) -> crate::Result<Vec<crate::ports::tasks::TaskRecord>> {
        Ok(Vec::new())
    }
    async fn upsert(
        &self,
        _company: &CompanyId,
        _task: &crate::ports::tasks::TaskRecord,
    ) -> crate::Result<()> {
        Err(OpenCompanyError::InvalidRequest(
            "task store offline".to_string(),
        ))
    }
    async fn update_if_column(
        &self,
        _company: &CompanyId,
        _task: &crate::ports::tasks::TaskRecord,
        _observed: &crate::ports::tasks::TaskRecord,
        _expected_column: &str,
    ) -> crate::Result<bool> {
        Err(OpenCompanyError::InvalidRequest(
            "task store offline".to_string(),
        ))
    }
    async fn delete(&self, _company: &CompanyId, _id: &str) -> crate::Result<bool> {
        Ok(false)
    }
}

/// CHAT-021: a card-open failure must not vanish into a server log while
/// the operator sees an ordinary success. The chat turn itself still
/// returns 200 — it did nothing wrong — but a durable system note in the
/// same desk must say the card did not open, exactly as a turn that
/// aborts mid-answer already leaves a visible notice rather than silence.
#[tokio::test]
async fn a_card_open_failure_is_reported_in_the_channel_not_swallowed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let store = FsCompanyStore::new(home.clone());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
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
    let runtime = RuntimeBuilder::new(home, manifest())
        .with_id(id.clone())
        .with_tasks(Arc::new(FailingTaskUpsert))
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id.clone(), Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    // `deliverable: "workflow"` opens a card deterministically, whatever
    // the lexical triage would have made of the words (see the test
    // above) — the fixture does not need to be a message the classifier
    // happens to card.
    let body = format!(
        r#"{{"text":{},"deliverable":"workflow"}}"#,
        serde_json::json!("automate the weekly report")
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the chat turn itself did nothing wrong and must still succeed"
    );

    let events = runtime
        .events()
        .read_from(&id, EventSeq::new(0), usize::MAX)
        .await
        .unwrap();
    let notice = events.into_iter().find_map(|stored| match stored.event {
        CompanyEvent::AgentReply { agent_id, text, .. }
            if agent_id == crate::ports::SYSTEM_AUTHOR =>
        {
            Some(text)
        }
        _ => None,
    });
    assert!(
        notice.is_some_and(|text| text.to_lowercase().contains("card")),
        "a card-open failure must leave a visible system note in the channel, not just a \
         server-side log line"
    );
}

/// Issue #1152: an explicit "Just chatting" **withholds** the card the
/// triage would otherwise have opened.
///
/// The mirror of the test above, and the asymmetry it closes. Since #845 the
/// operator could override the classifier *upward* — mint a card it
/// declined — and there was no control anywhere that overrode it downward.
/// So a message the lexical layer reads as `Track` ("can you build the
/// landing page?" asked rhetorically, while thinking out loud) opened a
/// card, assigned it to a desk, and started a planning pass, and the only
/// recourse was to go to the board and delete it.
///
/// The fixture's verdict is asserted `Track` **first**, in the strongest
/// direction available: the `chat` run is made before any other, on an empty
/// board, and the unmarked run right after it opens the card on the very
/// same words. So "zero cards" is the intent doing the work, not a message
/// the classifier was never going to card.
#[tokio::test]
async fn just_chatting_withholds_the_card_the_triage_would_have_opened() {
    use crate::ports::tasks::TaskDeliverable;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let app = router(state);

    // Work by construction — the request frame beats the interrogative, so
    // the triage names a title and the card branch fires.
    let text = "can you build the landing page?";
    assert!(
        matches!(
            crate::company::task_intent::triage_message(text),
            crate::company::task_intent::MessageTriage::Track(_)
        ),
        "fixture must be a message the handler cards, or this proves nothing"
    );

    let chat = |intent: Option<&str>| {
        let body = match intent {
            Some(i) => format!(
                r#"{{"text":{},"deliverable":"{i}"}}"#,
                serde_json::json!(text)
            ),
            None => format!(r#"{{"text":{}}}"#, serde_json::json!(text)),
        };
        Request::builder()
            .method("POST")
            .uri("/api/v1/company/chat")
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap()
    };

    // `chat`: the operator's statement outranks the classifier's `Track`.
    let r = app.clone().oneshot(chat(Some("chat"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK, "the message is still answered");
    assert!(
        runtime.tasks().list(&id).await.unwrap().is_empty(),
        "a message sent as chat must open no card, whatever the triage read"
    );

    // The same words, unmarked: the card the run above withheld.
    let r = app.clone().oneshot(chat(None)).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "an unmarked message is unchanged — this is what the `chat` run withheld"
    );
    assert_eq!(tasks[0].deliverable, TaskDeliverable::Once);

    // …and so are both work words, on the same words again.
    let r = app.clone().oneshot(chat(Some("once"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        runtime.tasks().list(&id).await.unwrap().len(),
        2,
        "`once` is unchanged"
    );

    let r = app.oneshot(chat(Some("workflow"))).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let tasks = runtime.tasks().list(&id).await.unwrap();
    assert_eq!(tasks.len(), 3, "`workflow` is unchanged");
    assert!(
        tasks
            .iter()
            .any(|t| t.deliverable == TaskDeliverable::Workflow),
        "and still routes its card to the builder pass: {tasks:?}"
    );
}

/// Issue #576: **who** asked decides whether the card self-promotes.
///
/// The promotion buys a planning pass, which is a model call. A person
/// spending one on their own typo is the cost the issue accepts; an agent
/// doing it is a loop — a card that plans, whose pass opens further cards,
/// which promote, which plan, with no human anywhere in it. So the branch is
/// on the actor, and this pins both sides of it.
///
/// Driven through `run_chat` directly rather than the route, because the
/// route's job is to *resolve* the actor and this test's job is to pin what
/// each resolved actor does. Going through HTTP would only ever exercise
/// whichever principal the test harness happens to authenticate as.
#[tokio::test]
async fn only_a_person_gets_a_self_promoting_card() {
    use crate::ports::tasks::{COLUMN_PLANNING, COLUMN_TODO};
    use crate::ports::types::{Actor, ActorKind};

    let ask = "build the landing page";
    let person = Actor {
        kind: ActorKind::User,
        id: "u-1".to_string(),
    };

    // Every actor that is not a person must leave the card where it has
    // always landed. `None` is a machine credential — the platform, or any
    // caller with no session behind it.
    for (label, by, expected) in [
        ("a signed-in user", Some(person.clone()), COLUMN_PLANNING),
        (
            "an operator",
            Some(Actor {
                kind: ActorKind::Operator,
                id: "op".to_string(),
            }),
            COLUMN_PLANNING,
        ),
        (
            "an agent",
            Some(Actor {
                kind: ActorKind::Agent,
                id: "ceo".to_string(),
            }),
            COLUMN_TODO,
        ),
        (
            "the runtime itself",
            Some(Actor {
                kind: ActorKind::System,
                id: "system".to_string(),
            }),
            COLUMN_TODO,
        ),
        ("a machine credential", None, COLUMN_TODO),
    ] {
        let home_dir = home();
        let state = state_with_company(home_dir.path(), "running").await;
        let id = CompanyId::new("acme");
        let runtime = state.registry().get(&id).unwrap();

        let message = ChatMessage {
            mentions: None,
            text: ask.to_string(),
            chat: None,
            parent: None,
            deliverable: None,
            detach: false,
            attachments: Vec::new(),
        };
        let accepted = accept_chat_turn(
            &runtime,
            &id,
            &message,
            by.as_ref(),
            None,
            crate::server::ops::language::DEFAULT_DESK,
        )
        .await
        .expect("the turn is accepted");
        run_chat(runtime.clone(), message, by, &accepted)
            .await
            .expect("the chat cycle runs");

        let tasks = runtime.tasks().list(&id).await.unwrap();
        assert_eq!(tasks.len(), 1, "{label}: one ask opens one card");
        assert_eq!(
            tasks[0].column, expected,
            "{label}: the card must land in `{expected}`"
        );
    }
}

/// End-to-end proof of the WS4 wire: with a [`HarnessBrain`] as the runtime's
/// cognition, `POST /company/chat` returns the **agent's** reply rather than
/// the echo brain's `"You said: …"`. The mock provider prefixes the routed
/// message, so `"mock: hi"` proves the operator message reached an openhuman
/// agent turn through the HTTP handler → `run_cycle` → brain path.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn chat_routes_through_the_harness_brain() {
    use crate::harness::provider::MockProvider;
    use crate::harness::{HarnessBrain, HarnessDeps, HarnessPool};
    use crate::ports::CompanyStore;
    use crate::store::{FsContextStore, FsOps};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let id = CompanyId::new("acme");
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief Executive\"\n",
    )
    .unwrap();

    let record = CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: id.clone(),
        manifest: manifest.clone(),
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
    FsCompanyStore::new(home.to_path_buf())
        .save(&record)
        .await
        .unwrap();

    let deps = HarnessDeps {
        emergency_gate: None,
        notifications: None,
        ledgers: None,
        ledger_registry: Default::default(),
        run_supervisor: crate::runtime::RunSupervisor::default(),
        provider: Arc::new(MockProvider::new("mock: ")),
        provider_slug: "mock".to_string(),
        serves: None,
        context: Arc::new(FsContextStore::new(home.to_path_buf())),
        store: Arc::new(FsCompanyStore::new(home.to_path_buf())),
        meter: Some(Arc::new(FsOps::new(home.to_path_buf()))),
        workspace_root: home.to_path_buf(),
        mcp_home: None,
        workspace_git_enabled: false,
        audit_root: home.to_path_buf(),
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
        delegations: crate::harness::orchestrator::DelegationQueue::default(),
        workflow_runner: crate::harness::orchestrator::WorkflowRunnerHandle::default(),
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
        delivery: None,
        search: None,
        tenant_search: None,
        workspace: None,
        workflow_runs: None,
        deep_trace: None,
    };
    let brain = HarnessBrain::new(Arc::new(HarnessPool::new()), deps, record);

    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .with_brain(Arc::new(brain))
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                // Issue #1725: not "hi". A bare pleasantry is answered by
                // the runtime without a turn, so it would reach no brain at
                // all — which is the opposite of what this asserts.
                .body(Body::from(r#"{"text":"ship the landing page"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let text = value["responses"][0]["text"].as_str().unwrap();
    // The mock provider's `mock: ` prefix proves the message went through an
    // openhuman agent turn; the trailing words are the operator message the
    // agent forwarded (the agent prepends a date/time context line).
    // Crucially it is NOT the echo brain's `"You said: …"`.
    assert!(text.starts_with("mock: "), "not an agent reply: {text:?}");
    assert!(
        text.trim_end().ends_with("ship the landing page"),
        "message not forwarded: {text:?}"
    );
    assert_ne!(
        text, "You said: ship the landing page",
        "still routing through the echo brain"
    );
    assert_eq!(value["responses"][0]["channel"], "operator");
}

/// A manifest with two agents and one desk (`studio`, led by `ceo`), used by
/// the desk-membership write tests.
fn desk_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n\
         [[group_chat]]\nid = \"studio\"\nname = \"Studio\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap()
}

/// A bare record carrying `manifest`, for resolvers that read nothing else.
fn record_with(manifest: CompanyManifest) -> CompanyRecord {
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

/// The name a chat turn's row carries.
///
/// Reproduced against a live company before this was written: a turn sent to
/// `#general` recorded `agent_id == chat_id == "main"`. A console that
/// reloaded mid-turn therefore had a durable row, a live turn, and nobody to
/// name — so its re-armed indicator could say only a bare "Working…", which
/// reads exactly like a console that has lost the turn.
#[test]
fn a_chat_turn_records_who_answers_rather_than_the_desk_it_was_sent_to() {
    let record = record_with(desk_manifest());
    assert_eq!(
        chat_turn_responder(&record, "studio"),
        "ceo",
        "a desk's turn is answered by the desk's lead, and that is the name \
         the reload leg has to render"
    );
    assert_ne!(
        chat_turn_responder(&record, "studio"),
        "studio",
        "recording the desk is the regression: it is what made every chat \
         row read `agent_id == chat_id`"
    );
}

/// A DM addresses the teammate directly — its thread id *is* a roster id —
/// so the row names that teammate rather than falling through to the
/// orchestrator.
#[test]
fn a_direct_message_records_the_teammate_it_addresses() {
    let record = record_with(desk_manifest());
    assert_eq!(chat_turn_responder(&record, "eng"), "eng");
}

/// Every spelling of the company's own line folds to one answer, so the
/// indicator does not name a different teammate depending on how the
/// console happened to address General.
#[test]
fn every_general_spelling_records_the_same_answer() {
    let record = record_with(desk_manifest());
    let folded: Vec<String> = ["", "main", "general", "General"]
        .into_iter()
        .map(|spelling| chat_turn_responder(&record, spelling))
        .collect();
    assert!(
        folded.windows(2).all(|pair| pair[0] == pair[1]),
        "the General spellings disagreed about who answers: {folded:?}"
    );
}

/// The floor. A company with nobody to name records the desk — which is
/// precisely what every chat turn recorded before this change, so the worst
/// case is the old behaviour rather than a row naming a teammate that does
/// not exist.
#[test]
fn a_company_with_no_roster_records_the_desk_exactly_as_before() {
    let empty: CompanyManifest =
        toml::from_str("[company]\nname = \"Acme\"\n").expect("a roster-less manifest");
    assert_eq!(chat_turn_responder(&record_with(empty), "studio"), "studio");
}

/// Builds an app state whose sole company carries `manifest`.
async fn state_with_manifest(home: &std::path::Path, manifest: CompanyManifest) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    use crate::ports::CompanyStore;
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest.clone(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

async fn get_desks(app: &axum::Router, cookie: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/desks")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Adding an overlay member persists it and surfaces it in `list_desks` as
/// both an effective member and a removable overlay member.
#[tokio::test]
async fn add_desk_member_persists_and_shows_in_list() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let add = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks/studio/members")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"agent_id":"eng"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["id"], "studio");
    // Manifest member first, overlay member appended.
    assert_eq!(desks[0]["members"][0], "ceo");
    assert_eq!(desks[0]["members"][1], "eng");
    assert_eq!(desks[0]["overlayMembers"][0], "eng");
}

/// Issue #1781 review (Codex P2): an overlay desk whose own id is a
/// General spelling (`general` or `main`) must not appear in `GET
/// .../desks` — `POST .../desks` has refused those ids since issue #1743,
/// so the only way one exists is a company upgraded from before that
/// guard, and `CompanyRecord::resolve_desk_id` already excludes exactly
/// this desk from routing. Listing it anyway would let `buildChannels`
/// (frontend) treat it as the company-wide line and suppress the real
/// built-in `#general` — showing edit/delete controls and a membership
/// list that has nothing to do with where a message actually lands.
///
/// Seeded directly on the stored record, not through `POST .../desks`:
/// that route's own guard means this shape can only be reached by data
/// that predates it, exactly the grandfathered case this proves.
#[tokio::test]
async fn list_desks_hides_an_overlay_desk_shadowing_general() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut record = runtime.store().load(&id).await.unwrap().unwrap();
    record.overlay_desks.push(OverlayDesk {
        id: "general".to_string(),
        name: "General".to_string(),
        description: None,
        members: vec!["ceo".to_string()],
        responder: ResponderMode::Lead,
        hive: Default::default(),
    });
    record.overlay_desks.push(OverlayDesk {
        id: "main".to_string(),
        name: "Front office".to_string(),
        description: None,
        members: vec!["eng".to_string()],
        responder: ResponderMode::Lead,
        hive: Default::default(),
    });
    runtime.store().save(&record).await.unwrap();

    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let desks = get_desks(&app, &cookie).await;
    let ids: Vec<&str> = desks
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();

    assert!(
        !ids.contains(&"general"),
        "an overlay desk at the reserved `general` id must not be listed: {ids:?}"
    );
    assert!(
        !ids.contains(&"main"),
        "an overlay desk at the reserved `main` id must not be listed: {ids:?}"
    );
    // The manifest desk and a non-shadowing overlay desk are unaffected —
    // this narrows one id, it does not hide desks generally.
    assert!(ids.contains(&"studio"), "unrelated desk dropped: {ids:?}");
}

/// Every desk mutation aimed at a bare General spelling — no legacy
/// overlay row at all — is refused with a reason, under **every** spelling
/// the host folds into the General conversation (issue #1743; restored PR
/// #1781 review, CodeRabbit P2).
///
/// This is the `is_general_channel` guard originally added by `da98130c1`
/// and its own regression test; an unrelated refactor (`3cbdb7a5f`) deleted
/// the guard, the four call sites, and this test together, and only the
/// read-side projection filter (`list_desks`/`resolve_desk_id`) was ever
/// restored (`0c07873db`) — this proves the write side is closed again.
///
/// The point of the assertion is the pair: a `409` **and** the sentence.
/// Before this guard, each of these was a bare `404`/`CompanyNotFound` —
/// "there is no such desk" — which is a different and wrong claim.
/// `#general` is not missing; it is reserved, and the caller needs to be
/// told which.
#[tokio::test]
async fn every_desk_mutation_aimed_at_a_bare_general_spelling_is_refused_with_a_reason() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    for spelling in ["general", "General", "GENERAL", "main", "Main"] {
        let cases: [(&str, String, &str); 4] = [
            ("DELETE", format!("/api/v1/company/desks/{spelling}"), ""),
            (
                "POST",
                format!("/api/v1/company/desks/{spelling}/members"),
                r#"{"agent_id":"eng"}"#,
            ),
            (
                "DELETE",
                format!("/api/v1/company/desks/{spelling}/members/ceo"),
                "",
            ),
            (
                "PUT",
                format!("/api/v1/company/desks/{spelling}/order"),
                r#"{"ordered_member_ids":["ceo"]}"#,
            ),
        ];
        for (method, uri, body) in cases {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(&uri)
                        .header("cookie", &cookie)
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::CONFLICT,
                "{method} {uri} must be refused, not answered 404"
            );
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains("company-wide channel"),
                "{method} {uri} must say why: got {text}"
            );
        }
    }
}

/// Sibling to [`list_desks_hides_an_overlay_desk_shadowing_general`]: the
/// same grandfathered overlay desk at the reserved `general` id — which
/// that test proves is hidden from `GET .../desks` and unroutable through
/// [`CompanyRecord::resolve_desk_id`] — must also be unreachable through
/// every desk *mutation* (issue #1781 review, CodeRabbit P2). Before this
/// guard was restored, `desk_exists("general")` was `true` for exactly this
/// desk (it really is in `overlay_desks`), so `add_desk_member`,
/// `remove_desk_member`, `set_desk_order`, and `delete_desk` — which
/// checked only `desk_exists` — would staff, reorder, or delete a desk no
/// read surface exposes at all.
///
/// Seeded directly on the stored record, the same way the read-side sibling
/// test is: `POST .../desks` has refused this id since issue #1743, so the
/// only way this shape exists is data that predates that guard.
#[tokio::test]
async fn desk_mutations_refuse_a_grandfathered_overlay_desk_shadowing_general() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut record = runtime.store().load(&id).await.unwrap().unwrap();
    record.overlay_desks.push(OverlayDesk {
        id: "general".to_string(),
        name: "General".to_string(),
        description: None,
        members: vec!["ceo".to_string()],
        responder: ResponderMode::Lead,
        hive: Default::default(),
    });
    runtime.store().save(&record).await.unwrap();

    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let cases: [(&str, &str, &str); 4] = [
        ("DELETE", "/api/v1/company/desks/general", ""),
        (
            "POST",
            "/api/v1/company/desks/general/members",
            r#"{"agent_id":"eng"}"#,
        ),
        ("DELETE", "/api/v1/company/desks/general/members/ceo", ""),
        (
            "PUT",
            "/api/v1/company/desks/general/order",
            r#"{"ordered_member_ids":["ceo"]}"#,
        ),
    ];
    for (method, uri, body) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "{method} {uri} must be refused even though the desk really \
             exists in the overlay — desk_exists alone is not enough"
        );
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("company-wide channel"),
            "{method} {uri} must say why: got {text}"
        );
    }
}

/// `add_desk_member` must serialize its load-modify-save cycle against
/// `company_write_lock`, exactly like every other console load-modify-save
/// write (`put_logo`, `set_lifecycle`, `patch_company`) — otherwise it can
/// silently revert a concurrent rename: `patch_company` is guarded by
/// `company_write_lock` alone, so a desk write racing in on only the
/// unrelated `serial` cycle lock can load the pre-rename record and save
/// the whole thing back after the rename lands (PR #1875 review finding).
/// Proven the same way `put_logo_serializes_against_the_company_write_lock`
/// proves it: hold the lock externally, drive the real handler through the
/// router, and demand it cannot finish while the lock is held.
#[tokio::test]
async fn add_desk_member_serializes_against_the_company_write_lock() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let id = CompanyId::new("acme");

    let lock = company_write_lock(&id);
    let guard = lock.lock().await;

    let app_for_task = app.clone();
    let cookie_for_task = cookie.clone();
    let mut task = tokio::spawn(async move {
        app_for_task
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/desks/studio/members")
                    .header("cookie", &cookie_for_task)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agent_id":"eng"}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    });

    // The handler must be blocked behind the held lock — give it every
    // chance to (wrongly) race ahead before declaring it stuck.
    let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "add_desk_member completed while company_write_lock was held \
         elsewhere — it is not serializing its load-modify-save cycle \
         against concurrent `ops` writers"
    );

    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("add_desk_member never resumed after the lock was released")
        .expect("add_desk_member task panicked");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// `set_desk_order` must serialize against `company_write_lock` too — same
/// load-modify-save shape and same finding as `add_desk_member`'s own test
/// above (PR #1875 review finding, round 9: the earlier fix covered five
/// handlers but this coverage only proved it for one).
#[tokio::test]
async fn set_desk_order_serializes_against_the_company_write_lock() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let id = CompanyId::new("acme");
    seed_overlay_eng(&app, &cookie).await;

    let lock = company_write_lock(&id);
    let guard = lock.lock().await;

    let app_for_task = app.clone();
    let cookie_for_task = cookie.clone();
    let mut task = tokio::spawn(async move {
        put_desk_order(
            &app_for_task,
            &cookie_for_task,
            "studio",
            r#"{"ordered_member_ids":["eng","ceo"]}"#,
        )
        .await
    });

    let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "set_desk_order completed while company_write_lock was held \
         elsewhere — it is not serializing its load-modify-save cycle \
         against concurrent `ops` writers"
    );

    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("set_desk_order never resumed after the lock was released")
        .expect("set_desk_order task panicked");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// `remove_desk_member` must serialize against `company_write_lock` too
/// (PR #1875 review finding, round 9 — see
/// `set_desk_order_serializes_against_the_company_write_lock`'s own doc).
#[tokio::test]
async fn remove_desk_member_serializes_against_the_company_write_lock() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let id = CompanyId::new("acme");
    seed_overlay_eng(&app, &cookie).await;

    let lock = company_write_lock(&id);
    let guard = lock.lock().await;

    let app_for_task = app.clone();
    let cookie_for_task = cookie.clone();
    let mut task = tokio::spawn(async move {
        app_for_task
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/company/desks/studio/members/eng")
                    .header("cookie", &cookie_for_task)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    });

    let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "remove_desk_member completed while company_write_lock was held \
         elsewhere — it is not serializing its load-modify-save cycle \
         against concurrent `ops` writers"
    );

    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("remove_desk_member never resumed after the lock was released")
        .expect("remove_desk_member task panicked");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// `create_desk` must serialize against `company_write_lock` too (PR #1875
/// review finding, round 9 — see
/// `set_desk_order_serializes_against_the_company_write_lock`'s own doc).
#[tokio::test]
async fn create_desk_serializes_against_the_company_write_lock() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let id = CompanyId::new("acme");

    let lock = company_write_lock(&id);
    let guard = lock.lock().await;

    let app_for_task = app.clone();
    let cookie_for_task = cookie.clone();
    let mut task = tokio::spawn(async move {
        app_for_task
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/desks")
                    .header("cookie", &cookie_for_task)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"Growth","members":["eng"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    });

    let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "create_desk completed while company_write_lock was held \
         elsewhere — it is not serializing its load-modify-save cycle \
         against concurrent `ops` writers"
    );

    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("create_desk never resumed after the lock was released")
        .expect("create_desk task panicked");
    assert_eq!(status, StatusCode::CREATED);
}

/// `delete_desk` must serialize against `company_write_lock` too (PR #1875
/// review finding, round 9 — see
/// `set_desk_order_serializes_against_the_company_write_lock`'s own doc).
#[tokio::test]
async fn delete_desk_serializes_against_the_company_write_lock() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let id = CompanyId::new("acme");

    // Create the overlay desk to delete before taking the lock — this
    // test proves serialization on the delete path, not the create path.
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Growth","members":["eng"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let lock = company_write_lock(&id);
    let guard = lock.lock().await;

    let app_for_task = app.clone();
    let cookie_for_task = cookie.clone();
    let mut task = tokio::spawn(async move {
        app_for_task
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/company/desks/growth")
                    .header("cookie", &cookie_for_task)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    });

    let raced_ahead = tokio::time::timeout(std::time::Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "delete_desk completed while company_write_lock was held \
         elsewhere — it is not serializing its load-modify-save cycle \
         against concurrent `ops` writers"
    );

    drop(guard);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("delete_desk never resumed after the lock was released")
        .expect("delete_desk task panicked");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// A desk that declares no `hive` block still reports the numbers the
/// runtime would derive, rather than blanks.
///
/// This is the difference the whole DTO exists for: the manifest says
/// nothing, so `declared` is empty — but the desk would still run on a
/// budget and a quorum, and a console showing an empty form would be
/// describing a desk that does not exist.
#[tokio::test]
async fn desk_hive_reports_derived_numbers_for_an_undeclared_block() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"a\"\nrole = \"A\"\n\
         [[agent]]\nid = \"b\"\nrole = \"B\"\n\
         [[agent]]\nid = \"c\"\nrole = \"C\"\n\
         [[group_chat]]\nid = \"solvers\"\nname = \"Solvers\"\nmembers = [\"a\", \"b\", \"c\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/desks/solvers/hive")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();

    assert_eq!(body["source"], "manifest");
    assert_eq!(body["deliberates"], true);
    // Three seats: 3 x members, and a majority that still leaves somebody out.
    assert_eq!(body["effective"]["turnBudget"], 9);
    assert_eq!(body["effective"]["quorum"], 2);
    // Nothing was declared, so the authored block is empty — which is
    // exactly what distinguishes it from an operator who wrote `9`.
    assert_eq!(body["declared"], serde_json::json!({}));
    // Every seat holds every move until a table narrows one.
    assert_eq!(body["seats"].as_array().unwrap().len(), 3);
    assert_eq!(body["seats"][0]["governed"], false);
    assert_eq!(body["seats"][0]["moves"].as_array().unwrap().len(), 9);
    assert_eq!(body["eligibleSupporters"], 3);
    assert_eq!(body["reachesQuorum"], true);
}

/// Installing a grammar takes effect, is reported back derived, and is
/// undone by a reset — without the manifest ever being rewritten.
#[tokio::test]
async fn a_move_grammar_installs_and_resets() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"a\"\nrole = \"A\"\n\
         [[agent]]\nid = \"b\"\nrole = \"B\"\n\
         [[agent]]\nid = \"c\"\nrole = \"C\"\n\
         [[group_chat]]\nid = \"solvers\"\nname = \"Solvers\"\nmembers = [\"a\", \"b\", \"c\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let install = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/company/desks/solvers/hive")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"quorum":2,"moves":{"a":["propose","support"],"b":["object","evidence"],"c":["support"]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(install.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(install.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["source"], "overlay");
    assert_eq!(body["effective"]["quorum"], 2);
    // `b` was narrowed to object/evidence, and still keeps the three no
    // table can take away.
    let b = body["seats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|seat| seat["agentId"] == "b")
        .unwrap()
        .clone();
    assert_eq!(b["governed"], true);
    let b_moves: Vec<String> = serde_json::from_value(b["moves"].clone()).unwrap();
    assert!(b_moves.contains(&"commit".to_string()));
    assert!(b_moves.contains(&"question".to_string()));
    assert!(b_moves.contains(&"defer".to_string()));
    assert!(!b_moves.contains(&"propose".to_string()));
    // a and c may support or propose; b may not. Two clears a quorum of two.
    assert_eq!(body["eligibleSupporters"], 2);
    assert_eq!(body["reachesQuorum"], true);

    let reset = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/solvers/hive")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(reset.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    // Back to the blueprint, which declared nothing.
    assert_eq!(body["source"], "manifest");
    assert_eq!(body["declared"], serde_json::json!({}));
    assert_eq!(body["eligibleSupporters"], 3);
}

/// The runtime refuses exactly what a manifest carrying the same block
/// would be refused for — and in the same words.
#[tokio::test]
async fn installing_an_unreachable_quorum_is_refused() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"a\"\nrole = \"A\"\n\
         [[agent]]\nid = \"b\"\nrole = \"B\"\n\
         [[agent]]\nid = \"c\"\nrole = \"C\"\n\
         [[group_chat]]\nid = \"solvers\"\nname = \"Solvers\"\nmembers = [\"a\", \"b\", \"c\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Only `a` may deposit a supporter, but the quorum asks for two — the
    // room could never decide anything however much it agreed.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/company/desks/solvers/hive")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"quorum":2,"moves":{"a":["propose"],"b":["object"],"c":["object"]}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // An unknown move kind, and an id that is not on the desk, are refused
    // for the same reason: both fail *open* at runtime, handing the seat
    // every move and letting the desk quietly go on voting.
    for body in [
        r#"{"moves":{"a":["shrug"]}}"#,
        r#"{"moves":{"ghost":["propose"]}}"#,
    ] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/company/desks/solvers/hive")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::BAD_REQUEST,
            "body {body} was accepted"
        );
    }
}

/// The structural rows a console draws its activity graph from.
///
/// Before these, "who created this desk" and "who moved this seat" were
/// answerable only from a live frame that does not survive a reload.
#[tokio::test]
async fn desk_lifecycle_is_journaled() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let events = runtime.events();
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    for (method, uri, body) in [
        (
            "POST",
            "/api/v1/company/team",
            Some(r#"{"name":"Dana","role":"Analyst"}"#),
        ),
        (
            "POST",
            "/api/v1/company/desks",
            Some(r#"{"name":"Growth","members":["eng"]}"#),
        ),
        (
            "POST",
            "/api/v1/company/desks/growth/members",
            Some(r#"{"agent_id":"ceo"}"#),
        ),
        (
            "PUT",
            "/api/v1/company/desks/growth/hive",
            Some(r#"{"quorum":1}"#),
        ),
        ("DELETE", "/api/v1/company/desks/growth/hive", None),
        ("DELETE", "/api/v1/company/desks/growth/members/ceo", None),
        ("DELETE", "/api/v1/company/desks/growth", None),
    ] {
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header("cookie", &cookie);
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let res = app
            .clone()
            .oneshot(req.body(body.map_or(Body::empty(), Body::from)).unwrap())
            .await
            .unwrap();
        assert!(
            res.status().is_success(),
            "{method} {uri} answered {}",
            res.status()
        );
    }

    let rows = events
        .read_from(&CompanyId::new("acme"), EventSeq::new(0), 500)
        .await
        .unwrap();
    let kinds: Vec<&str> = rows.iter().map(|row| row.event.kind()).collect();
    assert!(kinds.contains(&"DeskCreated"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"DeskDeleted"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"TeammateAdded"), "kinds: {kinds:?}");
    assert_eq!(
        kinds.iter().filter(|k| **k == "DeskHiveConfigured").count(),
        2,
        "one row for install and one for reset: {kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == "DeskMembersChanged").count(),
        2,
        "one row for the add and one for the remove: {kinds:?}"
    );
}

/// Deleting a desk takes its installed grammar with it.
///
/// Left behind, an overlay desk re-created with the same id silently
/// inherits a table nobody installed on it.
#[tokio::test]
async fn deleting_a_desk_drops_its_installed_grammar() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Growth","members":["eng","ceo"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let installed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/company/desks/growth/hive")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"moves":{"eng":["propose","support"]}}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(installed.status(), StatusCode::OK);

    let deleted = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/growth")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    // Re-create the same id; it must come back ungoverned.
    let again = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Growth","members":["eng","ceo"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::CREATED);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/desks/growth/hive")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["source"], "default");
    for seat in body["seats"].as_array().unwrap() {
        assert_eq!(
            seat["governed"], false,
            "a re-created desk inherited a grammar"
        );
    }
}

/// Removing an overlay member drops it from the merged view; a manifest
/// member cannot be removed (409), and an unknown overlay member is a 404.
#[tokio::test]
async fn remove_desk_member_drops_overlay_and_guards_manifest() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Seed an overlay member.
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks/studio/members")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"agent_id":"eng"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // Removing a manifest member is a 409.
    let manifest_remove = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/studio/members/ceo")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(manifest_remove.status(), StatusCode::CONFLICT);

    // Removing the overlay member succeeds and drops it from the list.
    let remove = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/studio/members/eng")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(remove.status(), StatusCode::NO_CONTENT);

    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["members"].as_array().unwrap().len(), 1);
    assert!(desks[0].get("overlayMembers").is_none());

    // Removing it again is a 404 (no such overlay member).
    let gone = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/studio/members/eng")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

/// Creates an overlay desk through the same route `create_desk` serves and
/// returns its derived id, so the desk under test exists only in the overlay
/// — nothing about it is declared in the manifest.
async fn seed_overlay_desk(app: &axum::Router, cookie: &str, body: &str) -> String {
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let bytes = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["id"].as_str().unwrap().to_string()
}

async fn post_desk_member(
    app: &axum::Router,
    cookie: &str,
    desk: &str,
    body: &str,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/desks/{desk}/members"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn delete_desk_member(
    app: &axum::Router,
    cookie: &str,
    desk: &str,
    agent: &str,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/company/desks/{desk}/members/{agent}"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Reads the `error` string out of an api.md error envelope.
async fn error_message(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["error"].as_str().unwrap().to_string()
}

/// Returns the effective member list of `desk` from `list_desks`.
async fn desk_members(app: &axum::Router, cookie: &str, desk: &str) -> Vec<String> {
    let desks = get_desks(app, cookie).await;
    desks
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == desk)
        .unwrap_or_else(|| panic!("desk {desk} present in list"))["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect()
}

/// A desk that exists only in the operator overlay can be staffed and
/// unstaffed like a manifest desk. Both membership handlers used to test the
/// manifest alone, so a console-created desk could be reordered and deleted
/// but never gain or lose a member (#833). Every other desk test seeds its
/// desk from the manifest, so only an overlay-created desk exercises this.
#[tokio::test]
async fn desk_member_writes_reach_an_overlay_created_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let desk =
        seed_overlay_desk(&app, &cookie, r#"{"name":"Growth desk","members":["ceo"]}"#).await;
    assert_eq!(desk, "growth_desk");
    assert_eq!(desk_members(&app, &cookie, &desk).await, ["ceo"]);

    let added = post_desk_member(&app, &cookie, &desk, r#"{"agent_id":"eng"}"#).await;
    assert_eq!(added.status(), StatusCode::NO_CONTENT);
    assert_eq!(desk_members(&app, &cookie, &desk).await, ["ceo", "eng"]);

    let removed = delete_desk_member(&app, &cookie, &desk, "eng").await;
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    assert_eq!(desk_members(&app, &cookie, &desk).await, ["ceo"]);
}

/// An unknown desk id is refused as a missing desk, not a missing company.
/// The refusal used to be raised as `CompanyNotFound("desk ghost")`, which
/// rendered as `company not found: desk ghost` — the wrong resource, and the
/// desk id stuffed into a company id slot (#833). The status stays `404`
/// because both variants map there.
#[tokio::test]
async fn unknown_desk_member_writes_refuse_as_a_missing_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let added = post_desk_member(&app, &cookie, "ghost", r#"{"agent_id":"eng"}"#).await;
    assert_eq!(added.status(), StatusCode::NOT_FOUND);
    let message = error_message(added).await;
    assert!(
        !message.contains("company not found"),
        "add refusal blames the company: {message:?}"
    );
    assert!(message.contains("ghost"), "add refusal drops the desk id");

    let removed = delete_desk_member(&app, &cookie, "ghost", "eng").await;
    assert_eq!(removed.status(), StatusCode::NOT_FOUND);
    let message = error_message(removed).await;
    assert!(
        !message.contains("company not found"),
        "remove refusal blames the company: {message:?}"
    );
    assert!(
        message.contains("ghost"),
        "remove refusal drops the desk id"
    );
}

/// Creating a desk persists it as an overlay and surfaces it in `list_desks`
/// alongside the manifest desks, flagged `overlayCreated` with its lead
/// first. The manifest is never rewritten.
#[tokio::test]
async fn create_desk_persists_and_appears_in_list() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"Growth desk","description":"Acquisition.","members":["eng","ceo"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let bytes = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Id derived from the name; the first member is the lead.
    assert_eq!(body["id"], "growth_desk");
    assert_eq!(body["name"], "Growth desk");
    assert_eq!(body["overlayCreated"], true);
    assert_eq!(body["members"][0], "eng");
    assert_eq!(body["members"][1], "ceo");

    // The list now carries the manifest desk and the created overlay desk.
    // The Operator feed is its own surface (issue #1757 rework) — it is
    // fetched through `GET {scope}/operator-channel`, not injected here.
    let desks = get_desks(&app, &cookie).await;
    let arr = desks.as_array().unwrap();
    assert_eq!(arr.len(), 2, "{arr:?}");
    assert_eq!(arr[0]["id"], "studio"); // manifest desk first
    assert_eq!(arr[1]["id"], "growth_desk");
    assert_eq!(arr[1]["overlayCreated"], true);
}

/// Issue #1835, both wire directions. A create that never mentions
/// `responder` — every existing caller, and the org chart today — answers
/// and lists with **no** `responder` key at all, so old consoles see the
/// pre-#1835 shape byte-for-byte. A create with `responder: "auto"`
/// answers and lists `"auto"`, and the mode survives the store round-trip
/// rather than collapsing back to a lead desk.
#[tokio::test]
async fn create_desk_carries_the_responder_mode_and_omits_the_default() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let post = |body: &'static str| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/company/desks")
                        .header("cookie", &cookie)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        }
    };

    let lead = post(r#"{"name":"Growth desk","members":["eng"]}"#).await;
    assert!(
        lead.get("responder").is_none(),
        "a mode never stated must not appear on the wire: {lead}"
    );
    let auto =
        post(r#"{"name":"Launch week","members":["eng","ceo"],"responder":"auto"}"#).await;
    assert_eq!(auto["responder"], "auto", "{auto}");

    // The list re-reads the store, so this is the round-trip half: the
    // manifest desk and the defaulted create stay keyless, the channel
    // keeps its mode.
    let desks = get_desks(&app, &cookie).await;
    let arr = desks.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr[0].get("responder").is_none(), "manifest desk: {desks}");
    assert!(
        arr[1].get("responder").is_none(),
        "defaulted create: {desks}"
    );
    assert_eq!(arr[2]["responder"], "auto", "{desks}");
}

/// Issue #1835, codex review: an `auto` channel cannot be created empty —
/// the selector would have no candidates and the first-member fallback no
/// first member, so its unmentioned messages would silently fall to the
/// orchestrator, contradicting the channel's own model. A **lead** desk
/// keeps its right to start empty and be staffed from the org chart.
/// Revert the guard in `create_desk` and the first assertion answers 201.
#[tokio::test]
async fn an_auto_channel_cannot_be_created_empty_but_a_lead_desk_still_can() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let post = |body: &'static str| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/desks")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    let refused = post(r#"{"name":"Launch week","responder":"auto"}"#).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(refused.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes).to_string();
    assert!(
        body.contains("at least one member"),
        "the refusal names the reason, not a generic 400: {body}"
    );

    let empty_lead = post(r#"{"name":"Someday desk"}"#).await;
    assert_eq!(
        empty_lead.status(),
        StatusCode::CREATED,
        "an empty lead desk is still legal — it gains members from the org chart"
    );
}

/// Create-desk validation: an empty name is 400, an id colliding with a
/// manifest desk is 409, an unknown member is 400, and — issue #1757 — an
/// id (explicit or name-derived) colliding with the reserved `operator`
/// system channel is 409 even though it is not a manifest or overlay desk
/// `desk_exists` would otherwise catch.
#[tokio::test]
async fn create_desk_validates_name_id_and_members() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let cases = [
        (r#"{"name":"   "}"#, StatusCode::BAD_REQUEST),
        (r#"{"name":"Studio","id":"studio"}"#, StatusCode::CONFLICT),
        (
            r#"{"name":"Ghost desk","members":["ghost"]}"#,
            StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"name":"Operator","id":"operator"}"#,
            StatusCode::CONFLICT,
        ),
        (r#"{"name":"operator"}"#, StatusCode::CONFLICT),
        // PR #1781 review (CodeRabbit P2 follow-up to `316bc9229`): the id
        // guard alone lets a display-name collision through — `{"id":
        // "ops", "name": "Operator"}` never touches the reserved id, but
        // `resolve_desk_id` would still fold a `?desk=Operator` selector
        // onto this desk exactly as it would onto one literally named
        // `operator`. Same shape for the collision-fallback display name.
        (r#"{"name":"Operator","id":"ops"}"#, StatusCode::CONFLICT),
        (
            r#"{"name":"operator-feed","id":"ops2"}"#,
            StatusCode::CONFLICT,
        ),
        // Issue #1743 / PR #1781 review: a desk claiming a General
        // spelling — by id or by display name — would shadow the
        // built-in `#general` channel exactly as an `operator`-id desk
        // shadows the Operator feed.
        (r#"{"name":"Ops","id":"general"}"#, StatusCode::CONFLICT),
        (r#"{"name":"Ops","id":"main"}"#, StatusCode::CONFLICT),
        (r#"{"name":"General"}"#, StatusCode::CONFLICT),
        (r#"{"name":"Main"}"#, StatusCode::CONFLICT),
    ];
    for (body, want) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/desks")
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), want, "body {body}");
    }
}

/// Deleting an operator-created desk drops it (and any of its overlay
/// members); a manifest desk cannot be deleted (409); an unknown id is 404.
#[tokio::test]
async fn delete_desk_removes_overlay_and_guards_manifest() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Create an overlay desk to delete.
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Growth","members":["eng"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    // A manifest desk cannot be deleted.
    let manifest_delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/studio")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(manifest_delete.status(), StatusCode::CONFLICT);

    // The overlay desk deletes and drops out of the list.
    let delete = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/growth")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let desks = get_desks(&app, &cookie).await;
    // Only the manifest desk remains — the Operator feed is its own
    // surface now (issue #1757 rework), not injected into this list.
    let arr = desks.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{arr:?}");
    assert_eq!(arr[0]["id"], "studio");

    // Deleting it again is a 404.
    let gone = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/growth")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

/// Issue #1781 review (Codex P2): deleting a legacy overlay desk that was
/// holding `operator_feed_channel()` on the fallback address must not let
/// it revert to `OPERATOR_CHANNEL`.
///
/// `desk_exists`/`resolve_desk_id` are live checks — with no tombstone,
/// removing the colliding desk makes them stop matching, so the divert
/// would silently flip back the moment `delete_desk` succeeds. Seeded
/// directly on the stored record rather than through `POST .../desks`
/// (as `list_desks_hides_an_overlay_desk_shadowing_general` does for its
/// own General case): `create_desk`'s own guard has refused the id and
/// name `operator` since `316bc9229`, so this shape can only be reached
/// by an overlay desk that predates it — exactly what this proves stays
/// safe to delete.
#[tokio::test]
async fn delete_desk_keeps_the_operator_feed_diverted_after_the_collision_is_gone() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut record = runtime.store().load(&id).await.unwrap().unwrap();
    record.overlay_desks.push(OverlayDesk {
        id: "operator".to_string(),
        name: "Legacy Ops".to_string(),
        description: None,
        members: vec![],
        responder: ResponderMode::Lead,
        hive: Default::default(),
    });
    runtime.store().save(&record).await.unwrap();

    let reloaded = runtime.store().load(&id).await.unwrap().unwrap();
    assert_eq!(
        reloaded.operator_feed_channel(),
        crate::runtime::channel::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "fixture must start in the collision state this test exercises"
    );

    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let delete = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/operator")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let after = runtime.store().load(&id).await.unwrap().unwrap();
    assert!(
        !after.desk_exists(crate::runtime::channel::OPERATOR_CHANNEL),
        "the colliding desk must actually be gone, or this is not \
         exercising the live-check-flips-back failure mode at all"
    );
    assert_eq!(
        after.operator_feed_channel(),
        crate::runtime::channel::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "the feed address must stay on the fallback once the desk that \
         caused the collision is deleted — flipping back to \
         OPERATOR_CHANNEL would orphan every report already journaled \
         under the fallback and let the deleted desk's own historical \
         transcript (chat_id == \"operator\") resurface as system-feed \
         content"
    );
}

/// Add-member validation: an unknown desk is 404, an unknown teammate is
/// 400, and a teammate already on the desk is 409.
#[tokio::test]
async fn add_desk_member_validates_desk_agent_and_duplicates() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let cases = [
        (
            "/api/v1/company/desks/ghost/members",
            r#"{"agent_id":"eng"}"#,
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/v1/company/desks/studio/members",
            r#"{"agent_id":"ghost"}"#,
            StatusCode::BAD_REQUEST,
        ),
        // `ceo` is already a manifest member of `studio`.
        (
            "/api/v1/company/desks/studio/members",
            r#"{"agent_id":"ceo"}"#,
            StatusCode::CONFLICT,
        ),
    ];
    for (uri, body, want) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), want, "{uri} {body}");
    }
}

/// Seeds `eng` as an overlay member of `studio` so a desk has two members to
/// reorder.
async fn seed_overlay_eng(app: &axum::Router, cookie: &str) {
    let add = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks/studio/members")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"agent_id":"eng"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);
}

async fn put_desk_order(
    app: &axum::Router,
    cookie: &str,
    desk: &str,
    body: &str,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/company/desks/{desk}/order"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// A `PUT .../order` reorders the desk; the change surfaces in `list_desks`
/// as the new `members` order (the hierarchy), and an empty body resets it.
#[tokio::test]
async fn set_desk_order_reorders_and_resets() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    seed_overlay_eng(&app, &cookie).await;

    // Base order is manifest-first: ceo, then the overlay eng.
    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["members"][0], "ceo");
    assert_eq!(desks[0]["members"][1], "eng");

    // Promote the overlay member to the lead slot.
    let status = put_desk_order(
        &app,
        &cookie,
        "studio",
        r#"{"ordered_member_ids":["eng","ceo"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["members"][0], "eng");
    assert_eq!(desks[0]["members"][1], "ceo");

    // An empty body clears the override, restoring the blueprint order.
    let status = put_desk_order(&app, &cookie, "studio", r#"{"ordered_member_ids":[]}"#).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["members"][0], "ceo");
    assert_eq!(desks[0]["members"][1], "eng");
}

/// An operator-created (overlay) desk can be reordered too — the set-order
/// handler validates existence with `desk_exists`, which covers overlay desks,
/// not just manifest group chats. A manifest-only check used to 404 here (#133).
#[tokio::test]
async fn set_desk_order_reorders_an_overlay_created_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Create an overlay desk with two members (lead is `ceo` by declaration).
    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/desks")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"Growth desk","members":["ceo","eng"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    // Reordering the overlay desk succeeds (not 404) and promotes `eng`.
    let status = put_desk_order(
        &app,
        &cookie,
        "growth_desk",
        r#"{"ordered_member_ids":["eng","ceo"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The new hierarchy surfaces in the list for the overlay desk.
    let desks = get_desks(&app, &cookie).await;
    let growth = desks
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "growth_desk")
        .expect("overlay desk present");
    assert_eq!(growth["members"][0], "eng");
    assert_eq!(growth["members"][1], "ceo");
}

/// Set-order validation: an unknown desk is 404, an unknown member id is 400,
/// and a duplicate id is 400.
#[tokio::test]
async fn set_desk_order_validates_desk_members_and_duplicates() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    seed_overlay_eng(&app, &cookie).await;

    // Unknown desk → 404.
    assert_eq!(
        put_desk_order(&app, &cookie, "ghost", r#"{"ordered_member_ids":["ceo"]}"#).await,
        StatusCode::NOT_FOUND
    );
    // A non-member id → 400.
    assert_eq!(
        put_desk_order(
            &app,
            &cookie,
            "studio",
            r#"{"ordered_member_ids":["ceo","ghost"]}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // Duplicate id → 400.
    assert_eq!(
        put_desk_order(
            &app,
            &cookie,
            "studio",
            r#"{"ordered_member_ids":["ceo","ceo"]}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );
}

/// Removing an overlay member prunes it from the desk's order overlay, so the
/// remaining members keep the operator's relative order without a stale id.
#[tokio::test]
async fn remove_desk_member_prunes_the_order_entry() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");
    seed_overlay_eng(&app, &cookie).await;

    // Reorder to [eng, ceo], then remove eng.
    assert_eq!(
        put_desk_order(
            &app,
            &cookie,
            "studio",
            r#"{"ordered_member_ids":["eng","ceo"]}"#
        )
        .await,
        StatusCode::NO_CONTENT
    );
    let remove = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/desks/studio/members/eng")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(remove.status(), StatusCode::NO_CONTENT);

    // Only the manifest member remains; the order entry is gone (no stale
    // eng lingering), so ceo is the lead.
    let desks = get_desks(&app, &cookie).await;
    assert_eq!(desks[0]["members"].as_array().unwrap().len(), 1);
    assert_eq!(desks[0]["members"][0], "ceo");
}

#[tokio::test]
async fn desks_route_returns_the_company_desks() {
    // The default test manifest defines no group chats, so the route
    // answers 200 with an empty list — the console falls back to its
    // static default threads. The Operator feed is a separate surface
    // (issue #1757 rework), fetched through `GET
    // {scope}/operator-channel`, and no longer folded into this list.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/desks")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let desks = value.as_array().unwrap();
    assert!(desks.is_empty(), "{desks:?}");
}

async fn get_operator_channel(app: &axum::Router, cookie: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/operator-channel")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Issue #1757 rework: `GET {scope}/operator-channel` returns the
/// dedicated feed's identity — never folded into `list_desks` any more —
/// and `list_desks` carries zero operator logic: the real desks are all
/// it returns.
#[tokio::test]
async fn operator_channel_route_returns_the_feed_identity_and_is_absent_from_desks() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let channel = get_operator_channel(&app, &cookie).await;
    assert_eq!(channel["id"], "operator");
    assert_eq!(channel["name"], "Operator");
    assert!(
        channel["description"]
            .as_str()
            .unwrap()
            .contains("what happened"),
        "{channel}"
    );

    let desks = get_desks(&app, &cookie).await;
    assert!(
        desks
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["id"] != "operator"),
        "list_desks must carry zero operator logic: {desks:?}"
    );
}

/// Issue #1757 rework: the always-present Operator feed is its own
/// surface — `GET {scope}/operator-channel` names it, `list_desks` never
/// does — and posting to it is still refused (it is a read-only report
/// feed).
#[tokio::test]
async fn the_operator_channel_is_a_separate_surface_and_stays_read_only() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let desks = get_desks(&app, &cookie).await;
    let desks = desks.as_array().unwrap();
    let ids: Vec<&str> = desks.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["studio"], "list_desks carries only real desks");

    let channel = get_operator_channel(&app, &cookie).await;
    assert_eq!(channel["id"], "operator");
    assert_eq!(channel["name"], "Operator");

    // A send addressed to it is refused (read-only), never journaled.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi","chat":"operator"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_client_error(),
        "posting to the operator channel must be refused, got {}",
        response.status()
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes).to_lowercase();
    assert!(body.contains("read-only"), "{body}");
}

/// Issue #1781 review (CodeRabbit): `CompanyRuntime::ensure_desk_writable`
/// re-loads the record on every operator-channel send (to catch a
/// grandfathered desk/teammate colliding with the reserved id) and
/// propagates a real `store().load` failure with `?` rather than folding
/// it into "no real recipient". Collapsing it would misreport a store
/// outage as the ordinary read-only refusal — same 4xx, same message,
/// same "read-only" wording an operator would wrongly believe.
///
/// Corrupting `company.toml` on disk after the app is built (rather than
/// mocking `CompanyStore`) exercises the real `FsCompanyStore::load`
/// error path — `Err(OpenCompanyError::Store("invalid company.toml: …"))`
/// — which has no `Store` arm in `ApiError::status` and therefore falls
/// to the catch-all `INTERNAL_SERVER_ERROR`. A collapsed-to-`false` read
/// would instead surface as `InvalidRequest` (400) with the read-only
/// wording, so the status code and body together distinguish the two.
#[tokio::test]
async fn a_failing_store_load_is_not_collapsed_into_the_read_only_refusal() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Corrupt the on-disk manifest so the next `store().load()` — the one
    // `ensure_desk_writable` runs fresh on every send — fails instead of
    // returning `Some(record)`.
    let toml_path = crate::store::Bundle::new(&home, &CompanyId::new("acme")).company_toml();
    tokio::fs::write(&toml_path, b"not valid toml [[[")
        .await
        .expect("corrupt company.toml");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi","chat":"operator"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a store load failure must propagate as itself, not the read-only 4xx"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8_lossy(&bytes).to_lowercase();
    assert!(
        !body.contains("read-only"),
        "a store outage must not be misreported as the ordinary read-only refusal: {body}"
    );
}

/// CodeRabbit review (PR #1781, P2): `operator_channel` used to fold a
/// `store().load()` failure into "no record" via `.ok().flatten()`, and
/// answer the default `operator` id anyway. For an upgraded company whose
/// grandfathered `operator` teammate requires the `operator-feed`
/// collision address, that silently mislabels the teammate's `operator`
/// transcript as the system feed while a transient outage lasts — and the
/// console would show it as healthy the whole time. This proves the fix:
/// a real load failure now propagates as an error instead of defaulting.
///
/// Corrupts `company.toml` on disk after the app is built (rather than
/// mocking `CompanyStore`) to exercise the real `FsCompanyStore::load`
/// error path — same technique as
/// `a_failing_store_load_is_not_collapsed_into_the_read_only_refusal`
/// above.
#[tokio::test]
async fn operator_channel_propagates_a_store_load_failure_instead_of_defaulting() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // Baseline: before any corruption, the route answers the default id.
    let channel = get_operator_channel(&app, &cookie).await;
    assert_eq!(channel["id"], "operator");

    // Corrupt the on-disk manifest so the next `store().load()` fails
    // instead of returning `Some(record)` or `None`.
    let toml_path = crate::store::Bundle::new(&home, &CompanyId::new("acme")).company_toml();
    tokio::fs::write(&toml_path, b"not valid toml [[[")
        .await
        .expect("corrupt company.toml");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/operator-channel")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a store load failure must propagate as itself, not the default operator id"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_ne!(
        body["id"], "operator",
        "a store outage must not be silently answered as the healthy default channel: {body}"
    );
}

/// Issue #1757 migration: `operator` was not a reserved id before this
/// issue, and a stored manifest is never re-validated on load
/// (`CompanyManifest::from_stored_toml` skips validation on purpose, so
/// tightening a rule never strands an already-running company) — so a
/// company provisioned earlier can already have a real `[[group_chat]]`
/// using that id. Built directly with `toml::from_str` (bypassing
/// `into_validated`, the same way a stored manifest reaches
/// `CompanyRuntime` without going through it) to stand in for exactly
/// that: data that predates the guard. Without the carve-outs in
/// `list_desks` and `chat_and_emit`, this desk would be shadowed by a
/// synthetic, read-only duplicate under the same id the moment this
/// feature shipped, and every send to it would be refused. This proves
/// it is grandfathered instead: listed once, not flagged `system`, and
/// still writable.
#[tokio::test]
async fn a_manifest_desk_predating_the_reserved_operator_id_stays_writable() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"operator\"\nname = \"Ops Room\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let desks = get_desks(&app, &cookie).await;
    let desks = desks.as_array().unwrap();
    assert_eq!(desks.len(), 1, "no duplicate synthetic entry: {desks:?}");
    assert_eq!(desks[0]["id"], "operator");
    assert_eq!(
        desks[0]["name"], "Ops Room",
        "the real desk's own name, not the synthetic channel's: {desks:?}"
    );
    assert!(
        desks[0].get("system").is_none(),
        "grandfathered desk is a real desk (system defaults false and is \
         omitted), not the system channel: {desks:?}"
    );

    // A send addressed to it must go through — this is the pre-existing
    // desk's own line, not the (absent) synthetic system channel.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"text":"ship the landing page","chat":"operator"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "a pre-existing desk that already owns the `operator` id must stay \
         writable, got {}",
        response.status()
    );
}

/// The name-collision sibling of the id-collision test above (issue #1781
/// review, Codex P1 follow-up): a manifest desk grandfathered onto the
/// **display name** `Operator` (`{ id = "legacy_ops", name = "Operator" }`)
/// rather than the literal id. `resolve_desk_id` — what every *read*
/// already resolves a `?desk=` selector through — matches this desk by
/// name just as thoroughly as the id-collision desk above is matched by
/// id, but `ensure_desk_writable` used to check the *raw* selector string
/// against `OPERATOR_CHANNEL` before any such resolution ran, so a send
/// addressed to the desk's own supported alias (`chat: "Operator"`,
/// case-insensitive) was refused as the read-only system feed — reachable
/// by name for reads, refused by name for writes, the exact mismatch
/// `create_desk`'s reservation comment (above) warns a desk can never be
/// addressed consistently under. A send addressed to the desk's real id
/// (`legacy_ops`) already sailed through either way, which this also
/// covers as the negative control.
#[tokio::test]
async fn a_manifest_desk_grandfathered_onto_the_operator_name_stays_writable() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"legacy_ops\"\nname = \"Operator\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // The desk's own real id still works — this was never broken.
    let by_id = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"by id","chat":"legacy_ops"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        by_id.status().is_success(),
        "a send addressed to the grandfathered desk's real id must stay writable, got {}",
        by_id.status()
    );

    // The desk's supported display-name alias must now work too.
    let by_name = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"by name","chat":"Operator"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        by_name.status().is_success(),
        "a send addressed to the grandfathered desk's own case-insensitive \
         `Operator` alias must resolve to the real desk, not the read-only \
         system feed, got {}",
        by_name.status()
    );
}

/// The fallback-address sibling of the test above (issue #1781 review,
/// Codex P2 follow-up): a manifest desk grandfathered onto the display
/// name `operator-feed` — `OPERATOR_CHANNEL_COLLISION_FALLBACK` itself —
/// rather than `Operator`. No desk or teammate here claims the *primary*
/// `operator` id or name, so `operator_feed_channel()` stays on the
/// literal address and never diverts; the fallback is purely this desk's
/// own pre-#1757 display name. `ensure_desk_writable` used to refuse the
/// fallback constant unconditionally, without resolving it through
/// `resolve_desk_id` first the way the primary branch does — so a send
/// addressed to this desk's own supported case-insensitive alias
/// (`chat: "operator-feed"`) was refused as if it named the synthetic
/// read-only system desk, even though nothing here is actually diverted.
/// A send to the desk's real id (`ops`) already sailed through either
/// way, which this also covers as the negative control.
#[tokio::test]
async fn a_manifest_desk_grandfathered_onto_the_fallback_name_stays_writable() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"ops\"\nname = \"operator-feed\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();
    let record = runtime.store().load(&id).await.unwrap().unwrap();
    assert_eq!(
        record.operator_feed_channel(),
        crate::runtime::channel::OPERATOR_CHANNEL,
        "fixture must NOT be in the diverted state — this proves the \
         fallback name is refused even with no primary collision at all, \
         which the diverted case above does not exercise"
    );
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // The desk's own real id still works — this was never broken.
    let by_id = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"by id","chat":"ops"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        by_id.status().is_success(),
        "a send addressed to the grandfathered desk's real id must stay writable, got {}",
        by_id.status()
    );

    // The desk's supported display-name alias must now work too.
    let by_name = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"by name","chat":"operator-feed"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        by_name.status().is_success(),
        "a send addressed to the grandfathered desk's own case-insensitive \
         `operator-feed` alias must resolve to the real desk, not the \
         read-only system feed, got {}",
        by_name.status()
    );
}

/// Issue #1757 migration, the other namespace: a **teammate**, not a desk,
/// already named `operator`. `ChatView` addresses a DM by the teammate's
/// bare id (issue #364), so a message meant for this person also arrives
/// here as `chat == "operator"` — the same shape as a send meant for the
/// system feed. `desk_exists` alone cannot tell them apart: it only walks
/// `group_chats` and `overlay_desks`, never the roster, so a company that
/// named a manifest agent "Operator" before this feature shipped would
/// find that teammate's DM permanently refused, with the console giving no
/// way to rename or migrate out of the collision (`RESERVED_AGENT_IDS` and
/// `mint_agent_id` only stop a *future* mint). `is_roster_agent` closes the
/// same gap `desk_exists` closes for desks.
#[tokio::test]
async fn a_manifest_agent_predating_the_reserved_operator_id_stays_dm_able() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"operator\"\nrole = \"Chief of Staff\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // A DM addressed to the grandfathered teammate — by its bare id, the
    // same address `ChatView` sends — must go through rather than be
    // refused as a send to the read-only system channel.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"text":"status update please","chat":"operator"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "a pre-existing teammate that already owns the `operator` id must \
         stay DM-able, got {}",
        response.status()
    );
}

/// Issue #1757 rework, the read side of the grandfather case the test
/// above covers on the write side: a company whose roster names a
/// teammate `operator` (no desk of the same id) must have `GET
/// {scope}/operator-channel` answer at the disjoint collision-fallback
/// id, not the literal `operator` one — a direct post to the visible
/// read-only feed and the teammate's own DM must stay distinguishable
/// (`chat_id == "operator"` for the DM, the fallback id for the feed) —
/// and that fallback id must itself stay refused as read-only.
#[tokio::test]
async fn the_operator_channel_diverts_off_a_grandfathered_teammates_operator_line() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"operator\"\nrole = \"Chief of Staff\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let app = router(state.clone());
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let channel = get_operator_channel(&app, &cookie).await;
    assert_eq!(
        channel["id"],
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "the feed must not claim the literal `operator` id once a \
         teammate already holds it: {channel:?}"
    );

    // list_desks carries no operator logic at all, so it is untouched by
    // this collision either way — nothing to assert there but its
    // absence of the teammate, which the DM test above already covers.

    // The disjoint fallback id is unmintable and system-only: a direct post
    // to it must stay refused exactly like the literal `operator` id is,
    // even though nothing minted it as a desk.
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"text":"hello","chat":"{}"}}"#,
                    crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the disjoint system-feed address must stay read-only"
    );
}

/// PR #1781 review (CodeRabbit): the same divert as the test above, for
/// the *other* grandfather shape — a real **desk** already owning
/// `operator` (see `a_manifest_desk_predating_the_reserved_operator_id_stays_writable`
/// for the write side of this same fixture). Left undiverted, `GET
/// {scope}/operator-channel` and `GET {scope}/desks` would answer the
/// same id for two different things: the console appends the pinned
/// Operator row *after* the desk section (`operatorSection`,
/// `frontend/src/views/ChatView.tsx`), so `findChannel` — first-section-match
/// — would resolve the pinned row to the desk, and every workflow report
/// would journal onto the desk's own transcript instead of a
/// distinguishable feed.
#[tokio::test]
async fn the_operator_channel_diverts_off_a_grandfathered_desks_own_operator_line() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let legacy_manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"operator\"\nname = \"Ops Room\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, legacy_manifest).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let desks = get_desks(&app, &cookie).await;
    let desks = desks.as_array().unwrap();
    assert_eq!(desks.len(), 1);
    assert_eq!(
        desks[0]["id"], "operator",
        "the desk itself must keep its own literal id: {desks:?}"
    );

    let channel = get_operator_channel(&app, &cookie).await;
    assert_eq!(
        channel["id"],
        crate::runtime::OPERATOR_CHANNEL_COLLISION_FALLBACK,
        "the pinned Operator row must not claim the literal `operator` id \
         once a desk already holds it — otherwise the console shows two \
         rows sharing one id and `findChannel` always resolves the pinned \
         row to the desk: {channel:?}"
    );
}

/// Issue #65: the console's default thread addresses sends with
/// `chat: "main"`, but pre-threading history and the synthetic operator
/// desk are keyed on `"General"`. A transcript spanning both ids — one
/// operator turn journaled under each — must read back as one history via
/// the REST route with no `?desk=` selector (the console's default read).
#[tokio::test]
async fn chat_history_route_reunifies_general_and_main_transcripts() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "General".to_string(),
                agent_id: "ceo".to_string(),
                text: "reply under General".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();
    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "main".to_string(),
                agent_id: "ceo".to_string(),
                text: "reply under main".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/chat/history")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let messages = value.as_array().unwrap();
    let texts: Vec<&str> = messages
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert!(
        texts.contains(&"reply under General"),
        "missing General-id reply: {texts:?}"
    );
    assert!(
        texts.contains(&"reply under main"),
        "missing main-id reply: {texts:?}"
    );
}

/// Everything one agent said and heard, for a company with one reply in it.
///
/// Fetched through the router so the assertion is about the wire, not about
/// the struct it was built from.
async fn session_rows(uri: &str) -> Vec<serde_json::Value> {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "General".to_string(),
                agent_id: "ceo".to_string(),
                text: "one turn, from one session".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "GET {uri}");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let rows = value.as_array().cloned().unwrap_or_default();
    assert!(!rows.is_empty(), "no session rows came back from {uri}");
    rows
}

/// The console must name the session the runtime actually uses.
///
/// Pinned to [`openhuman_session_key`] itself rather than to the literal
/// `"acme:ceo"`, because the property worth keeping is not the current
/// spelling — it is that there is only ever **one** spelling. A route that
/// built its own `format!` would pass a literal assertion on the day it was
/// written and go on passing it the day the minting function changed.
#[tokio::test]
async fn the_session_route_reports_the_key_openhuman_session_key_mints() {
    let expected = crate::session_key::openhuman_session_key(&CompanyId::new("acme"), "ceo");
    let rows = session_rows("/api/v1/companies/acme/agents/ceo/session").await;
    for row in &rows {
        assert_eq!(
            row.get("openhumanSessionKey").and_then(|k| k.as_str()),
            Some(expected.as_str()),
            "every row of one agent's session belongs to that one session: {row}"
        );
    }
}

/// The single-company alias resolves the same company, so it must report
/// the same session — an operator reading the same agent through the other
/// scope form is not looking at a second session.
#[tokio::test]
async fn both_scope_forms_of_the_session_route_report_the_same_key() {
    let scoped = session_rows("/api/v1/companies/acme/agents/ceo/session").await;
    let alias = session_rows("/api/v1/company/agents/ceo/session").await;
    assert_eq!(
        scoped[0].get("openhumanSessionKey"),
        alias[0].get("openhumanSessionKey"),
    );
}

/// The field reaches the browser under the name the console binds to.
/// `tsc` cannot check a hand-written interface against a Rust DTO; this is
/// that check, in the idiom the folded-aside test above set.
#[tokio::test]
async fn the_session_key_reaches_the_wire_as_camel_case() {
    let rows = session_rows("/api/v1/companies/acme/agents/ceo/session").await;
    assert!(
        rows[0].get("openhuman_session_key").is_none(),
        "snake_case would silently read as undefined in the console: {}",
        rows[0]
    );
    assert!(rows[0].get("openhumanSessionKey").is_some(), "{}", rows[0]);
}

/// Regression: a reply's tool-call timeline must survive a history reload —
/// switching threads and coming back reloads `chat/history`, which used to
/// return text only, so the steps vanished. They are now persisted on the
/// `AgentReply` and projected back through the DTO.
#[tokio::test]
async fn chat_history_route_rehydrates_reply_steps() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "main".to_string(),
                agent_id: "ceo".to_string(),
                text: "done".to_string(),
                steps: vec![TurnStep {
                    kind: crate::ports::types::TurnStepKind::ToolCall,
                    status: crate::ports::types::TurnStepStatus::Ok,
                    label: "Reading messages".to_string(),
                    detail: None,
                    elapsed_ms: Some(9),
                    ..TurnStep::default()
                }],
            },
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/chat/history")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let reply = value
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["text"] == "done")
        .expect("the reply is in history");
    assert_eq!(
        reply["steps"][0]["label"], "Reading messages",
        "the persisted timeline must ride back on the history DTO"
    );
    assert_eq!(reply["steps"][0]["status"], "ok");
    assert_eq!(reply["steps"][0]["elapsedMs"], 9);
}

/// A reply's produced-file buttons are durable transcript data, and the
/// projection must stop returning either kind once its target is gone.
#[tokio::test]
async fn chat_history_route_rehydrates_outputs_and_drops_deleted_targets() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let company = runtime.id().clone();

    runtime
        .workspace()
        .create(
            &company,
            &attachment_note_node("node-1", "launch-note.md"),
            Some("Launch notes"),
        )
        .await
        .unwrap();
    runtime
        .workspace()
        .create(
            &company,
            &attachment_note_node("node-2", "surviving-note.md"),
            Some("Keep this note"),
        )
        .await
        .unwrap();
    runtime
        .artifacts()
        .upsert(
            &company,
            &crate::ports::artifacts::ArtifactRecord::new(
                "artifact-1",
                "task-1",
                "Launch brief",
                crate::ports::artifacts::ArtifactKind::Markdown,
                "# Launch",
                "ceo",
                1,
            ),
        )
        .await
        .unwrap();
    runtime
        .events()
        .append(
            &company,
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: vec![
                    crate::ports::types::ChatOutput {
                        kind: crate::ports::types::ChatOutputKind::WorkspaceNode,
                        target_id: "node-1".to_string(),
                        title: "launch-note.md".to_string(),
                        task_id: None,
                        version: None,
                    },
                    crate::ports::types::ChatOutput {
                        kind: crate::ports::types::ChatOutputKind::WorkspaceNode,
                        target_id: "node-2".to_string(),
                        title: "surviving-note.md".to_string(),
                        task_id: None,
                        version: None,
                    },
                    crate::ports::types::ChatOutput {
                        kind: crate::ports::types::ChatOutputKind::Artifact,
                        target_id: "artifact-1".to_string(),
                        title: "Launch brief".to_string(),
                        task_id: Some("task-1".to_string()),
                        version: Some(1),
                    },
                ],
                chat_id: "main".to_string(),
                agent_id: "ceo".to_string(),
                text: "I wrote both files.".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();

    let history = |app: axum::Router| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/company/chat/history")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let app = router(state);
    let first = history(app.clone()).await;
    let reply = first
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["text"] == "I wrote both files.")
        .unwrap();
    assert_eq!(reply["outputs"].as_array().unwrap().len(), 3);
    assert_eq!(reply["outputs"][0]["targetId"], "node-1");
    assert_eq!(reply["outputs"][1]["targetId"], "node-2");
    assert_eq!(reply["outputs"][2]["kind"], "artifact");
    assert_eq!(reply["outputs"][2]["taskId"], "task-1");
    assert_eq!(reply["outputs"][2]["version"], 1);

    runtime
        .workspace()
        .delete(&company, "node-1")
        .await
        .unwrap();
    runtime
        .artifacts()
        .delete(&company, "artifact-1")
        .await
        .unwrap();

    let reloaded = history(app).await;
    let reply = reloaded
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["text"] == "I wrote both files.")
        .unwrap();
    let outputs = reply["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1, "only live targets may rehydrate: {reply}");
    assert_eq!(outputs[0]["targetId"], "node-2");
}

/// Issue #246: a reply that opened a board card must still say so after a
/// transcript reload. The "card opened" chip is rendered from `taskId`, and
/// a chip that exists only on the live POST response vanishes the moment
/// the operator switches threads and comes back — which is exactly when
/// they would go looking for it.
#[tokio::test]
async fn chat_history_route_rehydrates_the_card_a_reply_opened() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // The card has to actually be on the board: the history projection
    // reports `taskId` only for a card that still exists, so that a chip
    // cannot come back pointing at a card someone deleted (issue #984).
    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-77".to_string(),
                title: TaskTitle::authored("Draft the launch note"),
                note: None,
                column: crate::ports::tasks::COLUMN_TODO.to_string(),
                priority: "medium".to_string(),
                assignee: String::new(),
                updated_at_millis: 1,
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
            },
        )
        .await
        .unwrap();

    for (text, task_id) in [
        ("opened one", Some("t-77".to_string())),
        ("just talking", None),
    ] {
        runtime
            .events()
            .append(
                runtime.id(),
                CompanyEvent::AgentReply {
                    audience: Vec::new(),
                    mentions: Vec::new(),
                    mention_depth: 0,
                    parent: None,
                    task_id,
                    outputs: Vec::new(),
                    chat_id: "main".to_string(),
                    agent_id: "ceo".to_string(),
                    text: text.to_string(),
                    steps: Vec::new(),
                },
            )
            .await
            .unwrap();
    }

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/chat/history")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let messages = value.as_array().unwrap();

    let opened = messages
        .iter()
        .find(|m| m["text"] == "opened one")
        .expect("the card-opening reply is in history");
    assert_eq!(
        opened["taskId"], "t-77",
        "the chip's correlation key must ride back on the history DTO"
    );

    // A reply that opened nothing omits the key rather than sending null,
    // so no bubble grows a chip it should not have — and every message
    // journaled before this field existed reads back unchanged.
    let chatter = messages
        .iter()
        .find(|m| m["text"] == "just talking")
        .expect("the ordinary reply is in history");
    assert!(
        chatter.get("taskId").is_none(),
        "an ordinary chat reply must not carry a card: {chatter}"
    );
}

/// A desk id with no `?desk=` selector defaults to the operator/General
/// thread; an unaddressed thread id that neither matches a manifest desk
/// nor the General desk reads back empty rather than erroring.
#[tokio::test]
async fn chat_history_route_unknown_desk_is_empty_not_an_error() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/chat/history?desk=strategy")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 0);
}

/// Issue #862: REST history carries the same cursor/window contract as the
/// paginated GraphQL surface. A copilot replay can therefore ask for the
/// tail it needs without the route reading past its cursor.
#[tokio::test]
async fn chat_history_route_honors_before_and_limit() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let mut seqs = Vec::new();
    for text in ["oldest", "kept", "newest"] {
        seqs.push(
            runtime
                .events()
                .append(
                    runtime.id(),
                    CompanyEvent::AgentReply {
                        audience: Vec::new(),
                        mentions: Vec::new(),
                        mention_depth: 0,
                        parent: None,
                        task_id: None,
                        outputs: Vec::new(),
                        chat_id: "workflow-copilot:weekly_report".to_string(),
                        agent_id: "ceo".to_string(),
                        text: text.to_string(),
                        steps: Vec::new(),
                    },
                )
                .await
                .unwrap(),
        );
    }

    let uri = format!(
        "/api/v1/company/chat/history?desk=workflow-copilot:weekly_report&before={}&limit=1",
        seqs[2].value()
    );

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value[0]["text"], "kept");
    assert_eq!(value.as_array().unwrap().len(), 1);
}

/// A history cursor pages messages, not the current reaction state. A
/// toggle can be journaled after the cursor for a message still selected by
/// that cursor, and must therefore remain visible on the paged result.
#[tokio::test]
async fn chat_history_cursor_keeps_later_reactions_on_displayed_messages() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let message = runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "General".to_string(),
                agent_id: "ceo".to_string(),
                text: "kept".to_string(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();
    let cursor = runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::FeedbackFiled {
                note: "cursor marker".to_string(),
            },
        )
        .await
        .unwrap();
    runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::ReactionToggled {
                message_seq: message,
                emoji: "👍".to_string(),
                on: true,
                by: None,
            },
        )
        .await
        .unwrap();

    let response = router(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/company/chat/history?before={}&limit=1",
                    cursor.value()
                ))
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value[0]["id"], message.value().to_string());
    assert_eq!(value[0]["reactions"][0]["emoji"], "👍");
}

/* ---- issue #364: durable ids, threads, reactions, channel isolation ---- */

/// Posts a chat message and returns the decoded `ChatResponse` body.
async fn post_chat(app: &Router, cookie: &str, body: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "chat POST failed");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Reads a desk's history. `desk` empty reads the default General thread.
async fn get_history(app: &Router, cookie: &str, desk: &str) -> Vec<serde_json::Value> {
    let uri = if desk.is_empty() {
        "/api/v1/company/chat/history".to_string()
    } else {
        format!("/api/v1/company/chat/history?desk={desk}")
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value.as_array().cloned().unwrap_or_default()
}

/// Sets or clears one reaction, returning the status.
async fn post_reaction(
    app: &Router,
    cookie: &str,
    seq: &str,
    emoji: &str,
    on: bool,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/chat/messages/{seq}/reactions"))
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "emoji": emoji, "on": on }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// The enabler for everything else in #364: a sent message comes back with
/// the durable id it was journaled under, on both halves of the exchange —
/// the operator's own line and each reply — and those ids are the same ones
/// `chat/history` returns on the next read.
#[tokio::test]
async fn chat_response_carries_durable_message_ids() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let sent = post_chat(&app, &cookie, r#"{"text":"hi"}"#).await;
    let mine = sent["messageId"].as_str().expect("own message id");
    let reply = sent["responses"][0]["messageId"]
        .as_str()
        .expect("reply message id");
    assert_ne!(mine, reply, "the two halves are separate journal lines");

    let history = get_history(&app, &cookie, "").await;
    let ids: Vec<&str> = history.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&mine), "own id absent from history: {ids:?}");
    assert!(
        ids.contains(&reply),
        "reply id absent from history: {ids:?}"
    );
}

/// `detach: true` answers `202` with the ids the accept already established,
/// claims nothing about a turn that has not settled, and — the half that
/// removes the 504 from the operator's path — arrives while the turn is
/// demonstrably still going (issue #983).
#[tokio::test]
async fn a_detached_turn_answers_202_before_the_turn_finishes() {
    let home_dir = home();
    // The same blocking brain the queue tests use: it parks inside the cycle
    // until released, so the turn is provably unfinished when the response
    // below is read.
    let (brain, entered, release) = BlockingChatBrain::new();
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(brain),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"do the long thing","detach":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(body["detached"], true, "{body}");
    assert!(
        body["turnId"].as_str().is_some_and(|s| !s.is_empty()),
        "the turn id is what the console polls: {body}"
    );
    assert!(
        body["messageId"].as_str().is_some_and(|s| !s.is_empty()),
        "the message is journaled at accept, so its id is knowable here: {body}"
    );
    // The whole point: this body is not allowed to look settled. A console
    // that found `responses` here would render an empty answer as the reply.
    assert!(
        body.get("responses").is_none(),
        "a detached response must not look settled: {body}"
    );
    assert!(
        body.get("stillAwaiting").is_none(),
        "a detached response must not look settled: {body}"
    );

    // And the turn really had not finished when that body was written — the
    // brain is still parked, holding the cycle open.
    entered.acquire().await.expect("the turn entered").forget();
    let statuses: Vec<String> = turn_rows(&runtime)
        .await
        .into_iter()
        .map(|(_, status)| status)
        .collect();
    assert!(
        statuses.iter().any(|s| s == "running" || s == "pending"),
        "the response beat the turn, which is the point: {statuses:?}"
    );

    // It settles on its own, with nobody waiting on it.
    release.add_permits(1);
    until("the detached turn never settled", async || {
        turn_rows(&runtime)
            .await
            .iter()
            .any(|(_, status)| status == "succeeded")
    })
    .await;
}

/// The wire-compat guarantee in the other direction: a caller that sends no
/// `detach` gets exactly the response it always got — a `200` carrying the
/// settled turn — plus the additive `turnId`. An older console is untouched.
#[tokio::test]
async fn a_body_without_detach_still_gets_the_synchronous_response() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    // `post_chat` asserts the 200 itself — the legacy status is part of what
    // this test is pinning.
    let body = post_chat(&app, &cookie, r#"{"text":"hi"}"#).await;

    assert!(
        body["responses"].as_array().is_some_and(|r| !r.is_empty()),
        "the settled shape carries the replies: {body}"
    );
    assert!(
        body["messageId"].as_str().is_some(),
        "the legacy durable id is unchanged: {body}"
    );
    assert!(
        body.get("detached").is_none(),
        "the synchronous response must not carry the detach discriminator: {body}"
    );
    assert!(
        body["turnId"].as_str().is_some(),
        "`turnId` is additive on the synchronous response too: {body}"
    );
}

/// The detached turn is not fire-and-forget: the message it journaled at
/// accept, and the answer the spawned task journals afterwards, both land in
/// the durable transcript. This is the backstop the console re-reads, and
/// the reason a dropped frame is not a lost answer.
#[tokio::test]
async fn a_detached_turn_still_journals_its_question_and_its_answer() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"detached hello","detach":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mine = body["messageId"].as_str().unwrap().to_string();

    // The turn owns its own settle, so wait for the answer to appear rather
    // than for a handle this route deliberately does not hold.
    let mut history = Vec::new();
    for _ in 0..100 {
        history = get_history(&app, &cookie, "").await;
        if history
            .iter()
            .any(|m| m["text"].as_str() == Some("You said: detached hello"))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    let ids: Vec<&str> = history.iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(
        ids.contains(&mine.as_str()),
        "the id handed back at 202 must resolve in history: {ids:?}"
    );
    assert!(
        history
            .iter()
            .any(|m| m["text"].as_str() == Some("You said: detached hello")),
        "the detached turn's answer never reached the transcript: {history:?}"
    );
}

/// **Issue #1000 — the floor of the `202` contract.** A detached response
/// is a promise the console can poll, and the poll starts from the body's
/// `turnId`; a `202` carrying no row is a promise a buffered-`/events`
/// tenant cannot collect, which strands the reply until reload. So when the
/// turn's row cannot be minted, the route must not answer `202` at all: it
/// settles the turn synchronously instead, handing the console the answer —
/// a state the console renders natively, being the same shape an older host
/// (one that ignored `detach`) has always returned.
#[tokio::test]
async fn a_detached_request_without_a_turn_row_settles_synchronously() {
    let home_dir = home();
    let state = state_with_failing_runs(home_dir.path()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let body = post_chat(&app, &cookie, r#"{"text":"rowless detach","detach":true}"#).await;

    // The settled shape, not the empty 202: the console is handed the
    // answer, never a turn id it cannot act on.
    assert!(
        body.get("detached").is_none(),
        "a rowless turn must not claim it can be read back: {body}"
    );
    assert!(
        body["responses"].as_array().is_some_and(|r| !r.is_empty()),
        "the synchronous fallback still delivers the reply: {body}"
    );
}

/// A thread reply survives a reload: the parent id posted with the message
/// comes back on both the operator's line and the answer it drew, so a
/// rehydrating console folds the exchange under the same row it was typed
/// under instead of flattening it into the channel.
#[tokio::test]
async fn thread_replies_survive_a_history_reload() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let root = post_chat(&app, &cookie, r#"{"text":"the plan"}"#).await;
    let root_id = root["messageId"].as_str().unwrap().to_string();

    let threaded = post_chat(
        &app,
        &cookie,
        &serde_json::json!({ "text": "a follow-up", "parent": root_id }).to_string(),
    )
    .await;
    assert!(
        threaded["responses"][0]["messageId"]
            .as_str()
            .is_some_and(|id| id != root_id),
        "the answer is its own journal line, not the root's"
    );

    let history = get_history(&app, &cookie, "").await;
    let parented: Vec<(&str, Option<&str>)> = history
        .iter()
        .map(|m| (m["text"].as_str().unwrap(), m["parentId"].as_str()))
        .collect();
    // The root sits in the channel; both halves of the threaded exchange
    // hang off it — the answer under the row the thread opened from, not
    // under the question, so a thread never nests inside a thread.
    assert!(
        parented.contains(&("the plan", None)),
        "root should be unparented: {parented:?}"
    );
    assert!(
        parented.contains(&("a follow-up", Some(root_id.as_str()))),
        "threaded message lost its parent: {parented:?}"
    );
    assert!(
        parented
            .iter()
            .any(|(text, parent)| *text == "You said: a follow-up"
                && *parent == Some(root_id.as_str())),
        "the reply to a threaded message left the thread: {parented:?}"
    );
}

/// A parent that is not a message id is a 400, not a silently-flattened
/// thread: a reply that quietly lands in the channel reads to the operator
/// as a reply that went missing.
#[tokio::test]
async fn chat_rejects_a_parent_that_is_not_a_message_id() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi","parent":"m3"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A reaction records who reacted and survives a reload; clearing it removes
/// the row; and setting the same reaction twice leaves exactly one row —
/// the explicit `on` flag is what makes the write idempotent.
#[tokio::test]
async fn reactions_persist_are_attributed_and_are_idempotent() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let sent = post_chat(&app, &cookie, r#"{"text":"ship it"}"#).await;
    let target = sent["messageId"].as_str().unwrap().to_string();

    assert_eq!(
        post_reaction(&app, &cookie, &target, "👍", true).await,
        StatusCode::NO_CONTENT
    );
    // Twice, deliberately: a retry or a double tap must not double the row.
    assert_eq!(
        post_reaction(&app, &cookie, &target, "👍", true).await,
        StatusCode::NO_CONTENT
    );

    let history = get_history(&app, &cookie, "").await;
    let reacted = history
        .iter()
        .find(|m| m["id"].as_str() == Some(target.as_str()))
        .expect("the reacted-to message is still in history");
    let rows = reacted["reactions"].as_array().expect("reactions present");
    assert_eq!(rows.len(), 1, "one row per person per emoji: {rows:?}");
    assert_eq!(rows[0]["emoji"], "👍");
    assert_eq!(rows[0]["mine"], true, "the reader is the one who reacted");
    assert!(
        rows[0]["by"].as_str().is_some_and(|by| !by.is_empty()),
        "a reaction names who made it: {rows:?}"
    );

    // Clearing drops the row entirely rather than leaving a zero behind.
    assert_eq!(
        post_reaction(&app, &cookie, &target, "👍", false).await,
        StatusCode::NO_CONTENT
    );
    let history = get_history(&app, &cookie, "").await;
    let cleared = history
        .iter()
        .find(|m| m["id"].as_str() == Some(target.as_str()))
        .unwrap();
    assert!(
        cleared.get("reactions").is_none(),
        "a cleared reaction leaves no row: {cleared:?}"
    );
}

/// A reaction may only name a chat message. A sequence position that holds
/// something else — or nothing at all — is a 404, so the log can never carry
/// a reaction no reader could render.
#[tokio::test]
async fn reactions_refuse_a_target_that_is_not_a_message() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let not_a_message = runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::FeedbackFiled {
                note: "unrelated".to_string(),
            },
        )
        .await
        .unwrap();
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    assert_eq!(
        post_reaction(
            &app,
            &cookie,
            &not_a_message.value().to_string(),
            "👍",
            true
        )
        .await,
        StatusCode::NOT_FOUND
    );
    // A sequence position nothing has ever occupied.
    assert_eq!(
        post_reaction(&app, &cookie, "99999", "👍", true).await,
        StatusCode::NOT_FOUND
    );
    // And a target that is not a sequence position at all.
    assert_eq!(
        post_reaction(&app, &cookie, "m3", "👍", true).await,
        StatusCode::BAD_REQUEST
    );
}

/// A reaction is a journal line read by the operator projection, so it takes
/// an emoji and not a payload: empty, oversized, and control-character
/// bodies are all refused.
#[tokio::test]
async fn reactions_refuse_a_body_that_is_not_an_emoji() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let sent = post_chat(&app, &cookie, r#"{"text":"hi"}"#).await;
    let target = sent["messageId"].as_str().unwrap().to_string();

    for bad in ["", "   ", "yes\nno", &"x".repeat(REACTION_MAX_BYTES + 1)] {
        assert_eq!(
            post_reaction(&app, &cookie, &target, bad, true).await,
            StatusCode::BAD_REQUEST,
            "accepted a non-emoji reaction: {bad:?}"
        );
    }
    // A multi-code-point emoji is still one reaction, and is accepted.
    assert_eq!(
        post_reaction(&app, &cookie, &target, "👩‍💻", true).await,
        StatusCode::NO_CONTENT
    );
}

/// PR #1781 review: `history_for_desk` (reload) and `project_event_for_viewer`
/// (live SSE) both already hide an owner-fallback report from a non-admin —
/// this proves the reaction route agrees, rather than letting a Member
/// react to (and thereby confirm the existence and sequence position of) a
/// report they cannot read. Answered with the same 404 an unknown sequence
/// gets, not a 403, so probing this endpoint cannot distinguish "hidden"
/// from "never existed".
#[tokio::test]
async fn reactions_refuse_a_target_that_is_an_admin_only_report() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let report = runtime
        .events()
        .append(
            runtime.id(),
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "operator".into(),
                agent_id: crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR.to_string(),
                text: "no admin has a mailbox".into(),
                steps: Vec::new(),
            },
        )
        .await
        .unwrap();
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let app = router(state);
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let admin_cookie = crate::server::test_support::fixed_cookie("acme");

    // A Member gets the same 404 an unknown message would.
    assert_eq!(
        post_reaction(
            &app,
            &member_cookie,
            &report.value().to_string(),
            "👍",
            true
        )
        .await,
        StatusCode::NOT_FOUND
    );
    // An admin may react to it normally.
    assert_eq!(
        post_reaction(&app, &admin_cookie, &report.value().to_string(), "👍", true).await,
        StatusCode::NO_CONTENT
    );
}

/// Regression for the third acceptance item of #364, which the console's
/// own scoping already satisfied but nothing pinned: a message posted in one
/// channel must be absent from another, end to end through the route — not
/// only in the `owns` predicate. Reactions ride the same boundary.
#[tokio::test]
async fn a_message_in_one_channel_is_absent_from_another() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let app = router(state);
    let cookie = crate::server::test_support::fixed_cookie("acme");

    let in_studio = post_chat(&app, &cookie, r#"{"text":"studio only","chat":"studio"}"#).await;
    let studio_id = in_studio["messageId"].as_str().unwrap().to_string();
    post_chat(&app, &cookie, r#"{"text":"general only"}"#).await;
    assert_eq!(
        post_reaction(&app, &cookie, &studio_id, "👀", true).await,
        StatusCode::NO_CONTENT
    );

    let studio: Vec<String> = get_history(&app, &cookie, "studio")
        .await
        .iter()
        .map(|m| m["text"].as_str().unwrap().to_string())
        .collect();
    let general = get_history(&app, &cookie, "").await;
    let general_texts: Vec<&str> = general
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();

    assert!(
        studio.iter().any(|t| t == "studio only"),
        "the desk lost its own message: {studio:?}"
    );
    assert!(
        !studio.iter().any(|t| t == "general only"),
        "a General message leaked into the desk: {studio:?}"
    );
    assert!(
        general_texts.contains(&"general only"),
        "General lost its own message: {general_texts:?}"
    );
    assert!(
        !general_texts.contains(&"studio only"),
        "a desk message leaked into General: {general_texts:?}"
    );
    // The reaction is on the desk's message, so it is not visible from a
    // channel that cannot see the message it is about.
    assert!(
        general.iter().all(|m| m.get("reactions").is_none()),
        "a reaction crossed a channel boundary: {general:?}"
    );
}

/// **Issue #2028 (finding 2, deadlock regression).** Answering a
/// task-backed blocker in a DM runs the whole path end to end: the route
/// reads and classifies the reply, settles the verdict, and waits on the
/// follow-up that re-dispatches the card — and that follow-up runs on a
/// spawned task which takes `task_writes` for its board edit.
///
/// So the route must not still hold `task_writes` when it waits. It did,
/// having mirrored the guard from the review branch above it, and the two
/// together are a deadlock: the handler waits for a task that is waiting for
/// the handler's lock. Explicitly bounded rather than left to hang, so a
/// regression fails in seconds instead of taking a runner down for an hour.
#[cfg(feature = "openhuman")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_dm_answer_to_a_task_backed_blocker_completes() {
    use crate::company::blocker_sender::BlockerSenderSignals;
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = build_state_with_brain_and_manifest(
        &home,
        "running",
        AppConfig::default(),
        None,
        roster_manifest(),
    )
    .await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    let mut card = crate::ports::tasks::TaskRecord {
        id: "t-9".to_string(),
        title: crate::ports::tasks::TaskTitle::authored("Draft the launch note"),
        note: None,
        column: crate::ports::tasks::COLUMN_PAUSED.to_string(),
        priority: "medium".to_string(),
        assignee: "backend_engineer".to_string(),
        updated_at_millis: 1,
        origin: None,
        origin_message_seq: None,
        parent_task_id: None,
        output: None,
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: crate::ports::tasks::TaskDeliverable::Once,
        workflow_proposal: None,
        origin_run_id: None,
        origin_workflow_id: None,
        bounced: None,
    };
    card.origin =
        crate::ports::tasks::TaskOrigin::new(Some("dm:backend_engineer".to_string()), None);
    runtime.tasks().upsert(runtime.id(), &card).await.unwrap();

    runtime
        .park_blocker(
            &BlockerPayload {
                kind: BlockerKind::Infrastructure,
                source: BlockerSource::Provider,
                step: Some(BlockerStep::Task {
                    task_id: "t-9".to_string(),
                }),
                reason: "the model id was rejected".to_string(),
                needed: "a model id this provider serves".to_string(),
                group_key: None,
            },
            "t-9",
            BlockerSenderSignals {
                started_by: None,
                owner_desk: None,
                assignee: Some("backend_engineer".to_string()),
            },
        )
        .await
        .expect("parks the blocker into the teammate's DM");

    let response = tokio::time::timeout(
        Duration::from_secs(30),
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/acme/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"chat":"dm:backend_engineer","text":"yes, go ahead and retry it"}"#,
                ))
                .unwrap(),
        ),
    )
    .await
    .expect(
        "answering a task-backed blocker in a DM deadlocked: the route held the board \
         lock while waiting on the follow-up that needs it",
    )
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        runtime.pending_approvals().is_empty(),
        "the answered blocker is retired"
    );
    let moved = runtime
        .tasks()
        .list(runtime.id())
        .await
        .expect("list")
        .into_iter()
        .find(|t| t.id == "t-9")
        .expect("the card is still on the board");
    assert_eq!(
        moved.column,
        crate::ports::tasks::COLUMN_IN_PROGRESS,
        "the DM answer re-dispatched the paused card"
    );
}

/// The ask-which question lands in the thread that asked it.
///
/// When two blocked things share a DM and the reply names neither, the
/// runtime asks which one was meant. That question is an answer to the
/// operator's message, so it threads off it the way every other reply in
/// this handler does — otherwise the operator reads their own line in a
/// thread and the teammate's follow-up at the channel root, which is the
/// split this tier exists to close.
#[cfg(feature = "openhuman")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_ask_which_question_threads_off_the_reply_that_was_ambiguous() {
    use crate::company::blocker_sender::BlockerSenderSignals;
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = build_state_with_brain_and_manifest(
        &home,
        "running",
        AppConfig::default(),
        None,
        roster_manifest(),
    )
    .await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    for (task, connection) in [("t-1", "connection:slack"), ("t-2", "connection:notion")] {
        runtime
            .park_blocker(
                &BlockerPayload {
                    kind: BlockerKind::Infrastructure,
                    source: BlockerSource::Provider,
                    step: Some(BlockerStep::Task {
                        task_id: task.to_string(),
                    }),
                    reason: format!("{connection} refused the call"),
                    needed: "a working connection".to_string(),
                    group_key: Some(connection.to_string()),
                },
                task,
                BlockerSenderSignals {
                    started_by: None,
                    owner_desk: None,
                    assignee: Some("backend_engineer".to_string()),
                },
            )
            .await
            .expect("parks the blocker into the teammate's DM");
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/acme/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"chat":"dm:backend_engineer","text":"retry it"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = runtime
        .events
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("read events");
    let asked = stored
        .iter()
        .find_map(|s| match &s.event {
            crate::ports::types::CompanyEvent::OperatorMessage { chat, text, .. }
                if chat.as_deref() == Some("dm:backend_engineer") && text == "retry it" =>
            {
                Some(s.seq)
            }
            _ => None,
        })
        .expect("the operator's ambiguous reply is journalled");
    let prompt = stored
        .iter()
        .find_map(|s| match &s.event {
            crate::ports::types::CompanyEvent::AgentReply {
                chat_id,
                text,
                parent,
                ..
            } if chat_id == "dm:backend_engineer" && text.contains("Which") => {
                Some((text.clone(), *parent))
            }
            _ => None,
        })
        .expect("the runtime asks which of the two was meant");
    assert_eq!(
        prompt.1,
        Some(asked),
        "the ask-which question must hang off the reply that was ambiguous, not the \
         channel root; prompt was {:?}",
        prompt.0
    );
}

#[tokio::test]
async fn chat_by_id_matches_registered_company() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/acme/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"yo"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn unknown_company_is_404() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/companies/ghost/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    // 401, not 404: the caller holds no credential for `ghost`, and
    // authentication precedes existence. Answering "no such company" to an
    // unauthenticated caller would let anyone enumerate which companies a
    // host runs. A user of `ghost` gets a real 404.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn paused_company_chat_is_409() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "paused").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn list_and_status_routes_report_the_company() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/companies")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let bytes = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 1);
    assert_eq!(value[0]["id"], "acme");

    let status = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/companies/acme")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let bytes = to_bytes(status.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["id"], "acme");
}

#[tokio::test]
async fn approvals_list_is_empty_before_any_park() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/approvals")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn amended_approve_resolves_and_returns_responses() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    // An `approve` verdict carrying an amended payload routes to the
    // approve-with-edit path. Even against an unknown id it resolves
    // cleanly (nothing to execute) and the follow-up cycle replies.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"verdict":"approve","amended_payload":{"text":"edited"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["responses"].is_array());
}

/// The tool call the operator is asked to sign off. `agent: Some(_)` is what
/// makes approving it mint a single-use grant rather than execute it
/// (issue #243) — which is the whole reason a lost continuation hurts: the
/// grant is spent on a turn that never happens.
///
/// The payload names an action the vendored catalogue tags `Write`, so
/// `consequence_of` classifies it as a send on its merits. Until issue #470
/// it named the slug under `tool_slug`, a key neither the tool nor the
/// classifier reads — so it was a call with no action at all, and it
/// reached the per-call verdict through the unknown-slug fallback instead.
fn gated_tool_call() -> crate::ports::types::Effect {
    crate::ports::types::Effect {
        kind: "composio_execute".into(),
        group: crate::ports::types::EffectGroup::Sign,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: crate::policy::test_support::composio_send_args(),
        agent: Some("ceo".into()),
        run_id: None,
    }
}

/// A tool call an operator MAY grant a standing permission for (issue
/// #431), which `gated_tool_call` deliberately is not: its Composio payload
/// names an action the catalogue tags `Write`, so `consequence_of` reads it
/// as a send and it stays a per-call decision. `file_write` is declared
/// grantable in `src/policy/consequence.rs` and carries an agent, so it
/// satisfies both halves of `check_broadly_grantable` — it mutates, but
/// only the agent's own sandboxed workspace.
fn grantable_tool_call() -> crate::ports::types::Effect {
    crate::ports::types::Effect {
        kind: "file_write".into(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "path": "notes/a.md", "body": "one" }),
        agent: Some("ceo".into()),
        run_id: None,
    }
}

/// Issue #618: membership gets you the approval, role gets you its
/// contents.
///
/// Issue #561: the receipt says whether this decision actually released the
/// turn.
///
/// A turn that parked two calls is blocked on two decisions (issue #469
/// continues it once, on the last one). The console used to tell the
/// operator "the agent is completing the action" on the first click, which
/// is false — nothing runs until the second. This is the count it now words
/// that sentence from: one still owed after the first decision, none after
/// the second.
#[tokio::test]
async fn a_receipt_says_how_many_decisions_the_turn_is_still_blocked_on() {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    let effect = |memo: &str| crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(10.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "board@example.test", "memo": memo }),
        agent: Some("ceo".into()),
        run_id: None,
    };

    // One turn, two parked calls — the shape an operator meets whenever an
    // agent gates more than once in a turn.
    for (id, memo) in [("appr-561-a", "first"), ("appr-561-b", "second")] {
        runtime
            .journal
            .record_parked(
                &crate::ports::types::ApprovalId::new(id),
                &effect(memo),
                1_000,
                TaskLink::Unlinked,
                ApprovalConversation::default(),
                Some("cycle-561".to_string()),
            )
            .await
            .unwrap();
        runtime.continuations.arm("cycle-561");
    }

    let app = router(state);
    let resolve = |app: axum::Router, id: &'static str| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/company/approvals/{id}"))
                    .header("content-type", "application/json")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::from(
                        serde_json::json!({ "verdict": "approve", "detach": true }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let first = resolve(app.clone(), "appr-561-a").await;
    assert_eq!(
        first["stillAwaiting"], 1,
        "the first decision releases nothing — the turn is still blocked on the second: {first}"
    );

    // The count is read per decision, at the moment the verdict lands. What
    // happens to the sibling afterwards is issue #848's business — a turn's
    // gated calls may be consolidated and settle together — and this test
    // deliberately asserts only the half the operator's confirmation is
    // worded from: this click did not release the turn.
    //
    // The other half — the last decision reporting nothing outstanding —
    // is pinned on the queue itself in
    // `runtime::continuation::test::outstanding_counts_the_decision_being_made`,
    // where it is deterministic rather than racing a spawned follow-up.
}

/// **The two-account part is the point.** The harness signs every request
/// in as an admin, so a redaction verified only as an admin passes
/// identically against no redaction at all — the test would prove nothing
/// while looking like coverage. This seeds a second, Member-role account
/// and drives the same route with both.
#[tokio::test]
async fn a_member_sees_the_approval_but_not_its_payload_or_amount() {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    let effect = crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(2400.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "board@example.test", "memo": "Q3 retainer" }),
        agent: Some("ceo".into()),
        run_id: None,
    };
    runtime
        .journal
        .record_parked(
            &crate::ports::types::ApprovalId::new("appr-618"),
            &effect,
            1_000,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();

    let app = router(state);

    async fn approvals_as(app: &axum::Router, cookie: String) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/company/approvals")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // The admin decides the sign-off, so the admin sees what it will do.
    let as_admin = approvals_as(&app, crate::server::test_support::fixed_cookie("acme")).await;
    let admin_row = &as_admin.as_array().unwrap()[0];
    assert_eq!(admin_row["amount_usd"].as_f64(), Some(2400.0));
    assert_eq!(admin_row["payload"]["to"], "board@example.test");
    assert!(
        admin_row.get("contents_hidden").is_none(),
        "an admin is not told anything was hidden: {admin_row}"
    );

    let as_member =
        approvals_as(&app, crate::server::test_support::member_cookie("acme")).await;
    let member_row = &as_member.as_array().unwrap()[0];

    // Still visible: everything that makes stalled work legible. This half
    // is what #468 depends on — a member must keep seeing that work is
    // waiting and what kind of call it is.
    assert_eq!(member_row["id"], "appr-618");
    assert_eq!(member_row["kind"], "payment.send");
    assert_eq!(member_row["agent"], "ceo");
    assert_eq!(member_row["at_millis"].as_u64(), Some(1_000));

    // Withheld: the recipient and the money.
    assert!(
        member_row.get("payload").is_none(),
        "the recipient must not reach a member: {member_row}"
    );
    // `null`, not absent: unlike `payload`, `amount_usd` carries no
    // `skip_serializing_if`, so it stays on the wire as an explicit null.
    // Both read as "no value" to the console (`a.amount_usd != null`
    // covers either), and changing the wire shape as a side effect of a
    // redaction would be a worse trade than asserting the shape that is
    // actually there.
    assert!(
        member_row["amount_usd"].is_null(),
        "nor the amount: {member_row}"
    );
    assert_eq!(
        member_row["contents_hidden"], true,
        "and the console must be able to say so rather than render an empty card: {member_row}"
    );

    // Belt and braces: the recipient string must appear nowhere in the
    // member's response, however the shape changes later.
    let raw = serde_json::to_string(&as_member).unwrap();
    assert!(
        !raw.contains("board@example.test") && !raw.contains("Q3 retainer"),
        "payload content leaked to a member: {raw}"
    );
}

/// The dotted kind the stalled brain parks once its follow-up turn gets
/// past the barrier. Parking journals durably (`record_parked`), so its
/// presence in `pending_approvals()` is proof the continuation reached the
/// end of the turn *and* wrote to disk — not merely that a task was alive.
const CONTINUATION_MARKER: &str = "continuation.marker";

/// A brain that parks one gated tool call per operator message and, on the
/// follow-up `ApprovalResolved` cycle, blocks mid-turn until the test
/// releases it — the shape of a slow agent turn behind a proxy.
struct StalledContinuationBrain {
    /// Fires once the follow-up turn has begun. By this point the verdict
    /// is journaled and the grant minted, so this is exactly the moment the
    /// field report's connection died.
    entered: Arc<tokio::sync::Notify>,
    /// The test's permission for the turn to finish.
    release: Arc<tokio::sync::Notify>,
    /// The effect parked for the operator's sign-off. Whether it may be
    /// granted a standing permission is a property of this effect, so the
    /// scope tests supply their own rather than sharing one fixture.
    parked: crate::ports::types::Effect,
}

#[async_trait::async_trait]
impl crate::ports::brain::Brain for StalledContinuationBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        for event in &req.events {
            match event {
                CompanyEvent::OperatorMessage { .. } => {
                    host.park_effect(self.parked.clone()).await?;
                }
                CompanyEvent::ApprovalResolved { .. } => {
                    self.entered.notify_one();
                    self.release.notified().await;
                    host.park_effect(crate::ports::types::Effect {
                        kind: CONTINUATION_MARKER.into(),
                        group: crate::ports::types::EffectGroup::Other,
                        amount_usd: None,
                        established_thread: false,
                        first_time_counterparty: false,
                        payload: serde_json::json!({}),
                        agent: None,
                        run_id: None,
                    })
                    .await?;
                }
                _ => {}
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses: Vec::new(),
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "stalled continuation",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

fn chat_request(text: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/company/chat")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({ "text": text }).to_string()))
        .unwrap()
}

/// A resolve against the single-company alias. `scope` lets the same body be
/// aimed at the `/companies/{id}` form, which must behave identically.
fn resolve_request_scoped(
    scope: &str,
    approval_id: &ApprovalId,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}"))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn resolve_request(approval_id: &ApprovalId, body: serde_json::Value) -> Request<Body> {
    resolve_request_scoped("/api/v1/company", approval_id, body)
}

// -- A blocker answered from the Approvals page (issue #2028) -------------

/// Parks a workflow-node blocker: `TaskLink::Unlinked` with no
/// conversation, which is the shape a node blocker takes and the reason the
/// chat blocker path — which filters on the thread — can never reach one.
#[cfg(feature = "openhuman")]
async fn park_node_blocker(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
    group_key: Option<&str>,
) -> ApprovalId {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource, BlockerStep};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let payload = BlockerPayload {
        kind: BlockerKind::Infrastructure,
        source: BlockerSource::Provider,
        step: Some(BlockerStep::Node {
            run_id: "run-1".to_string(),
            node_id: "draft".to_string(),
        }),
        reason: "the model id `gpt-nope` was rejected".to_string(),
        needed: "a model id this provider serves".to_string(),
        group_key: group_key.map(str::to_string),
    };
    let approval = ApprovalId::new(id);
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: Some("run-1".to_string()),
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();
    approval
}

/// Every `BlockerResolved` line the durable journal holds, in append order —
/// what the operator's answer actually banked, read off disk rather than off
/// the in-memory map the resume consumes.
#[cfg(feature = "openhuman")]
async fn banked_resolutions(
    home: &std::path::Path,
    company: &CompanyId,
) -> Vec<serde_json::Value> {
    let path = crate::store::paths::Bundle::new(home, company).journal_jsonl();
    let raw = tokio::fs::read_to_string(path).await.unwrap_or_default();
    raw.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|line| line["record"] == "BlockerResolved")
        .collect()
}

/// A company with one parked workflow-node blocker, and the pieces a resolve
/// test needs to read back what its click banked.
#[cfg(feature = "openhuman")]
struct BlockedCompany {
    app: axum::Router,
    runtime: Arc<CompanyRuntime>,
    home: std::path::PathBuf,
    company: CompanyId,
    approval_id: ApprovalId,
}

#[cfg(feature = "openhuman")]
async fn blocked_company(home: &std::path::Path) -> BlockedCompany {
    let home = home.to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let approval_id = park_node_blocker(&runtime, "blocker-1", None).await;
    BlockedCompany {
        app,
        runtime,
        home,
        company,
        approval_id,
    }
}

/// Posts a resolve and returns its status and parsed body.
#[cfg(feature = "openhuman")]
async fn post_resolve(
    app: &axum::Router,
    id: &ApprovalId,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(resolve_request(id, body))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Every refusal owes the same two things beyond its 400: the blocker is
/// still parked, and nothing was banked. A validation that answered 400
/// after journaling a verdict would have spent the operator's question.
#[cfg(feature = "openhuman")]
async fn assert_refused(body: serde_json::Value, expect_in_error: &str) {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(&c.app, &c.approval_id, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    let message = answer["error"].as_str().unwrap_or_default();
    assert!(
        message.contains(expect_in_error),
        "the refusal must say why; got {message:?}"
    );
    assert!(
        c.runtime
            .pending_approvals()
            .iter()
            .any(|p| p.id == c.approval_id),
        "a refused request must leave the blocker parked"
    );
    assert!(
        banked_resolutions(&c.home, &c.company).await.is_empty(),
        "a refused request must journal no verdict"
    );
}

/// **Issue #2028 — the bug.** An Approvals click that says `skip` banks a
/// skip. Before the route arm existed the same request banked a `retry`,
/// because `verdict: approve` was the only thing the host read.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_skip_from_the_approvals_page_banks_a_skip() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1, "one answer, one banked resolution");
    assert_eq!(
        banked[0]["resolution"]["verdict"], "skip",
        "the operator asked to skip the node, not to run it again"
    );
}

/// The amend twin: the words the operator typed reach the banked resolution
/// verbatim, which is what the re-entered step reads.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_amend_from_the_approvals_page_carries_the_answer_verbatim() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "amend",
            "blocker_answer": "use gpt-4o-mini instead",
            "detach": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(banked[0]["resolution"]["verdict"], "amend");
    assert_eq!(
        banked[0]["resolution"]["answer"], "use gpt-4o-mini instead",
        "the correction must reach the step, or the re-run repeats the failure"
    );
}

/// A cancel still denies, and is still the only verdict that does.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_cancel_from_the_approvals_page_banks_a_cancel() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "deny", "blocker_verdict": "cancel", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(banked[0]["resolution"]["verdict"], "cancel");
}

/// Answering one member of a root-cause group answers all of them — the
/// same fan-out a DM answer performs — and the receipt names every id it
/// settled so the console can drop the siblings' cards too.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_group_settles_together_and_the_receipt_names_every_member() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let first = park_node_blocker(&runtime, "grouped-1", Some("connection:slack")).await;
    let second = park_node_blocker(&runtime, "grouped-2", Some("connection:slack")).await;

    let (status, answer) = post_resolve(
        &app,
        &first,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["settledIds"],
        serde_json::json!(["grouped-1", "grouped-2"]),
        "the receipt must name the siblings the answer settled: {answer}"
    );
    assert!(
        runtime.pending_approvals().is_empty(),
        "one answer to a root-cause group retires every member of it"
    );
    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 2, "both members banked the same verdict");
    for line in &banked {
        assert_eq!(line["resolution"]["verdict"], "skip");
    }
    let _ = second;
}

/// An ordinary resolve is unchanged: no `settledIds` key at all, so a
/// console predating the field reads the same body it always did.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_ordinary_resolve_names_no_settled_ids() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "approve", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        answer.get("settledIds").is_none(),
        "a resolve that fanned to nothing must carry no list: {answer}"
    );
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_disagreeing_verdict_pair_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "deny", "blocker_verdict": "skip" }),
        "cannot accompany verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blank_amend_is_refused_rather_than_downgraded() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "amend",
            "blocker_answer": "   \n\t ",
        }),
        "needs a non-empty blocker_answer",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_amend_with_no_answer_at_all_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "amend" }),
        "needs a non-empty blocker_answer",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_answer_with_no_verdict_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_answer": "use gpt-4o-mini" }),
        "blocker_answer needs a blocker_verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_answer_on_a_wordless_verdict_is_refused() {
    for verdict in ["retry", "skip", "cancel"] {
        let event = if verdict == "cancel" {
            "deny"
        } else {
            "approve"
        };
        assert_refused(
            serde_json::json!({
                "verdict": event,
                "blocker_verdict": verdict,
                "blocker_answer": "words this verdict cannot carry",
            }),
            "only accompanies blocker_verdict",
        )
        .await;
    }
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_unknown_blocker_verdict_is_refused_by_name() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "ignore" }),
        "unknown blocker_verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_with_an_amended_payload_is_refused() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "skip",
            "amended_payload": { "text": "edited" },
        }),
        "cannot accompany amended_payload",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_with_a_tool_scope_is_refused() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "skip",
            "scope": "tool",
            "expires_in_millis": 3_600_000,
        }),
        "cannot accompany scope",
    )
    .await;
}

/// A `blocker_verdict` on an approval that is not a parked blocker is a 400,
/// not a quiet fall-through to the two-value path — which would lose the
/// operator's verdict without telling anyone.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_on_an_ordinary_approval_is_refused() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let ordinary = park_for_extend(&runtime, "ordinary-1", crate::ports::now_millis()).await;

    let (status, answer) = post_resolve(
        &app,
        &ordinary,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or_default()
            .contains("is not a parked blocker"),
        "{answer}"
    );
    assert!(
        runtime.pending_approvals().iter().any(|p| p.id == ordinary),
        "a refused request must leave the approval parked"
    );
    assert!(banked_resolutions(&home, &company).await.is_empty());
}

/// A stepless blocker uses its task link to settle the card it paused.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn skipping_an_agent_question_settles_the_card_its_approval_is_linked_to() {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-9".to_string(),
                title: crate::ports::tasks::TaskTitle::authored("Draft the launch note"),
                note: None,
                column: crate::ports::tasks::COLUMN_PAUSED.to_string(),
                priority: "medium".to_string(),
                assignee: "eng".to_string(),
                updated_at_millis: 1,
                origin: None,
                origin_message_seq: None,
                parent_task_id: None,
                output: Some(crate::ports::tasks::TaskOutput {
                    source: crate::ports::tasks::TaskOutputSource::Run {
                        run_id: "old-run".to_string(),
                        attempt: Some(1),
                    },
                    at_millis: 1,
                    artifacts: Vec::new(),
                    workflows: Vec::new(),
                }),
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                bounced: Some("stale failure".to_string()),
            },
        )
        .await
        .unwrap();

    let payload = BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::AgentQuestion,
        step: None,
        reason: "which of the two briefs is current?".to_string(),
        needed: "an answer from you".to_string(),
        group_key: None,
    };
    let approval = ApprovalId::new("question-1");
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: None,
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::from_task_id(Some("t-9")),
            ApprovalConversation {
                thread: Some("dm:eng".to_string()),
                parent: None,
            },
            None,
        )
        .await
        .unwrap();

    let (status, answer) = post_resolve(
        &app,
        &approval,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["settledIds"],
        serde_json::json!(["question-1"]),
        "the non-detached body names what it settled too: {answer}"
    );

    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(
        banked[0]["resolution"]["verdict"], "skip",
        "the operator's verdict is banked whatever the resume can do with it"
    );
    assert!(
        banked[0]["resolution"].get("step").is_none(),
        "the durable record keeps the stepless park the blocker carried: {}",
        banked[0]
    );
    let card = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-9")
        .expect("the card still exists");
    assert_eq!(
        card.column,
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "the skipped card is ready for human review"
    );
    assert!(card.output.is_none(), "a skip produces no output");
    assert!(card.bounced.is_none(), "a skip clears the old bounce chip");
    assert_eq!(card.origin_chat_id(), Some("dm:eng"));
    assert!(
        card.note
            .as_deref()
            .is_some_and(|note| { note.contains("blocker question waived by the operator") })
    );
    assert!(
        runtime
            .runs()
            .list_runs(
                runtime.id(),
                &crate::ports::runs::RunFilter::for_task("t-9"),
            )
            .await
            .unwrap()
            .is_empty(),
        "a skip must not open another attempt"
    );
}

/// The link is followed only to a card the board still holds.
///
/// A stepless question's approval carries a task link because a card was in
/// hand when it was asked, not because the card is the thing to re-enter.
/// When that card is gone — deleted, or never on this board — reading the
/// link as a card resume answers the operator with *that card is no longer
/// on the board*, which is a report about a card in place of the answer to
/// the question they just gave. The answer goes back into the conversation
/// instead, exactly as it does for a question that was never linked.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_agent_question_linked_to_a_card_the_board_lost_still_answers_the_question() {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    let payload = BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::AgentQuestion,
        step: None,
        reason: "which of the two briefs is current?".to_string(),
        needed: "an answer from you".to_string(),
        group_key: None,
    };
    let approval = ApprovalId::new("question-2");
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: None,
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    // The link names a card that is not on the board, which is the whole
    // case: nothing is seeded for `t-gone`.
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::from_task_id(Some("t-gone")),
            ApprovalConversation {
                thread: Some("dm:eng".to_string()),
                parent: None,
            },
            None,
        )
        .await
        .unwrap();

    let (status, answer) = post_resolve(
        &app,
        &approval,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "retry" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(
        banked[0]["resolution"]["verdict"], "retry",
        "the operator's answer is banked whatever the resume finds: {}",
        banked[0]
    );
    let notes: Vec<String> = runtime
        .events
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .expect("read events")
        .into_iter()
        .filter_map(|stored| match stored.event {
            crate::ports::types::CompanyEvent::AgentReply { chat_id, text, .. }
                if chat_id == "dm:eng" =>
            {
                Some(text)
            }
            _ => None,
        })
        .collect();
    assert!(
        !notes
            .iter()
            .any(|note| note.contains("no longer on the board")),
        "answering a question must not report on a card the asker never mentioned; \
         posted: {notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note == "Got it — picking that back up now."),
        "the answer must still reach the conversation it was asked in; posted: {notes:?}"
    );
}

/// A build with no blocker resume refuses the field outright. Accepting and
/// ignoring it would answer `200` to a skip that silently became a retry —
/// the exact defect, reintroduced by a feature flag.
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn a_build_without_the_resume_refuses_a_blocker_verdict() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(resolve_request(
            &ApprovalId::new("missing"),
            serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not supported by this build"),
        "{value}"
    );
}

// -- Extend the deadline (issue #1805) -----------------------------------

/// Parks one effect in BOTH the gate and the journal under a fixed id, at a
/// controllable instant — the gate is what `extend_approval` asks whether an
/// id is live, and the journal is what projects the deadline, so an extend
/// test needs both seeded exactly as a real park leaves them.
async fn park_for_extend(
    runtime: &Arc<CompanyRuntime>,
    id: &str,
    at_millis: u64,
) -> ApprovalId {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};
    let approval = ApprovalId::new(id);
    let effect = crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(1_200.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "vendor@example.test" }),
        agent: Some("ceo".into()),
        run_id: None,
    };
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at_millis);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at_millis,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();
    approval
}

fn extend_request_with_cookie(approval_id: &ApprovalId, cookie: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/company/approvals/{approval_id}/extend"))
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap()
}

fn extend_request(approval_id: &ApprovalId) -> Request<Body> {
    extend_request_with_cookie(
        approval_id,
        crate::server::test_support::fixed_cookie("acme"),
    )
}

async fn body_json(response: Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The keystone (issue #1805): extending a parked approval pushes its
/// deadline out to a fresh full window, and the receipt names the new one —
/// the console can redraw the countdown without re-fetching the list.
#[tokio::test]
async fn extending_a_parked_approval_moves_its_deadline() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    // Parked long ago, so its original deadline is `1_000 + ttl`.
    let id = park_for_extend(&runtime, "appr-ext", 1_000).await;
    let before = runtime.pending_approvals()[0]
        .expires_at_millis
        .expect("a deadline is projected");

    let app = router(state);
    let response = app.oneshot(extend_request(&id)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;

    let after = runtime.pending_approvals()[0]
        .expires_at_millis
        .expect("a deadline is still projected");
    assert!(
        after > before,
        "the deadline moved out: before={before} after={after}"
    );
    assert!(body["extended"].as_bool().unwrap());
    assert_eq!(
        body["expiresAtMillis"].as_f64().unwrap() as u64,
        after,
        "the receipt's deadline is the one the card now projects"
    );
}

/// Extending something that is not parked — an unknown id, or one already
/// resolved or expired — is a 404, not a 200 over nothing.
#[tokio::test]
async fn extending_an_unknown_approval_is_404() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);
    let response = app
        .oneshot(extend_request(&ApprovalId::new("does-not-exist")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// POL-011 (INPUT). `{aid}` is an opaque path segment carried straight
/// into `ApprovalId::new` with no format check of its own — the id space
/// is "whatever a park was given", so the whole of input-safety here is
/// that an adversarial or malformed segment resolves to the same ordinary
/// 404 an unknown id does, never a panic or a 500.
#[tokio::test]
async fn extending_a_malformed_approval_id_is_404_not_a_crash() {
    fn percent_encode_path_segment(raw: &str) -> String {
        let mut out = String::new();
        for byte in raw.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(byte as char);
                }
                _ => out.push_str(&format!("%{byte:02X}")),
            }
        }
        out
    }

    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);

    let hostile_ids = [
        "../../../etc/passwd".to_string(),
        "🎉💥-not-an-approval".to_string(),
        "a".repeat(10_000),
        "'; DROP TABLE approvals; --".to_string(),
        "appr\u{0}-null-byte".to_string(),
    ];
    for raw in hostile_ids {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/company/approvals/{}/extend",
                        percent_encode_path_segment(&raw)
                    ))
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "a malformed id ({raw:?}) must answer the same 404 an unknown id does, not crash"
        );
    }
}

/// APPR-004: extend must be able to win a race the sweep has not yet run —
/// an approval whose deadline has already passed but that is still
/// physically parked (nothing has swept it out of the gate) must still be
/// extendable, and the extension must genuinely move the deadline rather
/// than just answer as if it had.
///
/// `resolve`'s own past-deadline check (`gate.rs`'s TTL math) and
/// `extend`'s (`ParkedApprovals::extend`, existence-only) are two
/// different tests over the same map — that gap is exactly the window
/// `/extend` exists to rescue something in, per issue #1805.
#[tokio::test]
async fn extending_beats_a_pending_sweep_on_an_already_past_deadline_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // Parked at the epoch: this host's TTL has long since passed, and
    // nothing has swept either entry out of the gate yet.
    let control = park_for_extend(&runtime, "appr-control", 1).await;
    let target = park_for_extend(&runtime, "appr-target", 1).await;

    let app = router(state.clone());

    // The control proves the premise: resolving an untouched twin of the
    // same stale park reports `expired`.
    let resolved = app
        .clone()
        .oneshot(resolve_request(
            &control,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let body = body_json(resolved).await;
    assert_eq!(
        body["outcome"], "expired",
        "premise: a park this old is already past this host's TTL, got {body}"
    );

    // Extending the other twin, before anything else touches it, must
    // still succeed — this is the whole reason `/extend` exists.
    let extended = app.clone().oneshot(extend_request(&target)).await.unwrap();
    assert_eq!(
        extended.status(),
        StatusCode::OK,
        "extend must be able to rescue a park the sweep has not yet reclaimed"
    );

    // And now resolving it must NOT report `expired` — the deadline
    // genuinely moved, not just the extend receipt's word for it.
    let resolved = app
        .oneshot(resolve_request(
            &target,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let body = body_json(resolved).await;
    assert_ne!(
        body["outcome"], "expired",
        "extend must genuinely push the deadline out, not just answer as if it did: {body}"
    );
}

/// PLAT-014 (Member ⇒ approve): the sharpest of the auth-matrix's four
/// rows. A Member sees a money-bearing approval exists (issue #468's
/// "waiting on approval" indicator has to survive for them) but not what
/// it is about (issue #618) — and cannot act on it at all: both
/// `POST {scope}/approvals/{aid}` and `/extend` are `AdminScopedCompany`.
/// All three properties are asserted against the same parked approval, so
/// the redaction and the auth gate cannot silently disagree about which
/// one is doing the protecting.
#[tokio::test]
async fn a_member_cannot_read_or_act_on_a_money_bearing_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-member", crate::ports::now_millis()).await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    // Sees it exists, but not what it costs.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/approvals")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let listed = body
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == approval.to_string())
        .expect("the approval is visible to a member");
    assert_eq!(
        listed["contents_hidden"], true,
        "a member must be told the contents were withheld: {listed}"
    );
    assert!(
        listed["amount_usd"].is_null(),
        "a member must not receive the dollar amount: {listed}"
    );
    assert!(
        listed["payload"].is_null(),
        "a member must not receive the payload either: {listed}"
    );

    // Cannot resolve it.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/company/approvals/{approval}"))
                .header("cookie", &member_cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "verdict": "approve" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a member must not be able to approve a parked effect"
    );

    // Cannot extend it either.
    let response = app
        .oneshot(extend_request_with_cookie(&approval, member_cookie))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a member must not be able to extend a parked effect's deadline"
    );
}

/// POL-011: `extend_approval` is one handler mounted under both scope
/// forms (`scoped("/approvals/{aid}/extend", ...)`), so the platform
/// `/companies/{id}/...` form must carry the exact same admin gate the
/// `/company/...` alias does — and must not become a side channel that
/// resolves against the wrong company merely because its id rode in the
/// path instead of the alias.
#[tokio::test]
async fn extend_on_the_scoped_route_form_enforces_admin_and_the_right_company() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-scoped", crate::ports::now_millis()).await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let admin_cookie = crate::server::test_support::fixed_cookie("acme");
    let app = router(state);

    // AUTH: a member is refused on the scoped form exactly as on the alias.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/acme/approvals/{approval}/extend"
                ))
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // FAIL: addressing a *different* company id on the scoped form must
    // 404 rather than reach into `acme`'s gate — the path segment is the
    // only thing naming the company here, unlike the alias.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/globex/approvals/{approval}/extend"
                ))
                .header("cookie", &admin_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a company id that does not exist must not extend acme's approval"
    );
    assert!(
        runtime.pending_approvals().iter().any(|a| a.id == approval),
        "the approval must still be sitting under its real company, untouched"
    );

    // And the scoped form works for the right admin and the right company.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/companies/acme/approvals/{approval}/extend"
                ))
                .header("cookie", &admin_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// Whether the stalled brain's follow-up turn has journaled its marker yet.
fn continued(runtime: &Arc<CompanyRuntime>) -> bool {
    runtime
        .pending_approvals()
        .iter()
        .any(|a| a.kind == CONTINUATION_MARKER)
}

/// Waits for the stalled brain's follow-up turn to journal its marker.
async fn await_continuation(runtime: &Arc<CompanyRuntime>) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !continued(runtime) {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

/// A running company with one tool call parked and a brain that will stall
/// on the follow-up turn until `release` is fired.
struct StalledCompany {
    app: axum::Router,
    runtime: Arc<CompanyRuntime>,
    approval_id: ApprovalId,
    /// Fires once the follow-up turn has begun — by which point the verdict
    /// is journaled and the grant minted.
    entered: Arc<tokio::sync::Notify>,
    /// The test's permission for that turn to finish.
    release: Arc<tokio::sync::Notify>,
}

async fn stalled_company(home: &std::path::Path) -> StalledCompany {
    stalled_company_parking(home, gated_tool_call()).await
}

/// `stalled_company`, with the parked effect chosen by the caller — because
/// whether a scope may be granted is decided by the effect, not the route.
async fn stalled_company_parking(
    home: &std::path::Path,
    parked: crate::ports::types::Effect,
) -> StalledCompany {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let state = build_state_with_brain(
        home,
        "running",
        AppConfig::default(),
        Some(Arc::new(StalledContinuationBrain {
            entered: entered.clone(),
            release: release.clone(),
            parked,
        })),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    let response = app.clone().oneshot(chat_request("do it")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let parked = runtime.pending_approvals();
    assert_eq!(parked.len(), 1, "the brain parked one tool call");
    let approval_id = parked[0].id.clone();

    StalledCompany {
        app,
        runtime,
        approval_id,
        entered,
        release,
    }
}

/// **Issue #383 / #380 defect 3 — the keystone.** A client that walks away
/// mid-turn must not take the agent's continuation with it.
///
/// The host is plain `axum::serve(listener, router(state))` and nothing on
/// the resolve path was spawned, so the follow-up agent turn lived *inside*
/// the request future. Hyper drops that future the moment the peer closes,
/// and nginx closes its upstream connection when it gives up on a slow
/// response. So on a hosted tenant the sequence was: verdict recorded,
/// journaled, single-use grant minted — and then the re-dispatch the grant
/// existed for cancelled mid-flight. The operator's approval was spent and
/// the conversation never resumed, which is precisely what #380 reported.
///
/// `Router::oneshot` reproduces that cancellation faithfully rather than by
/// analogy: the mechanism is the same one hyper uses — the handler future is
/// owned by the future the caller is polling, and dropping the latter drops
/// the former.
#[tokio::test]
async fn a_dropped_connection_does_not_cancel_the_follow_up_cycle() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    // Approve it, then let the connection die once the turn is under way.
    let mut resolving = Box::pin(c.app.clone().oneshot(resolve_request(
        &c.approval_id,
        serde_json::json!({"verdict":"approve"}),
    )));
    tokio::select! {
        _ = &mut resolving => panic!("the resolve answered before the follow-up turn began"),
        _ = c.entered.notified() => {}
    }
    drop(resolving);

    // The verdict is already durable and the grant already spent — this is
    // the state the operator is left in when the proxy gives up.
    assert!(
        !c.runtime
            .pending_approvals()
            .iter()
            .any(|a| a.id == c.approval_id),
        "the verdict was journaled before the connection dropped"
    );
    assert!(
        c.runtime.grants.peek(&c.approval_id).is_some(),
        "the single-use grant was minted before the connection dropped"
    );

    // So the continuation the grant exists for must still complete.
    c.release.notify_one();
    assert!(
        await_continuation(&c.runtime).await,
        "the follow-up cycle died with the dropped connection: the grant is spent \
         and the agent never continued"
    );
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "the continuation minted no second grant"
    );
}

/// The reply a stalled chat turn produces once released.
const SLOW_TURN_REPLY: &str = "the slow turn's answer";

/// A brain that stalls on the operator's **first** turn — the chat lane,
/// rather than the approval follow-up `StalledContinuationBrain` stalls on.
struct StalledChatBrain {
    /// Fires once the turn is under way, which is the moment the field
    /// report's proxy gave up and closed the connection.
    entered: Arc<tokio::sync::Notify>,
    /// The test's permission for that turn to finish.
    release: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl crate::ports::brain::Brain for StalledChatBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        _host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        let mut channel_responses = Vec::new();
        for event in &req.events {
            if matches!(event, CompanyEvent::OperatorMessage { .. }) {
                self.entered.notify_one();
                self.release.notified().await;
                channel_responses.push(crate::ports::types::OutboundMessage {
                    message_id: None,
                    task_id: None,
                    outputs: Vec::new(),
                    channel: "operator".into(),
                    agent: None,
                    text: SLOW_TURN_REPLY.into(),
                    steps: Vec::new(),
                    reply_to: None,
                    mentions: Vec::new(),
                });
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses,
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "stalled chat",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

/// Whether the turn's answer reached the durable journal.
async fn reply_journaled(runtime: &Arc<CompanyRuntime>) -> bool {
    runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .iter()
        .any(|stored| {
            matches!(
                &stored.event,
                CompanyEvent::AgentReply { text, .. } if text == SLOW_TURN_REPLY
            )
        })
}

/// Waits for the released turn to journal its reply.
async fn await_reply_journaled(runtime: &Arc<CompanyRuntime>) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !reply_journaled(runtime).await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

/// **Issue #882.** A chat turn whose caller walks away mid-flight must still
/// finish and still journal its answer.
///
/// This is the chat-lane twin of
/// `a_dropped_connection_does_not_cancel_the_follow_up_cycle`. Both the
/// cycle and the `AgentReply` append used to live inside the request future,
/// so a turn slower than nginx's read timeout was cancelled mid-flight and
/// the answer was never written. The operator's DM history then held their
/// question and nothing else — the turn could not be read back on reload and
/// could not be resumed, which is what #882 reported. Workflow runs survived
/// the identical 504 precisely because they are spawned.
///
/// `Router::oneshot` reproduces the cancellation by the same mechanism hyper
/// uses: the handler future is owned by the future the caller polls, so
/// dropping the latter drops the former.
#[tokio::test]
async fn a_dropped_connection_does_not_lose_the_chat_turns_work() {
    let home_dir = home();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(Arc::new(StalledChatBrain {
            entered: entered.clone(),
            release: release.clone(),
        })),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    // Send the turn, then let the connection die once it is under way —
    // exactly what the proxy does when it decides the upstream is too slow.
    let mut chatting = Box::pin(app.clone().oneshot(chat_request("run the seo audit")));
    tokio::select! {
        _ = &mut chatting => panic!("the chat answered before the turn began"),
        _ = entered.notified() => {}
    }
    drop(chatting);

    // Nothing is journaled yet: the turn is still stalled inside the brain.
    assert!(
        !reply_journaled(&runtime).await,
        "the reply was journaled before the turn was released"
    );

    // Issue #983: the turn was recorded the instant it was accepted, and
    // the record is what a re-read resolves — so at this point the operator
    // has walked away and the turn is still `Running` rather than absent.
    let row = turn_rows(&runtime)
        .await
        .pop()
        .expect("accepting the turn minted a row");
    assert_eq!(
        row.1, "running",
        "a turn whose caller is gone must still read as under way"
    );

    // The work must survive the caller giving up.
    release.notify_one();
    assert!(
        await_reply_journaled(&runtime).await,
        "the chat turn died with the dropped connection: the operator's \
         message is journaled, the answer is not, and the turn can neither \
         be read back nor resumed (issue #882)"
    );

    // Issue #983: and so must the settle. The row is written by the spawned
    // task, not by the handler, so a dropped connection leaving it
    // `Running` forever would be the #882 bug one layer down — the turn
    // finishes, the answer lands, and the status surface still claims work
    // is in flight until the next boot reaps it.
    until("the settle died with the dropped connection", async || {
        turn_rows(&runtime)
            .await
            .iter()
            .all(|(_, status)| status == "succeeded")
    })
    .await;
}

// ── Issue #983: an accepted turn exists and can be read back ────────────

/// A brain that blocks every operator turn on a semaphore the test holds.
///
/// Deliberately a `Semaphore` rather than a `Notify`: these tests run two
/// turns at once and release both, and `notify_one` wakes exactly one
/// waiter while `notify_waiters` wakes only those already parked. Permits
/// are held whether or not anybody is waiting yet, so the release cannot
/// race the turns into a hang.
struct BlockingChatBrain {
    /// One permit added per turn that has entered the brain.
    entered: Arc<tokio::sync::Semaphore>,
    /// The test's permission for a turn to finish — one permit each.
    release: Arc<tokio::sync::Semaphore>,
}

impl BlockingChatBrain {
    fn new() -> (
        Arc<Self>,
        Arc<tokio::sync::Semaphore>,
        Arc<tokio::sync::Semaphore>,
    ) {
        let entered = Arc::new(tokio::sync::Semaphore::new(0));
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        (
            Arc::new(Self {
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
            }),
            entered,
            release,
        )
    }
}

#[async_trait::async_trait]
impl crate::ports::brain::Brain for BlockingChatBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        _host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        let mut channel_responses = Vec::new();
        for event in &req.events {
            if let CompanyEvent::OperatorMessage { text, .. } = event {
                self.entered.add_permits(1);
                self.release.acquire().await.expect("released").forget();
                channel_responses.push(crate::ports::types::OutboundMessage {
                    message_id: None,
                    task_id: None,
                    outputs: Vec::new(),
                    channel: "operator".into(),
                    agent: None,
                    text: format!("answered: {text}"),
                    steps: Vec::new(),
                    reply_to: None,
                    mentions: Vec::new(),
                });
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses,
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "blocking chat",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

/// Polls `f` until it holds, or fails the test.
async fn until(label: &str, mut f: impl AsyncFnMut() -> bool) {
    let ok = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !f().await {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok();
    assert!(ok, "{label}");
}

/// The operator messages `chat/history` currently shows for the main desk.
async fn history_texts(app: &axum::Router) -> Vec<String> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/chat/history")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    body.as_array()
        .expect("the history route answers with an array")
        .iter()
        .filter_map(|m| m["text"].as_str().map(str::to_string))
        .collect()
}

/// The company's turn rows, id → status.
async fn turn_rows(runtime: &Arc<CompanyRuntime>) -> Vec<(String, String)> {
    let mut rows = runtime
        .runs()
        .list_runs(runtime.id(), &crate::ports::runs::RunFilter::default())
        .await
        .unwrap();
    rows.sort_by_key(|r| r.created_at_millis);
    rows.into_iter()
        .map(|r| (r.id, r.status.to_string()))
        .collect()
}

/// **Issue #983 — the direct regression for the observed empty history.**
///
/// The operator's message used to be appended *inside* the per-company
/// serial lock, so a message sent while another turn held that lock did not
/// exist anywhere until the turn ahead of it finished. Reloading during a
/// long turn showed an empty conversation: the operator could not see their
/// own question, could not tell whether it had been received, and re-sent it.
///
/// The blocking first turn is what makes this a real test. With a single
/// turn the lock is free and the cycle appends immediately, so the bug is
/// invisible — which is exactly why it survived. Two turns reproduce the
/// serial train the field report saw with five.
#[tokio::test]
async fn a_queued_message_is_in_the_transcript_before_its_turn_runs() {
    let home_dir = home();
    let (brain, entered, release) = BlockingChatBrain::new();
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(brain),
    )
    .await;
    let app = router(state);

    // Turn one takes the lock and stops inside the brain.
    let first = tokio::spawn({
        let app = app.clone();
        async move { app.oneshot(chat_request("the first question")).await }
    });
    entered.acquire().await.expect("turn one entered").forget();

    // Turn two is accepted while turn one still owns the lock.
    let second = tokio::spawn({
        let app = app.clone();
        async move { app.oneshot(chat_request("the second question")).await }
    });

    until(
        "the queued message never reached the transcript",
        async || {
            history_texts(&app)
                .await
                .iter()
                .any(|t| t == "the second question")
        },
    )
    .await;

    // …and it is there while its turn is provably not finished: no answer
    // has been journaled for either message.
    let texts = history_texts(&app).await;
    assert!(
        texts.contains(&"the first question".to_string()),
        "the running turn's own message is missing: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("answered:")),
        "a turn finished before the assertion could run: {texts:?}"
    );

    release.add_permits(2);
    for turn in [first, second] {
        let response = turn.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

/// One POST journals **exactly one** `OperatorMessage`, and the response's
/// `messageId` is that message's own sequence.
///
/// The pin for the pre-journaled cycle path. The route now appends the
/// message itself and hands the cycle the seq; a cycle that appended again
/// would double every operator message in every transcript, and one that
/// reported a seq of its own would hand the console an id that resolves to
/// the wrong line — both silent, both only visible here.
#[tokio::test]
async fn one_post_journals_one_message_and_reports_its_seq() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    let response = app
        .clone()
        .oneshot(chat_request("just the one"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();

    let journaled: Vec<(EventSeq, String)> = runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::OperatorMessage { text, .. } => Some((s.seq, text)),
            _ => None,
        })
        .collect();
    assert_eq!(
        journaled.len(),
        1,
        "one POST must journal one message, got {journaled:?}"
    );
    assert_eq!(journaled[0].1, "just the one");
    assert_eq!(
        body["messageId"].as_str(),
        Some(journaled[0].0.value().to_string().as_str()),
        "messageId must resolve to the message's own line"
    );
}

/// **Issue #983 — the direct regression for the serial train.**
///
/// The per-company cycle lock is held for a whole turn with unbounded
/// waiters, so five concurrent messages became a queue and the fifth
/// inherited the whole queue's latency. Nothing recorded that, so an
/// operator watching a slow company could not tell "my turn is queued" from
/// "my turn is wedged" from "nothing was received".
///
/// The two statuses are what makes the wait legible, which is why the row
/// is created at accept and started only once the cycle holds the lock.
/// Collapsing them — starting the row where it is created — would make both
/// turns read `Running`, and this assertion is what stops that.
#[tokio::test]
async fn a_queued_turn_is_pending_while_the_running_one_holds_the_lock() {
    let home_dir = home();
    let (brain, entered, release) = BlockingChatBrain::new();
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(brain),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    let first = tokio::spawn({
        let app = app.clone();
        async move { app.oneshot(chat_request("first")).await }
    });
    entered.acquire().await.expect("turn one entered").forget();
    let second = tokio::spawn({
        let app = app.clone();
        async move { app.oneshot(chat_request("second")).await }
    });

    // Compared as a sorted pair rather than in row order: two POSTs a
    // millisecond apart can tie on `created_at_millis`, and what is being
    // asserted is that the two turns hold *different* statuses at once, not
    // which row the store lists first.
    until(
        "the second turn never queued behind the first",
        async || {
            let mut statuses: Vec<String> = turn_rows(&runtime)
                .await
                .into_iter()
                .map(|(_, status)| status)
                .collect();
            statuses.sort();
            statuses == ["pending", "running"]
        },
    )
    .await;

    release.add_permits(2);
    for turn in [first, second] {
        assert_eq!(turn.await.unwrap().unwrap().status(), StatusCode::OK);
    }

    until("both turns must reach a terminal status", async || {
        turn_rows(&runtime)
            .await
            .iter()
            .all(|(_, status)| status == "succeeded")
    })
    .await;
    assert_eq!(turn_rows(&runtime).await.len(), 2, "one row per POST");
}

/// The same proof over a **real socket**, so the keystone rests on hyper's
/// actual behaviour rather than on `oneshot` being a good model of it.
///
/// This boots the production server — `axum::serve` over a bound
/// `TcpListener` — writes the resolve by hand, and then hangs up mid-turn
/// the way a proxy does when it gives up on a slow upstream. Hyper reads the
/// peer's close while the handler is still pending and drops the request,
/// which is precisely the cancellation #380's hosted tenant hit. A graceful
/// `FIN` is enough; it does not take a reset.
///
/// **The pause after the close is load-bearing.** Hyper does not learn the
/// peer is gone the instant the client calls `close` — it learns when its
/// connection task next polls the socket and reads EOF. Release the barrier
/// before that happens and the turn finishes on its own merits, so the test
/// passes whether or not the cycle is drop-safe and proves nothing. Measured
/// while building this: without the pause, the pre-fix inline code passed
/// this test; with it, the pre-fix code fails and the fix passes.
#[tokio::test]
async fn a_real_socket_close_does_not_cancel_the_follow_up_cycle() {
    use tokio::io::AsyncWriteExt;

    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = c.app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    let body = serde_json::json!({ "verdict": "approve" }).to_string();
    let request = format!(
        "POST /api/v1/company/approvals/{} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Cookie: {}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        c.approval_id,
        crate::server::test_support::fixed_cookie("acme"),
        body.len(),
    );
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();

    // The turn is under way — the verdict is journaled and the grant minted.
    // Now the client goes away without ever reading a response, and we wait
    // for hyper to actually notice (see the note above).
    c.entered.notified().await;
    socket.shutdown().await.unwrap();
    drop(socket);
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    assert!(
        c.runtime.grants.peek(&c.approval_id).is_some(),
        "the grant was minted before the socket closed"
    );

    c.release.notify_one();
    assert!(
        await_continuation(&c.runtime).await,
        "a real peer close cancelled the follow-up cycle: the grant is spent \
         and the agent never continued"
    );
    assert_eq!(c.runtime.grants.live_count(), 1);
    server.abort();
}

/// `detach` answers on the verdict, not on the turn (issue #383).
///
/// This is the half that removes the *wait*, and with it #380's gateway
/// timeout: the response is already in the operator's hands while the agent
/// is demonstrably still mid-turn. The continuation then arrives on the
/// event stream, where the console is already subscribed.
#[tokio::test]
async fn a_detached_resolve_answers_before_the_turn_finishes() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        c.app.clone().oneshot(resolve_request(
            &c.approval_id,
            serde_json::json!({"verdict":"approve","detach":true}),
        )),
    )
    .await
    .expect("a detached resolve must not wait on the agent turn")
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "recorded": true, "alreadyResolved": false, "stillAwaiting": 0, "outcome": "settled" })
    );

    // The answer really did precede the work: the turn is only now under
    // way, and is still blocked.
    c.entered.notified().await;
    assert!(
        !continued(&c.runtime),
        "the turn had already finished, so this proved nothing about waiting"
    );
    assert!(
        c.runtime.grants.peek(&c.approval_id).is_some(),
        "the grant is minted before the response, not after the turn"
    );

    c.release.notify_one();
    assert!(
        await_continuation(&c.runtime).await,
        "a detached continuation must still land"
    );
    assert_eq!(c.runtime.grants.live_count(), 1);
}

/// **Issue #431.** A *detached* resolve — the verb the inline chat card
/// uses — mints a standing grant when one was asked for, and that grant is
/// visible on the one list both surfaces read.
///
/// This pairing had no coverage before this test, which is why it looked
/// covered: its neighbour above sends `detach` with no scope, so the grant
/// it asserts is the single-use one, while every standing-grant test
/// resolves *without* `detach`. Until #431 the console could not ask for
/// this combination at all, so nothing exercised it; now the chat card can,
/// it is the console's only way to mint a standing grant.
///
/// It holds because `run_resolve` computes the scope and hands it to
/// `resolve_approval_spawned` *before* it branches on `detach` — statement
/// ordering inside one function, which a refactor could reverse without a
/// single existing test going red. Hence asserting it rather than reading it.
#[tokio::test]
async fn a_detached_resolve_mints_a_standing_grant_and_lists_it() {
    let home_dir = home();
    let c = stalled_company_parking(home_dir.path(), grantable_tool_call()).await;

    // The same flag the inline card gates its scope control on: if this is
    // false the console offers no choice, and the rest of this is moot.
    assert!(
        c.runtime.pending_approvals()[0].broadly_grantable,
        "the fixture must be an approval a standing scope may be asked for"
    );

    let response = c
        .app
        .clone()
        .oneshot(resolve_request(
            &c.approval_id,
            serde_json::json!({
                "verdict": "approve",
                "detach": true,
                "scope": "tool",
                "expires_in_millis": 60 * 60 * 1000,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Minted by the time the detached answer is written, not merely promised
    // by it — the receipt says `recorded`, so a grant that appeared later
    // would make that a lie.
    assert_eq!(
        c.runtime.grants.standing_count(),
        1,
        "a detached resolve carrying a tool scope must mint a standing grant"
    );

    // And visible on the one list route both surfaces read, described the
    // same way — this is what "appears in the same list as one granted from
    // the page" means, there being only one list.
    let listed = c
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/grants")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let bytes = to_bytes(listed.into_body(), usize::MAX).await.unwrap();
    let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["tool"], "file_write");
    assert_eq!(rows[0]["agent"], "ceo");

    // The continuation still lands, so the scope did not cost the detach.
    c.entered.notified().await;
    c.release.notify_one();
    assert!(
        await_continuation(&c.runtime).await,
        "a detached continuation must still land when a scope was granted"
    );
}

/// The default is unchanged: no `detach` key means the response still
/// carries the follow-up cycle's messages, in the same `ChatResponse` shape
/// every existing caller parses. Only the drop-safety is new.
#[tokio::test]
async fn the_default_resolve_still_answers_with_the_cycle_response() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;

    // Let the turn through the moment it starts.
    c.release.notify_one();
    let response = c
        .app
        .clone()
        .oneshot(resolve_request(
            &c.approval_id,
            serde_json::json!({"verdict":"approve"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value.get("responses").is_some_and(|r| r.is_array()),
        "the un-detached body is still a ChatResponse, got {value}"
    );
    assert!(
        continued(&c.runtime),
        "the un-detached resolve waited for the turn, as it always did"
    );
    assert_eq!(c.runtime.grants.live_count(), 1);
}

/// A second resolve of the same approval is a success, not a failure, and
/// mints nothing (issue #243). `detach` reports that as `alreadyResolved`,
/// which is what makes a retry after a timeout safe to *show* as a retry
/// rather than as an error — the thing #380's operator had no way to know.
#[tokio::test]
async fn a_second_resolve_reports_already_resolved_and_mints_nothing() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;
    c.release.notify_one();

    let first = c
        .app
        .clone()
        .oneshot(resolve_request(
            &c.approval_id,
            serde_json::json!({"verdict":"approve","detach":true}),
        ))
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let bytes = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["alreadyResolved"], false);
    assert!(await_continuation(&c.runtime).await);

    let second = c
        .app
        .clone()
        .oneshot(resolve_request(
            &c.approval_id,
            serde_json::json!({"verdict":"approve","detach":true}),
        ))
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let bytes = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "recorded": true, "alreadyResolved": true, "stillAwaiting": 0, "outcome": "already_resolved" })
    );
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "re-approving minted no second grant"
    );
}

/// **Issue #1449 on the wire.** A card past its deadline answers `expired`,
/// on both response shapes, and journals no approval against the operator.
///
/// The two shapes matter independently. The **detached** receipt is what the
/// inline chat card reads; the **synchronous** `ChatResponse` is what the
/// Approvals page reads — the surface the defect was reported on — and it
/// never sees a receipt at all, so a discriminator that only rode on the
/// receipt would have left the reproduced bug in place.
#[tokio::test]
async fn a_resolve_past_the_deadline_answers_expired_on_both_shapes() {
    let home_dir = home();
    // `approval_ttl_hours = 0`: anything parked is past its deadline the
    // instant it lands, which is the state an operator meets when they get
    // to a queue late.
    let expiring: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\napproval_ttl_hours = 0\n",
    )
    .unwrap();
    let state = build_state_with_brain_and_manifest(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(Arc::new(StalledContinuationBrain {
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            parked: gated_tool_call(),
        })),
        expiring,
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let app = router(state);

    let response = app.clone().oneshot(chat_request("do it")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let approval_id = runtime.pending_approvals()[0].id.clone();

    // The detached shape.
    let detached = app
        .clone()
        .oneshot(resolve_request(
            &approval_id,
            serde_json::json!({"verdict":"approve","detach":true}),
        ))
        .await
        .unwrap();
    assert_eq!(detached.status(), StatusCode::OK);
    let bytes = to_bytes(detached.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["outcome"], "expired",
        "the host default-denied this; the receipt has to be able to say so, got {value}"
    );
    assert_eq!(
        runtime.grants.live_count(),
        0,
        "and it minted nothing, as it always did"
    );

    // The synchronous shape, on a second card of the same company.
    let response = app
        .clone()
        .oneshot(chat_request("do it again"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = runtime.pending_approvals()[0].id.clone();
    let sync = app
        .clone()
        .oneshot(resolve_request(
            &second,
            serde_json::json!({"verdict":"approve"}),
        ))
        .await
        .unwrap();
    assert_eq!(sync.status(), StatusCode::OK);
    let bytes = to_bytes(sync.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value.get("responses").is_some_and(|r| r.is_array()),
        "still a ChatResponse, got {value}"
    );
    assert_eq!(
        value["outcome"], "expired",
        "the Approvals page's own shape carries it too, got {value}"
    );
    assert_eq!(runtime.grants.live_count(), 0);
}

/// Both scope forms carry `detach` identically — the `/companies/{id}` route
/// and the single-company alias are the same handler, and a console pointed
/// at either must get the same contract.
#[tokio::test]
async fn detach_works_on_the_company_id_scope_too() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;
    c.release.notify_one();

    let response = c
        .app
        .clone()
        .oneshot(resolve_request_scoped(
            "/api/v1/companies/acme",
            &c.approval_id,
            serde_json::json!({"verdict":"approve","detach":true}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "recorded": true, "alreadyResolved": false, "stillAwaiting": 0, "outcome": "settled" })
    );
    assert!(await_continuation(&c.runtime).await);
    assert_eq!(c.runtime.grants.live_count(), 1);
}

#[tokio::test]
async fn deny_with_amended_payload_is_400() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    // The contradiction is rejected before anything is settled, so `detach`
    // cannot turn it into a `200 { recorded: true }` over a decision that was
    // never taken (issue #383).
    for body in [
        r#"{"verdict":"deny","amended_payload":{"text":"edited"}}"#,
        r#"{"verdict":"deny","amended_payload":{"text":"edited"},"detach":true}"#,
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/approvals/missing")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "for {body}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["code"], "invalid_request");
    }
}

#[tokio::test]
async fn a_session_is_required_and_sufficient() {
    // Replaces `operator_token_guards_routes`. That token could never be
    // set, so the test only ever proved the guard worked in a state no
    // deployment could reach; every real host served this route to anyone.
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = build_state(&home, "running", AppConfig::default()).await;

    // No credential at all: closed.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/companies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A garbage bearer buys nothing either — there is no bearer path in
    // prosumer mode at all now.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/companies")
                .header("authorization", "Bearer nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A signed-in human gets their own company.
    let cookie = crate::server::test_support::seed_admin(&state, "acme").await;
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/companies")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

// ---- issue #66: the operator attention SSE feed ----

use crate::ports::types::{EventSeq, StoredEvent};

fn stored(event: CompanyEvent) -> StoredEvent {
    StoredEvent {
        seq: EventSeq::new(7),
        company: CompanyId::new("acme"),
        event,
        at_millis: 1_700_000_000_000,
    }
}

#[test]
fn projects_a_gap_with_structural_fields_only() {
    let value = super::project_stream_item_for_viewer(
        &EventStreamItem::Gap { missed: 44 },
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("a gap must reach the console");
    assert_eq!(
        value,
        serde_json::json!({ "type": "stream_gap", "missed": 44 })
    );
}

#[test]
fn projects_agent_reply_with_chat_fields_and_steps() {
    use crate::ports::types::{TurnStep, TurnStepKind, TurnStepStatus};
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "shipped it".into(),
        steps: vec![TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "Reading messages".into(),
            detail: None,
            elapsed_ms: Some(12),
            ..TurnStep::default()
        }],
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(v["type"], "agent_reply");
    assert_eq!(v["seq"], 7);
    assert_eq!(v["atMillis"], 1_700_000_000_000_u64);
    assert_eq!(v["chatId"], "General");
    assert_eq!(v["agentId"], "ceo");
    assert_eq!(v["text"], "shipped it");
    // The scrubbed timeline rides along so a live listener sees the steps.
    assert_eq!(v["steps"][0]["label"], "Reading messages");
    assert_eq!(v["steps"][0]["status"], "ok");
    // A channel reply names no thread, so the legacy frame is unchanged.
    assert!(v.get("parentId").is_none(), "unexpected parentId: {v}");
}

/// **The live frame carries the model's own body, not only the operator's.**
///
/// `MessageView` has shipped both since it gained `cue_text`; the live frame
/// had only `text`, so anything needing the room's grammar had to scrape it
/// back out of the operator-facing body. `frontend/src/lib/hive/episode.ts`
/// does exactly that (`moveOf(m.text)`), which is why rewriting `text` here
/// costs the deliberation panel rather than merely tidying a bubble.
///
/// Pinned now, while the two are equal, so the step that rewrites `text`
/// cannot quietly take `cueText` with it.
#[test]
fn projects_the_agents_own_body_beside_the_operators() {
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "returns".into(),
        agent_id: "refunds".into(),
        text: "!support #kettle ^16 the swap is the customer's first preference".into(),
        steps: Vec::new(),
    });
    let value = super::project_event_for_viewer(
        &stored,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("agent_reply is an attention signal");

    assert_eq!(
        value["cueText"], "!support #kettle ^16 the swap is the customer's first preference",
        "the room's grammar is what the fold reads; it must survive on this frame: {value}"
    );
    assert_eq!(
        value["text"], "the swap is the customer's first preference",
        "and the operator reads prose, exactly as the reload already gives them: {value}"
    );
}

/// And on a desk that does not deliberate the two are byte-equal, so no
/// consumer has to choose between them for an ordinary reply.
#[test]
fn a_reply_with_no_move_carries_the_same_body_twice() {
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "general".into(),
        agent_id: "ceo".into(),
        text: "here is the summary you asked for".into(),
        steps: Vec::new(),
    });
    let value = super::project_event_for_viewer(
        &stored,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("agent_reply is an attention signal");

    assert_eq!(value["cueText"], value["text"]);
}

#[test]
fn projects_agent_reply_with_viewer_mention_metadata() {
    use crate::ports::types::{Mention, MentionTarget};
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: vec![
            Mention {
                target: MentionTarget::User { id: "u-1".into() },
                text: "@Ada".into(),
                offset: 0,
                quiet: false,
            },
            Mention {
                target: MentionTarget::Everyone,
                text: "@everyone".into(),
                offset: 5,
                quiet: true,
            },
        ],
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "@Ada @everyone".into(),
        steps: Vec::new(),
    });
    let authors = std::collections::HashMap::from([(String::from("u-1"), String::from("Ada"))]);
    let value =
        super::project_event_for_viewer(&stored, &authors, &Viewer::User("u-1".into()), false)
            .expect("agent_reply is an attention signal");
    assert_eq!(
        value["mentions"],
        serde_json::json!([
            { "text": "@Ada", "offset": 0, "label": "Ada", "mine": true },
            { "text": "@everyone", "offset": 5, "label": "everyone", "mine": true, "quiet": true },
        ])
    );
}

/// Issue #1781 review, Codex P1: `history_for_desk` already hides an
/// owner-fallback report from a non-admin on reload; this proves the live
/// SSE projection agrees, rather than handing a non-admin console the full
/// admin-only text the instant it lands.
#[test]
fn drops_owner_fallback_report_from_a_non_admin_viewer() {
    let event = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "operator".into(),
        agent_id: crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR.to_string(),
        text: "no admin has a mailbox".into(),
        steps: Vec::new(),
    });

    let non_admin = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::User("member-1".into()),
        false,
    );
    assert!(
        non_admin.is_none(),
        "a non-admin viewer must not receive the admin-only report live: {non_admin:?}"
    );

    let admin = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::User("admin-1".into()),
        true,
    )
    .expect("an admin viewer still receives the report live");
    assert_eq!(
        admin["agentId"],
        crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR
    );

    // The Operator viewer (issue #66's original, unrestricted principal)
    // must see it too — same as `project_event`'s `is_admin: true` default.
    let operator = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("the operator viewer still receives the report live");
    assert_eq!(operator["text"], "no admin has a mailbox");
}

#[test]
fn projects_agent_reply_with_its_thread_parent() {
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: Some(EventSeq::new(4)),
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "in the thread".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(v["parentId"], "4");
}

/// Issue #983: the accept frame carries the turn, the desk and the thread —
/// and **nothing else**.
///
/// The negative half is what this test is for. `TurnStarted` is the first
/// frame on this stream that brackets an operator's own message, so it is
/// the obvious place for somebody to "helpfully" add the text or the asker
/// — which is exactly the payload the deny-by-default projection exists to
/// keep off the wire, and which `OperatorMessage` is dropped to avoid.
#[test]
fn projects_turn_started_with_structural_keys_only() {
    use crate::ports::types::{Actor, ActorKind};
    let v = super::project_event(&stored(CompanyEvent::TurnStarted {
        turn_id: "turn-1".into(),
        chat_id: "General".into(),
        parent: Some(EventSeq::new(4)),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u-1".into(),
        }),
    }))
    .expect("an accepted turn is an attention signal");
    assert_eq!(v["type"], "turn_started");
    assert_eq!(v["turnId"], "turn-1");
    assert_eq!(v["chatId"], "General");
    assert_eq!(v["parentId"], "4");
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["type", "seq", "atMillis", "turnId", "chatId", "parentId"],
        "the accept frame grew a key: {v}"
    );

    // A turn answering the channel itself omits the thread rather than
    // sending null, so the console's check is a presence check.
    let v = super::project_event(&stored(CompanyEvent::TurnStarted {
        turn_id: "turn-2".into(),
        chat_id: "General".into(),
        parent: None,
        by: None,
    }))
    .expect("an accepted turn is an attention signal");
    assert!(v.get("parentId").is_none(), "unexpected parentId: {v}");
}

/// The settle frame says a turn is over and **not why**.
///
/// `TurnFailed::error` is a reason in our own words that can name
/// internals; the console learns the reason from the tenant-scoped run row.
#[test]
fn projects_turn_settled_without_the_failure_reason() {
    let v = super::project_event(&stored(CompanyEvent::TurnFailed {
        turn_id: "turn-1".into(),
        error: "connection to db-primary.internal refused".into(),
    }))
    .expect("a settled turn is an attention signal");
    assert_eq!(v["type"], "turn_settled");
    assert_eq!(v["turnId"], "turn-1");
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["type", "seq", "atMillis", "turnId"],
        "the settle frame grew a key: {v}"
    );
}

/// The operator's own message is **still** dropped (issue #983).
///
/// Pinned because #983 added the two arms above right beside it, and the
/// natural next step — "the console needs the message too, project it" —
/// would put operator-authored free text onto this stream for the first
/// time. It does not need it: the message is already in the POST's own
/// response and in `chat/history`, which is the point of journaling it at
/// accept time. If somebody later decides otherwise, they say so here.
#[test]
fn projects_nothing_for_the_operators_own_message() {
    assert!(
        super::project_event(&stored(CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            text: "the operator's own words".into(),
            by: None,
            chat: Some("General".into()),
            parent: None,
            deliverable: None,
            attachments: Vec::new(),
        }))
        .is_none(),
        "the operator's own message must not reach the console over SSE"
    );
}

/// A reaction is deliberately NOT on the attention stream (issue #364).
///
/// Pinned rather than left to the deny-by-default fall-through, because the
/// omission is a decision and not an oversight: the frame would have to
/// carry the reacting person, and this stream has no per-viewer projection
/// to turn an actor into a label. Reload-visibility is what the issue asks
/// for. If someone later decides reactions should stream, this test is
/// where they say so out loud.
#[test]
fn projects_nothing_for_a_reaction() {
    assert!(
        super::project_event(&stored(CompanyEvent::ReactionToggled {
            message_seq: EventSeq::new(4),
            emoji: "👍".into(),
            on: true,
            by: None,
        }))
        .is_none(),
        "a reaction must not reach the console over SSE"
    );
}

/// Issue #379: the park frame carries an id, a kind and the channel — and
/// **nothing else**.
///
/// The negative half is the load-bearing one. The effect's arguments are
/// redacted in exactly one place (`pending_approvals`), and if this frame
/// ever grew a `payload` key it would become a second surface that has to
/// redact and one day will not. Asserting the absence is what makes that a
/// build failure rather than a leak.
#[test]
fn projects_approval_parked_with_a_channel_and_no_payload() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalParked {
        approval_id: ApprovalId::new("appr-1"),
        effect_kind: "payment.send".into(),
        thread: Some("desk-finance".into()),
    }))
    .expect("a parked approval is an attention signal");
    assert_eq!(v["type"], "approval_parked");
    assert_eq!(v["approvalId"], "appr-1");
    assert_eq!(v["kind"], "payment.send");
    assert_eq!(v["chatId"], "desk-finance");
    for forbidden in ["payload", "agent", "amountUsd", "effect", "args"] {
        assert!(
            v.get(forbidden).is_none(),
            "the park frame must stay thin — `{forbidden}` leaked: {v}",
        );
    }
    assert_eq!(
        v.as_object().unwrap().len(),
        6,
        "type, seq, atMillis, approvalId, kind, chatId — and nothing more: {v}",
    );
}

/// A park with no conversation behind it omits the channel entirely, so a
/// console filtering by thread matches it nowhere and it stays on the
/// Approvals page (#379).
#[test]
fn projects_approval_parked_without_a_channel_when_no_thread_produced_it() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalParked {
        approval_id: ApprovalId::new("appr-cron"),
        effect_kind: "email.send".into(),
        thread: None,
    }))
    .expect("a parked approval is an attention signal");
    assert_eq!(v["type"], "approval_parked");
    assert!(
        v.get("chatId").is_none(),
        "a page-only approval must carry no channel: {v}",
    );
}

#[test]
fn projects_agent_reply_omits_empty_steps() {
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "hi".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    // A tool-less reply keeps the legacy wire shape — no `steps` key.
    assert!(v.get("steps").is_none());
    // …and an uncorrelated reply carries no `taskId` either, so the
    // pre-#185 wire shape is byte-for-byte what it was.
    assert!(v.get("taskId").is_none());
}

/// #185: the correlation key rides the SSE stream when — and only when — the
/// event carries one. Both directions matter: its presence is what lets a
/// live console route a frame to the right task, and its absence is what
/// keeps the legacy shape intact for every ordinary chat reply.
#[test]
fn projects_task_id_only_when_the_event_is_correlated() {
    let reply = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: Some("t-1".into()),
        outputs: Vec::new(),
        chat_id: "t-1".into(),
        agent_id: "ceo".into(),
        text: "on it".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(reply["taskId"], serde_json::json!("t-1"));

    let failure = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: Some("t-1".into()),
        server: "gh".into(),
        tool: "issues".into(),
        status: "credential_required".into(),
        message: "needs auth".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert_eq!(failure["taskId"], serde_json::json!("t-1"));

    let uncorrelated = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: None,
        server: "gh".into(),
        tool: "issues".into(),
        status: "credential_required".into(),
        message: "needs auth".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert!(uncorrelated.get("taskId").is_none());
}

/// #185/#377: the dispatch terminal projects the structural fields, plus
/// the conversation the card was raised from. `column` is the one that
/// matters most — it is how a console tells a clean finish from a cancelled
/// or failed run — and `chatId` is what says which channel it belongs in.
#[test]
fn projects_desk_task_completed_with_every_field() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["type"], serde_json::json!("desk_task_completed"));
    assert_eq!(v["taskId"], serde_json::json!("t-1"));
    assert_eq!(v["desk"], serde_json::json!("engineer"));
    assert_eq!(v["column"], serde_json::json!("in_review"));
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    // The envelope's own keys still ride along — the console mints the
    // marker's identity from `seq` (issue #483's mechanism), so losing it
    // here would silently disable the reload dedupe.
    assert!(v.get("seq").is_some(), "{v}");
    assert!(v.get("atMillis").is_some(), "{v}");
}

/// Issue #377: the run's prose is **not** on this frame.
///
/// The relay bubble (#151) already carries the agent's words into the same
/// channel this marker lands in. Projecting `output` here as well would put
/// one run's text into one conversation twice, and dropping it at the
/// projection is what stops any later reader from reintroducing that.
#[test]
fn desk_task_completed_does_not_project_the_runs_prose() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "the whole reply, verbatim".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert!(v.get("output").is_none(), "{v}");
    assert!(
        !v.to_string().contains("the whole reply"),
        "the prose must not reach the wire under any key: {v}"
    );
}

/// Issue #377: a card nobody raised from a conversation omits `chatId`
/// rather than sending null — so "board-created" is a presence check on the
/// console, the same shape `approval_parked` uses for a page-only approval.
#[test]
fn desk_task_completed_omits_the_chat_id_for_a_board_created_card() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: None,
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert!(v.get("chatId").is_none(), "{v}");
    assert_eq!(v["column"], serde_json::json!("in_review"));
}

/// Issue #1890 B: the thread inside the channel, on exactly the terms
/// `chatId` rides on.
///
/// Stringified, because the console keys threads by message id and a
/// message id is a string there — `chat/history` renders the same root the
/// same way, and the two must agree or the marker would render inline live
/// and jump into a thread on reload.
#[test]
fn desk_task_completed_projects_the_thread_its_card_was_raised_in() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: Some(crate::ports::types::EventSeq::new(41)),
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    assert_eq!(v["parentId"], serde_json::json!("41"));
}

/// A card raised straight into a channel omits `parentId` rather than
/// sending null — the same presence-check shape `chatId` takes, so the
/// console reads "channel level" without a null check.
#[test]
fn desk_task_completed_omits_the_parent_for_a_channel_level_card() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    assert!(v.get("parentId").is_none(), "{v}");
}

#[test]
fn projects_task_dispatched() {
    let v = super::project_event(&stored(CompanyEvent::TaskDispatched {
        task_id: "t-42".into(),
        run_id: None,
    }))
    .expect("task_dispatched is an attention signal");
    assert_eq!(v["type"], "task_dispatched");
    assert_eq!(v["taskId"], "t-42");
}

/// Issue #464: an opened card reaches the console as its own frame. This is
/// the half a unit test can prove — that the projection exists and carries
/// the card; that the *board* redraws off it is a browser fact.
#[test]
fn projects_task_card_changed() {
    let v = super::project_event(&stored(CompanyEvent::TaskCardChanged {
        task_id: "t-77".into(),
        change: crate::runtime::CHANGE_OPENED.into(),
        column: Some("todo".into()),
    }))
    .expect("a board write is an attention signal");
    assert_eq!(v["type"], "task_card_changed");
    assert_eq!(v["taskId"], "t-77");
    assert_eq!(v["change"], "opened");
    assert_eq!(v["column"], "todo");
}

/// A removed card is projected without a column — the console's "is it
/// gone?" check is a presence check, never a null one.
#[test]
fn projects_a_removed_card_without_a_column() {
    let v = super::project_event(&stored(CompanyEvent::TaskCardChanged {
        task_id: "t-77".into(),
        change: crate::runtime::CHANGE_REMOVED.into(),
        column: None,
    }))
    .expect("a board write is an attention signal");
    assert_eq!(v["change"], "removed");
    assert!(
        v.get("column").is_none(),
        "a removed card is in no column: {v}"
    );
}

/// Issue #327: the workspace's own frame. The stream is deny-by-default, so
/// an event with no arm is silently unprojected — this is what proves the
/// arm exists at all.
///
/// Also pins what is **not** on the wire: no node name, no body. A note's
/// text is operator- or agent-authored free text, and this frame's job is
/// to say something moved, not to carry the tree.
#[test]
fn projects_workspace_changed_without_a_name_or_a_body() {
    let v = super::project_event(&stored(CompanyEvent::WorkspaceChanged {
        node_id: "n-9".into(),
        change: crate::runtime::CHANGE_UPDATED.into(),
    }))
    .expect("a workspace write must reach the console");
    assert_eq!(v["type"], "workspace_changed");
    assert_eq!(v["nodeId"], "n-9");
    assert_eq!(v["change"], "updated");
    assert!(v.get("name").is_none(), "no node name on the wire: {v}");
    assert!(v.get("content").is_none(), "no body on the wire: {v}");
}

#[test]
fn projects_mcp_call_failed_with_scrubbed_message() {
    let v = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: None,
        server: "browserbase".into(),
        tool: "browse".into(),
        status: "tool_call_rejected".into(),
        message: "server rejected the call".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert_eq!(v["type"], "mcp_call_failed");
    assert_eq!(v["server"], "browserbase");
    assert_eq!(v["tool"], "browse");
    assert_eq!(v["status"], "tool_call_rejected");
    // The message is already scrubbed at the source; we forward exactly it.
    assert_eq!(v["message"], "server rejected the call");
}

#[test]
fn projects_approval_resolved_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalResolved {
        approval_id: ApprovalId::new("ap-1"),
        verdict: Verdict::Approve,
        by: Actor {
            kind: ActorKind::User,
            // A user id must never reach the wire via the attention feed.
            id: "secret-user-id".into(),
        },
    }))
    .expect("approval_resolved is an attention signal");
    assert_eq!(v["type"], "approval_resolved");
    assert_eq!(v["approvalId"], "ap-1");
    assert_eq!(v["verdict"], "approve");
    // The actor is intentionally dropped — the projection carries no `by`,
    // and the serialized bytes never mention the user id.
    assert!(v.get("by").is_none(), "actor must not be projected");
    assert!(
        !v.to_string().contains("secret-user-id"),
        "user id leaked onto the wire"
    );
    // Issue #971: and a person's decision carries no `automatic` flag, so
    // the console's "an operator decided this" reading of its absence is
    // the correct one.
    assert!(
        v.get("automatic").is_none(),
        "a user's own decision is not automatic"
    );
}

/// **T6 (issue #971).** A host-side expiry says so, without saying who.
///
/// The defect: an expiry appends `ApprovalResolved { Deny, System }`, this
/// frame dropped the actor, and the console toasted "Approval denied" — so
/// an operator was told they had declined a request they never saw. With a
/// 24-hour deadline that stops being rare.
///
/// The assertion above is **extended here, not replaced**: the new field is
/// a bit derived from `by.kind`, and the no-actor / no-user-id property it
/// is derived from has to keep holding, so it is re-asserted on this arm
/// with a `System` actor whose id is equally secret.
#[test]
fn projects_a_host_side_expiry_as_automatic_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalResolved {
        approval_id: ApprovalId::new("ap-2"),
        verdict: Verdict::Deny,
        by: Actor {
            kind: ActorKind::System,
            // Even the system actor's id stays off the feed: the console
            // needs the *fact* that no person decided this, not the name of
            // the internal path that did.
            id: "expiry".into(),
        },
    }))
    .expect("approval_resolved is an attention signal");
    assert_eq!(v["type"], "approval_resolved");
    assert_eq!(v["approvalId"], "ap-2");
    assert_eq!(v["verdict"], "deny");
    assert_eq!(
        v["automatic"], true,
        "the console must be able to say the deadline passed rather than \
         attributing the deny to whoever is looking at it"
    );
    // The extended property, restated on this arm.
    assert!(v.get("by").is_none(), "actor must not be projected");
    assert!(
        !v.to_string().contains("expiry"),
        "the actor id must not reach the wire on this arm either"
    );
}

#[test]
fn projects_task_steered_without_actor_or_instruction() {
    let v = super::project_event(&stored(CompanyEvent::TaskSteered {
        task_id: "t-9".into(),
        action: "redirect".into(),
        instruction: Some("focus on the API".into()),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        }),
    }))
    .expect("task_steered is an attention signal");
    assert_eq!(v["type"], "task_steered");
    assert_eq!(v["taskId"], "t-9");
    assert_eq!(v["action"], "redirect");
    let wire = v.to_string();
    assert!(!wire.contains("secret-user-id"));
    assert!(!wire.contains("focus on the API"));
}

#[test]
fn projects_workflow_created_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowCreated {
        workflow_id: "greeter".into(),
        name: "Greeter".into(),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        }),
    }))
    .expect("workflow_created is an attention signal");
    assert_eq!(v["type"], "workflow_created");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter");
    assert!(!v.to_string().contains("secret-user-id"));
}

/// Issue #259: the edit and delete signals project the same two fields and
/// drop the actor, exactly like `workflow_created` above.
#[test]
fn projects_workflow_updated_and_deleted_without_the_actor() {
    let actor = || {
        Some(Actor {
            kind: ActorKind::User,
            id: "secret-user-id".into(),
        })
    };

    let v = super::project_event(&stored(CompanyEvent::WorkflowUpdated {
        workflow_id: "greeter".into(),
        name: "Greeter v2".into(),
        by: actor(),
    }))
    .expect("workflow_updated is an attention signal");
    assert_eq!(v["type"], "workflow_updated");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter v2");
    assert!(!v.to_string().contains("secret-user-id"));

    let v = super::project_event(&stored(CompanyEvent::WorkflowDeleted {
        workflow_id: "greeter".into(),
        name: "Greeter".into(),
        by: actor(),
    }))
    .expect("workflow_deleted is an attention signal");
    assert_eq!(v["type"], "workflow_deleted");
    assert_eq!(v["workflowId"], "greeter");
    assert_eq!(v["name"], "Greeter");
    assert!(!v.to_string().contains("secret-user-id"));
}

#[test]
fn projects_lifecycle_changed_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::LifecycleChanged {
        from: "running".into(),
        to: "paused".into(),
        by: Actor {
            kind: ActorKind::Operator,
            id: "operator".into(),
        },
    }))
    .expect("lifecycle_changed is an attention signal");
    assert_eq!(v["type"], "lifecycle_changed");
    assert_eq!(v["from"], "running");
    assert_eq!(v["to"], "paused");
    assert!(v.get("by").is_none(), "actor must not be projected");
}

#[test]
fn projects_payment_received() {
    let v = super::project_event(&stored(CompanyEvent::PaymentReceived {
        amount_usd: 25.0,
        memo: "invoice #1".into(),
    }))
    .expect("payment_received is an attention signal");
    assert_eq!(v["type"], "payment_received");
    assert_eq!(v["amountUsd"], 25.0);
    assert_eq!(v["memo"], "invoice #1");
}

// ---- issue #228: the workflow-run outcome projection ----

fn delivery_row(
    node: &str,
    status: crate::ports::DeliveryStatus,
) -> crate::ports::DeliveryReport {
    crate::ports::DeliveryReport {
        node: node.into(),
        kind: "email".into(),
        target: Some("ada@example.com".into()),
        status,
        detail: "this recipient has never written to the company".into(),
        reason: crate::ports::DeliveryReason::RecipientNotEstablished,
    }
}

/// The live half of #228: a finished run reaches the console as it happens,
/// carrying exactly the fields the run drawer already renders — so the
/// console can toast an undelivered report instead of waiting for a reload.
#[test]
fn projects_workflow_run_finished_with_the_fields_the_drawer_renders() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: vec![
            delivery_row("owner_summary", crate::ports::DeliveryStatus::Skipped),
            delivery_row("also_sent", crate::ports::DeliveryStatus::Sent),
        ],
        pending_approvals: vec!["review".into()],
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["type"], "workflow_run_finished");
    assert_eq!(v["seq"], 7);
    assert_eq!(v["workflowId"], "digest");
    assert_eq!(v["scheduled"], true);
    assert_eq!(v["pendingApprovals"][0], "review");

    // Per-row node/kind/target/status/detail — the same shape the manual
    // run's HTTP response already ships to this console.
    let rows = v["deliveries"].as_array().expect("rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["node"], "owner_summary");
    assert_eq!(rows[0]["kind"], "email");
    assert_eq!(rows[0]["status"], "skipped");
    assert_eq!(rows[0]["target"], "ada@example.com");
    assert!(
        rows[0]["detail"]
            .as_str()
            .unwrap()
            .contains("never written"),
        "the detail names the fix: {v}"
    );

    // A run that finished carries no `error` key, and `runId` — always
    // `None` today — is never a permanently-null key on the wire.
    assert!(v.get("error").is_none(), "{v}");
    assert!(v.get("runId").is_none(), "{v}");
}

/// The failure arm reaches the console too — it is the outcome that used to
/// produce nothing but a host-stdout warning.
#[test]
fn projects_workflow_run_finished_with_the_failure_reason() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: Some("no inference source for agent node `worker`".into()),
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["error"], "no inference source for agent node `worker`");
    assert_eq!(v["deliveries"].as_array().unwrap().len(), 0);
}

/// Issues #881 / #880: the blocked arm reaches the console live.
///
/// Without it a console watching a run settle would be told it finished
/// cleanly — no error, not cancelled, nothing delivered — and then the
/// history it reloads a moment later would say the run blocked. The two
/// surfaces read the same journal event, so they must project the same
/// facts.
#[test]
fn projects_workflow_run_finished_with_its_blocked_nodes_and_parked_approvals() {
    let v = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: Some("run-b".into()),
        deliveries: Vec::new(),
        pending_approvals: vec!["spec".into()],
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: vec![crate::ports::WorkflowBlockedNode {
            node_id: "spec".into(),
            tools: vec!["publish_artifact".into()],
            approval_ids: vec!["appr-1".into()],
            unparkable: 0,
            stranded: 0,
            blockers: 0,
        }],
        approvals: vec![crate::ports::WorkflowRunApprovalRow {
            node_id: Some("spec".into()),
            tool: Some("publish_artifact".into()),
            outcome: crate::ports::WorkflowApprovalOutcome::Parked,
            approval_id: Some("appr-1".into()),
        }],
    }))
    .expect("workflow_run_finished is an attention signal");
    assert_eq!(v["blockedNodes"][0]["nodeId"], "spec");
    assert_eq!(v["blockedNodes"][0]["tools"][0], "publish_artifact");
    assert_eq!(v["approvals"][0]["outcome"], "parked");
    assert!(
        v.get("error").is_none(),
        "a run waiting on a person did not fail: {v}"
    );

    // The presence-check discipline: a run that blocked on nobody sends
    // neither key, so an existing frame is byte-unchanged.
    let clean = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: true,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projects");
    assert!(clean.get("blockedNodes").is_none(), "{clean}");
    assert!(clean.get("approvals").is_none(), "{clean}");
}

/// Issue #371: the live per-node trail. Both arms project, both carry the
/// run id that ties them to the run's settle-frame, and — the point — the
/// node arm carries a status and a duration and nothing else.
#[test]
fn projects_the_per_node_progress_trail() {
    let started = super::project_event(&stored(CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        scheduled: true,
        started_by: None,
        resume_semantic: None,
    }))
    .expect("workflow_run_started reaches the console");
    assert_eq!(started["type"], "workflow_run_started");
    assert_eq!(started["workflowId"], "digest");
    assert_eq!(started["runId"], "run-1");
    assert_eq!(started["scheduled"], true);
    assert!(
        started.get("startedBy").is_none(),
        "no sender projects no key: {started}"
    );

    // Issue #1862 prerequisite: when the journal carries a sender, the SSE
    // frame forwards it under `startedBy`.
    let started_with_sender = super::project_event(&stored(CompanyEvent::WorkflowRunStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        scheduled: false,
        started_by: Some(crate::ports::types::StartedBy::Agent("ceo".into())),
        resume_semantic: None,
    }))
    .expect("workflow_run_started reaches the console");
    assert_eq!(
        started_with_sender["startedBy"],
        serde_json::json!({"agent": "ceo"})
    );

    let node = super::project_event(&stored(CompanyEvent::WorkflowNodeFinished {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        node_id: "ceo".into(),
        status: crate::ports::types::WorkflowNodeStatus::Error,
        elapsed_ms: 1234,
        diagnostics: Vec::new(),
        agent_run_id: None,
    }))
    .expect("workflow_node_finished reaches the console");
    assert_eq!(node["type"], "workflow_node_finished");
    assert_eq!(node["runId"], "run-1");
    assert_eq!(node["nodeId"], "ceo");
    assert_eq!(node["status"], "error");
    assert_eq!(node["elapsedMs"], 1234);

    // The scrubbing claim, stated as a test: an errored node projects a
    // status word and NOTHING that could carry the node's own words. The
    // event type has no field to hold them, so this can only regress by
    // widening the event — which is the point of keeping it closed.
    let mut keys: Vec<&str> = node
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "atMillis",
            "elapsedMs",
            "nodeId",
            "runId",
            "seq",
            "status",
            "type",
            "workflowId",
        ],
        "the node frame carries only structural fields: {node}"
    );
}

/// Issue #382: the per-node START bracket reaches the console too. Without
/// its own arm it would fall to `project_event`'s `_ => return None` wildcard
/// and be silently dropped — the exact trap this file has been bitten by
/// three times — and the canvas would be back to guessing which node runs.
/// It carries the ids and NOTHING else: no status or duration (the node has
/// not run) and no input, so the frame is structural by construction.
#[test]
fn projects_the_per_node_started_bracket() {
    let node = super::project_event(&stored(CompanyEvent::WorkflowNodeStarted {
        workflow_id: "digest".into(),
        run_id: "run-1".into(),
        node_id: "ceo".into(),
    }))
    .expect("workflow_node_started reaches the console");
    assert_eq!(node["type"], "workflow_node_started");
    assert_eq!(node["workflowId"], "digest");
    assert_eq!(node["runId"], "run-1");
    assert_eq!(node["nodeId"], "ceo");

    // Structural-only: ids plus the envelope, and no status/duration/payload
    // slot the finish frame has. Regresses only by widening the event.
    let mut keys: Vec<&str> = node
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["atMillis", "nodeId", "runId", "seq", "type", "workflowId"],
        "the started frame carries only structural ids: {node}"
    );
}

/// Issue #371 also starts projecting the run id on the settle-frame — the
/// key that lets the console clear the right canvas when two runs overlap.
/// Still omitted for a pre-#371 row, so no permanently-null key appears.
#[test]
fn projects_the_run_id_on_a_finished_run_only_when_there_is_one() {
    let with_id = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: false,
        run_id: Some("run-9".into()),
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projected");
    assert_eq!(with_id["runId"], "run-9");

    let legacy = super::project_event(&stored(CompanyEvent::WorkflowRunFinished {
        workflow_id: "digest".into(),
        scheduled: false,
        run_id: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
    }))
    .expect("projected");
    assert!(legacy.get("runId").is_none(), "{legacy}");
}

#[test]
fn drops_non_attention_and_raw_payload_events() {
    // The operator's own message, and every variant that carries a raw
    // third-party payload or is audit-only, is dropped so nothing unexpected
    // (or secret-bearing) ever reaches the console.
    //
    // This list is unchanged by #228: adding `workflow_run_finished` to the
    // projection widened the wire by exactly one listed variant, and this
    // test passing untouched is what proves the deny-by-default default
    // still drops everything it dropped before.
    let dropped = [
        CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            parent: None,
            text: "hi".into(),
            by: None,
            chat: None,
            deliverable: None,
            attachments: Vec::new(),
        },
        CompanyEvent::WebhookReceived {
            channel: "email".into(),
            body: serde_json::json!({"authorization": "Bearer sk-secret"}),
        },
        CompanyEvent::A2aTaskReceived {
            from: "@peer".into(),
            task: serde_json::json!({"token": "sk-secret"}),
        },
        CompanyEvent::ScheduleFired {
            cron: "0 9 * * *".into(),
            prompt: "daily standup".into(),
        },
        CompanyEvent::FeedbackFiled {
            note: "too slow".into(),
        },
        CompanyEvent::MemoryFactDeleted {
            fact_id: "f-1".into(),
        },
    ];
    for event in dropped {
        assert!(
            super::project_event(&stored(event.clone())).is_none(),
            "event should be dropped from the SSE feed: {event:?}"
        );
    }
}

#[tokio::test]
async fn events_route_streams_text_event_stream() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/events")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The SSE head is returned immediately; the body streams indefinitely, so
    // we assert the status + content-type without draining it.
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
}

#[tokio::test]
async fn events_route_requires_a_session() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The composer's own typing pings must not echo back to it — the bus has
/// no per-listener addressing, so this filter is the only thing standing
/// between "you typed" and a fresh "Alice is typing…" line under your own
/// cursor.
#[test]
fn a_typing_frame_from_the_viewer_is_dropped_and_from_anybody_else_is_kept() {
    let mine = crate::turn_stream::LiveFrame::Typing(crate::turn_stream::TypingFrame {
        kind: "typing",
        user_id: "u1".into(),
        chat_id: "engineering".into(),
        parent_id: None,
        at_millis: 0,
    });
    assert!(super::is_own_typing_frame(&mine, Some("u1")));
    assert!(!super::is_own_typing_frame(&mine, Some("u2")));
    assert!(
        !super::is_own_typing_frame(&mine, None),
        "a machine credential with nobody behind it authors nothing to echo"
    );

    let presence = crate::turn_stream::LiveFrame::Presence(crate::turn_stream::PresenceFrame {
        kind: "presence",
        user_id: "u1".into(),
        status: "online",
        at_millis: 0,
    });
    assert!(
        !super::is_own_typing_frame(&presence, Some("u1")),
        "presence is left alone — only typing echoes"
    );
}

// -----------------------------------------------------------------------
// Standing permissions (issue #374)
// -----------------------------------------------------------------------

/// Every contradictory or unbounded scope request is a 400, and none of them
/// reaches the runtime.
///
/// The approval id is deliberately one that does not exist: each of these
/// must be refused at the edge, so the fact that resolving a missing
/// approval would otherwise be a harmless no-op never gets a chance to mask
/// a body that should not have been accepted.
///
/// A deny may now ride the tool scope (issue #1458 — a standing refusal),
/// so that pairing is asserted as *accepted* at the bottom rather than
/// listed among the refusals.
#[tokio::test]
async fn a_contradictory_or_unbounded_scope_is_refused() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let day: u64 = 24 * 60 * 60 * 1000;
    for (label, body) in [
        (
            "an argument edit and a standing grant contradict",
            format!(
                r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{day},"amended_payload":{{"to":"x"}}}}"#
            ),
        ),
        (
            "the deadline is mandatory",
            r#"{"verdict":"approve","scope":"tool"}"#.to_string(),
        ),
        (
            "zero is not a duration",
            r#"{"verdict":"approve","scope":"tool","expires_in_millis":0}"#.to_string(),
        ),
        (
            "past the seven-day cap is refused, never clamped",
            format!(
                r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{}}}"#,
                MAX_STANDING_GRANT_MILLIS + 1
            ),
        ),
        (
            "a duration is meaningless on the once scope",
            format!(r#"{{"verdict":"approve","scope":"once","expires_in_millis":{day}}}"#),
        ),
    ] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/company/approvals/appr-missing")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{label}: must be refused at the edge"
        );
    }

    // An unrecognised scope is refused too, one layer earlier: `ResolveScope`
    // is a closed enum, so axum's JSON extractor rejects it as 422 before
    // any handler runs. The status differs from the checks above; what
    // matters is that it is never silently downgraded to `once`, which would
    // hand an operator a single call when they asked for a standing one.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"verdict":"approve","scope":"forever"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Exactly at the cap is fine — the boundary is inclusive.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"verdict":"approve","scope":"tool","expires_in_millis":{MAX_STANDING_GRANT_MILLIS}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::BAD_REQUEST);

    // A deny riding the tool scope is no longer a contradiction: it mints a
    // standing refusal (issue #1458). Same edge validation as an approve —
    // duration mandatory, bounded, and the missing approval resolves as a
    // no-op — so it is accepted exactly where a matching approve would be.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"verdict":"deny","scope":"tool","expires_in_millis":{day}}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::BAD_REQUEST);
}

/// The default body — no `scope` key at all — is accepted exactly as before.
#[tokio::test]
async fn an_omitted_scope_is_the_pre_374_request() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/approvals/appr-missing")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"verdict":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// The grants list is empty on a fresh company, and revoking something that
/// is not there is a 404 rather than a cheerful no-op.
#[tokio::test]
async fn the_grants_list_starts_empty_and_revoking_nothing_is_a_404() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/grants")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 0);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/nope")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A standing grant is listed with the authenticated user's id, is revocable,
/// and revoking is idempotent-by-404. Both scope forms answer.
#[tokio::test]
async fn a_standing_grant_is_listed_under_its_granter_and_can_be_revoked() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    runtime
        .grants
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("g1"),
            agent: "ops".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Approve,
            granted_by: Actor {
                kind: ActorKind::User,
                id: "user-7".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: 1_000,
            expires_at_millis: crate::ports::now_millis() + 60 * 60 * 1000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });

    // Both addressing forms list it.
    for uri in ["/api/v1/company/grants", "/api/v1/companies/acme/grants"] {
        let response = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value[0]["id"], "g1", "{uri}");
        assert_eq!(value[0]["tool"], "workspace_write");
        assert_eq!(value[0]["agent"], "ops");
        assert_eq!(
            value[0]["granted_by"]["id"], "user-7",
            "the list names who actually granted it"
        );
        assert!(
            value[0].get("payload").is_none() && value[0].get("args").is_none(),
            "a standing grant has no arguments, so the list opens no redaction surface"
        );
    }

    // Revoke, then it is gone and a second revoke is a 404.
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g1")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(runtime.standing_grants().len(), 0);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g1")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// GRANT-012 (AUTH): `GET {scope}/grants` stays readable by any member —
/// the same consistency `GET {scope}/tools/grants` holds — but revoking one
/// is an admin action (issue #2169). A Member must see the list and be
/// refused the delete.
#[tokio::test]
async fn a_member_may_list_standing_grants_but_not_revoke_one() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    runtime
        .grants
        .grant_standing(crate::runtime::grants::StandingGrant {
            id: crate::runtime::grants::GrantId::new("g-member"),
            agent: "ops".into(),
            workflow: None,
            tool: "workspace_write".into(),
            verdict: Verdict::Approve,
            granted_by: Actor {
                kind: ActorKind::User,
                id: "user-7".into(),
            },
            approval_id: ApprovalId::new("appr-1"),
            at_millis: 1_000,
            expires_at_millis: crate::ports::now_millis() + 60 * 60 * 1000,
            origin_thread: None,
            origin_parent: None,
            origin_task: None,
            scope: None,
        });
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let member_cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/grants")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a member may read the standing-grants list"
    );
    let body = body_json(response).await;
    assert_eq!(body[0]["id"], "g-member");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g-member")
                .header("cookie", &member_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "revoking a standing grant is an admin action, matching the tools/grants plane"
    );
}

/// GRANT-012 (FAIL): `revoke_standing` is a plain map removal with no
/// expiry check of its own — a grant past its deadline that nothing has
/// *swept* yet is still found and revoked normally (204), exactly as
/// `/extend` can still rescue a not-yet-swept approval. Only once
/// `sweep_standing` has actually removed it does revoke correctly answer
/// the "nothing to revoke" 404 the route's own doc promises — the same
/// distinction as an already-revoked id, never a 500.
#[tokio::test]
async fn revoking_a_grant_is_404_only_once_it_is_actually_swept() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let stale_grant = |id: &str| crate::runtime::grants::StandingGrant {
        id: crate::runtime::grants::GrantId::new(id),
        agent: "ops".into(),
        workflow: None,
        tool: "workspace_write".into(),
        verdict: Verdict::Approve,
        granted_by: Actor {
            kind: ActorKind::User,
            id: "user-7".into(),
        },
        approval_id: ApprovalId::new("appr-1"),
        at_millis: 1_000,
        // Already in the past either way; only sweeping tells the two apart.
        expires_at_millis: 1_001,
        origin_thread: None,
        origin_parent: None,
        origin_task: None,
        scope: None,
    };
    runtime.grants.grant_standing(stale_grant("g-unswept"));
    runtime.grants.grant_standing(stale_grant("g-swept"));

    let app = router(state.clone());
    let delete = |id: &'static str| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/company/grants/{id}"))
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };

    // Past-deadline but not yet swept: still a normal, successful revoke.
    let response = delete("g-unswept").await;
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "an expired-but-unswept grant is still physically present, so revoking it is an \
         ordinary success — exactly as extend can still rescue an unswept approval"
    );

    // Now actually sweep the other one out from under the route.
    let swept = runtime.grants.sweep_standing(crate::ports::now_millis());
    assert_eq!(swept.len(), 1, "premise: the grant was in fact swept");

    let response = delete("g-swept").await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "once actually swept, revoke must report the same 'nothing to revoke' answer an \
         already-revoked id does"
    );
}

fn racing_standing_grant(id: &str) -> crate::runtime::grants::StandingGrant {
    crate::runtime::grants::StandingGrant {
        id: crate::runtime::grants::GrantId::new(id),
        agent: "ops".into(),
        workflow: None,
        tool: "workspace_write".into(),
        verdict: Verdict::Approve,
        granted_by: Actor {
            kind: ActorKind::User,
            id: "user-7".into(),
        },
        approval_id: ApprovalId::new("appr-1"),
        at_millis: 1_000,
        expires_at_millis: crate::ports::now_millis() + 60 * 60 * 1000,
        origin_thread: None,
        origin_parent: None,
        origin_task: None,
        scope: None,
    }
}

/// GRANT-012 (CONC). Two browsers — or one double-click — racing a
/// `DELETE` on the same grant id must not both report success:
/// `revoke_standing` is a plain `HashMap::remove`, so exactly one caller
/// takes the grant and every other must see the ordinary "already gone"
/// 404 a second revoke gets, not a duplicate 204 or a panic on a double
/// free.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_simultaneous_revokes_of_the_same_grant_settle_once() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    runtime
        .grants
        .grant_standing(racing_standing_grant("g-race"));

    let app = router(state);
    let racers: Vec<_> = (0..8)
        .map(|_| {
            let app = app.clone();
            tokio::spawn(async move {
                app.oneshot(
                    Request::builder()
                        .method("DELETE")
                        .uri("/api/v1/company/grants/g-race")
                        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            })
        })
        .collect();

    let mut statuses = Vec::new();
    for racer in racers {
        statuses.push(
            racer
                .await
                .expect("the request task did not panic")
                .status(),
        );
    }
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::NO_CONTENT)
            .count(),
        1,
        "exactly one simultaneous revoke may take the grant, got {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::NOT_FOUND)
            .count(),
        7,
        "every loser must see the ordinary already-gone 404, got {statuses:?}"
    );
    assert_eq!(runtime.grants.standing().len(), 0);
}

/// A [`JournalStore`](crate::ports::journal::JournalStore) that refuses
/// every `StandingGrantRevoked` line and passes everything else through.
/// Targets the **direct** `DELETE {scope}/grants/{gid}` append —
/// distinct from `runtime::cycle::test::FailStandingRevokeStore`, which
/// pins the mint/revoke *reconcile* path's own (oppositely ordered)
/// append.
struct RefusingGrantRevokeStore {
    inner: crate::ports::journal::MemoryJournalStore,
}

#[async_trait::async_trait]
impl crate::ports::journal::JournalStore for RefusingGrantRevokeStore {
    async fn append_journal(
        &self,
        id: &CompanyId,
        line: &str,
        durability: crate::ports::journal::Durability,
    ) -> crate::Result<()> {
        if line.contains("StandingGrantRevoked") {
            return Err(OpenCompanyError::Store(
                "RefusingGrantRevokeStore: the volume is full".to_string(),
            ));
        }
        self.inner.append_journal(id, line, durability).await
    }

    async fn read_journal(&self, id: &CompanyId) -> crate::Result<Vec<String>> {
        self.inner.read_journal(id).await
    }

    async fn journal_imported(&self, id: &CompanyId) -> crate::Result<bool> {
        self.inner.journal_imported(id).await
    }

    async fn complete_import(&self, id: &CompanyId, lines: Vec<String>) -> crate::Result<()> {
        self.inner.complete_import(id, lines).await
    }
}

/// GRANT-012 (FAIL). `revoke_standing_grant` takes the grant out of the
/// live set **before** its durable journal append — the opposite order
/// from minting, and on purpose (see the function's own doc): a crash
/// here must fail toward no-permission, never toward a permission nobody
/// can see is still live. When the append then fails, the caller is told
/// the revoke failed, but the grant must already be gone from the live
/// set that actually governs future calls.
#[tokio::test]
async fn a_failed_revoke_append_still_removes_the_grant_from_the_live_set() {
    let home_dir = home();
    let store = std::sync::Arc::new(RefusingGrantRevokeStore {
        inner: crate::ports::journal::MemoryJournalStore::default(),
    });
    let m = manifest();
    let id = CompanyId::new("acme");
    let fs_store = FsCompanyStore::new(home_dir.path().to_path_buf());
    {
        use crate::ports::store::CompanyStore;
        fs_store
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: m.clone(),
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
    }
    let runtime = RuntimeBuilder::new(home_dir.path().to_path_buf(), m)
        .with_id(id.clone())
        .with_journal_store(store)
        .build()
        .await
        .unwrap();
    let runtime = Arc::new(runtime);
    runtime
        .grants
        .grant_standing(racing_standing_grant("g-append-fail"));

    let state = AppState::new(AppConfig::default());
    state.registry().insert(id.clone(), runtime.clone());
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/company/grants/g-append-fail")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "the forced append failure must surface"
    );
    assert_eq!(
        runtime.grants.standing().len(),
        0,
        "the live-set removal must land even though the durable record of it failed — \
         fail toward no permission, never toward one nobody can see is still granted"
    );
}

// ---------------------------------------------------------------------
// Issue #469 — a turn that parks several approvals.
//
// Every test above parks exactly one, which is the case that always
// worked. The failure the operator hit needs more than one: four
// `composio_execute` calls from a single turn, all approved, and then
// silence. These drive that shape end to end over the real router.
// ---------------------------------------------------------------------

/// A brain that parks `parks` gated tool calls on one operator message and
/// answers each `ApprovalResolved` it is told about.
///
/// Deliberately shaped like `HarnessBrain`'s approval arm rather than like a
/// convenient stub: it consults the live grant set and produces **no reply
/// at all** when there is no grant left to redeem, because that silent
/// no-op is exactly what the later of several follow-up cycles used to hit.
struct MultiParkBrain {
    parks: usize,
    /// One entry per `ApprovalResolved` the brain was handed, across all
    /// cycles.
    decisions: Arc<std::sync::Mutex<Vec<String>>>,
    /// How many cycles ran in total (the first is the chat turn).
    cycles: Arc<std::sync::atomic::AtomicUsize>,
    /// The runtime, so the brain can reach the grant set the way the
    /// harness's re-dispatch does. Filled by the test after the build.
    rt: Arc<std::sync::OnceLock<Arc<CompanyRuntime>>>,
    /// Fail the continuation cycle, to exercise defect 4.
    fail_continuation: bool,
    /// Stamp a workflow run id onto every parked effect (issue #1092), so
    /// the park records the shape a workflow node's gated tool call has:
    /// explicitly unlinked from any card, and carrying a run.
    run_id: Option<String>,
    /// An `@mention` to append to every continuation reply. Exercises the
    /// durable half of a reply's mention: the re-issue's reply journaling
    /// must badge the person it names, same as the `/chat` path.
    continuation_mention: Option<String>,
}

#[async_trait::async_trait]
impl crate::ports::brain::Brain for MultiParkBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        self.cycles
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut responses = Vec::new();
        for event in &req.events {
            match event {
                CompanyEvent::OperatorMessage { .. } => {
                    // Deliberately uncatalogued slugs (issue #470): this
                    // brain exists to park a *number* of distinct calls and
                    // is indifferent to how any of them classify. They
                    // still land under the real action key, so each reaches
                    // the catalogue lookup and misses it, rather than
                    // carrying no action for the classifier to find.
                    for i in 0..self.parks {
                        let mut effect = gated_tool_call();
                        effect.payload =
                            crate::policy::test_support::composio_unclassified_args_numbered(i);
                        effect.run_id = self.run_id.clone();
                        host.park_effect(effect).await?;
                    }
                }
                CompanyEvent::ApprovalResolved { approval_id, .. } => {
                    if self.fail_continuation {
                        return Err(crate::error::OpenCompanyError::BackgroundTask(
                            "the continuation turn fell over".into(),
                        ));
                    }
                    self.decisions.lock().unwrap().push(approval_id.to_string());
                    let rt = self.rt.get().expect("the test wires the runtime");
                    let Some(grant) = rt.grants.peek(approval_id) else {
                        continue;
                    };
                    rt.grants.consume(&grant.agent, &grant.tool, &grant.args);
                    let mut text = format!("re-issued {approval_id}");
                    if let Some(mention) = &self.continuation_mention {
                        text.push(' ');
                        text.push_str(mention);
                    }
                    responses.push(crate::ports::types::OutboundMessage {
                        message_id: None,
                        task_id: None,
                        outputs: Vec::new(),
                        channel: grant.agent.clone(),
                        agent: None,
                        text,
                        steps: Vec::new(),
                        reply_to: None,
                        mentions: Vec::new(),
                    });
                }
                _ => {}
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses: responses,
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "multi-park cycle",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

/// A company whose next turn parks four sign-offs.
struct MultiParkCompany {
    app: axum::Router,
    runtime: Arc<CompanyRuntime>,
    approvals: Vec<ApprovalId>,
    decisions: Arc<std::sync::Mutex<Vec<String>>>,
    cycles: Arc<std::sync::atomic::AtomicUsize>,
}

async fn multi_park_company(
    home: &std::path::Path,
    parks: usize,
    chat: Option<&str>,
    fail_continuation: bool,
) -> MultiParkCompany {
    multi_park_company_run(home, parks, chat, fail_continuation, None, None).await
}

/// [`multi_park_company`], with the parked effects stamped as a workflow
/// run (issue #1092).
async fn multi_park_company_run(
    home: &std::path::Path,
    parks: usize,
    chat: Option<&str>,
    fail_continuation: bool,
    run_id: Option<&str>,
    continuation_mention: Option<&str>,
) -> MultiParkCompany {
    let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let cycles = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let rt_slot: Arc<std::sync::OnceLock<Arc<CompanyRuntime>>> =
        Arc::new(std::sync::OnceLock::new());
    let state = build_state_with_brain(
        home,
        "running",
        AppConfig::default(),
        Some(Arc::new(MultiParkBrain {
            parks,
            decisions: decisions.clone(),
            cycles: cycles.clone(),
            rt: rt_slot.clone(),
            fail_continuation,
            run_id: run_id.map(str::to_string),
            continuation_mention: continuation_mention.map(str::to_string),
        })),
    )
    .await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let _ = rt_slot.set(runtime.clone());
    let app = router(state);

    let body = match chat {
        Some(chat) => serde_json::json!({ "text": "do it", "chat": chat }),
        None => serde_json::json!({ "text": "do it" }),
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let approvals: Vec<_> = runtime
        .pending_approvals()
        .iter()
        .map(|a| a.id.clone())
        .collect();
    assert_eq!(approvals.len(), parks, "the turn parked {parks} sign-offs");

    MultiParkCompany {
        app,
        runtime,
        approvals,
        decisions,
        cycles,
    }
}

/// Every `AgentReply` in the log, as `chat_id|text` — what the console's
/// event stream projects as an `agent_reply` frame and what a transcript
/// reload rebuilds from. An empty list means the operator saw nothing.
async fn agent_replies(runtime: &Arc<CompanyRuntime>) -> Vec<String> {
    use crate::ports::types::EventSeq;
    runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::AgentReply { text, chat_id, .. } => Some(format!("{chat_id}|{text}")),
            _ => None,
        })
        .collect()
}

/// The authors of every journaled `AgentReply`, in order (issue #966).
///
/// Separate from [`agent_replies`] because that one folds the author away.
async fn agent_reply_authors(runtime: &Arc<CompanyRuntime>) -> Vec<String> {
    use crate::ports::types::EventSeq;
    runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::AgentReply { agent_id, .. } => Some(agent_id),
            _ => None,
        })
        .collect()
}

fn approve_detached(id: &ApprovalId) -> Request<Body> {
    resolve_request(id, serde_json::json!({"verdict":"approve","detach":true}))
}

/// Waits for the follow-up work a detached resolve spawned to settle:
/// first for the turn to unblock, then for its continuation to have
/// journaled `expected_replies` `AgentReply` rows.
///
/// # Condition, not clock (issue #1071)
///
/// The second half used to be `sleep(400ms)` with a comment admitting what
/// it was — "the continuation itself runs on a spawned task; let it finish".
/// `ContinuationQueue::waiting()` drops to zero when the turn is
/// **unblocked**, which is strictly earlier than when the continuation's
/// replies are **written**, so the gap had to be covered by something. A
/// fixed sleep covers it only on a machine fast enough that day: on a loaded
/// CI runner the assertions read the event log first and came back short —
/// `3` replies instead of `4`, or `[]` instead of `["ceo"]` — on branches
/// with nothing to do with this code.
///
/// Raising the sleep is the tempting fix and only moves the threshold. This
/// waits for the thing the caller is about to assert, the same way the first
/// half already waits for `waiting()`, under the same 10-second cap. A test
/// that is going to check for N replies has no reason to proceed before N
/// replies exist, and every reason not to.
///
/// The count is the caller's because only the caller knows it. Passing a
/// number smaller than the assertion would reintroduce the race quietly, so
/// pass exactly what is asserted.
async fn settle(runtime: &Arc<CompanyRuntime>, expected_replies: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while runtime.continuations.waiting() > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the turn never unblocked");
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while agent_replies(runtime).await.len() < expected_replies {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the continuation never journaled {expected_replies} replies"));
}

/// **The keystone (issue #469).** A turn that parks four sign-offs, all
/// approved, produces exactly ONE continuation — and an answer the operator
/// can actually see.
///
/// Before this, each resolve spawned its own follow-up cycle: four full
/// re-runs of one turn, each told about one decision. They did not race —
/// the per-company serial lock made them queue — but the later ones found
/// the grants the earlier ones had redeemed and produced nothing at all.
/// And none of it reached the operator either way, because the resolve
/// route never journaled a continuation's replies, so no `agent_reply`
/// frame was ever projected. Four approvals, four wasted turns, silence.
#[tokio::test]
async fn four_sign_offs_from_one_turn_produce_one_continuation() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 4, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    let mut handles = Vec::new();
    for id in &c.approvals {
        let app = c.app.clone();
        let request = approve_detached(id);
        handles.push(tokio::spawn(
            async move { app.oneshot(request).await.unwrap() },
        ));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap().status(), StatusCode::OK);
    }
    settle(&c.runtime, 4).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "one turn owes one continuation, not one per approval"
    );
    assert_eq!(
        c.decisions.lock().unwrap().len(),
        4,
        "the single continuation carries every decision, so the brain learns all four"
    );
    assert!(
        c.runtime.pending_approvals().is_empty(),
        "every sign-off was decided"
    );
    assert_eq!(
        agent_replies(&c.runtime).await.len(),
        4,
        "the continuation's answers must reach the event stream, or the operator \
         watches an approved action in silence"
    );
}

/// The two orders an operator can decide in must end in the same place.
///
/// Approving four at once and approving them one at a time are the same
/// request spread over a different span, and the gate is the last decision
/// rather than a time window — so neither can produce more continuations
/// than the other. A design that coalesced only what arrived together would
/// pass the test above and still re-run the turn four times here.
#[tokio::test]
async fn deciding_one_at_a_time_ends_where_deciding_all_at_once_does() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 4, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    for (i, id) in c.approvals.iter().enumerate() {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let ran = c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before;
        if i < 3 {
            assert_eq!(
                ran,
                0,
                "the turn is still blocked on {} more sign-off(s); continuing now \
                 would re-park them",
                3 - i
            );
        }
    }
    settle(&c.runtime, 4).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "the last decision unblocks the turn, and it runs once"
    );
    assert_eq!(c.decisions.lock().unwrap().len(), 4);
    assert_eq!(agent_replies(&c.runtime).await.len(), 4);
}

/// The continuation answers in the conversation the sign-off was raised in.
///
/// Not on the answering agent's own line: a desk channel's request and a
/// direct message to that channel's lead are answered by the same teammate,
/// so keying the reply on the agent delivers a channel's continuation into a
/// private thread nobody is watching (issue #379's lesson, which the reply
/// path had never learned — only the re-park had).
#[tokio::test]
async fn a_continuation_answers_in_the_thread_the_sign_off_was_raised_in() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 2, Some("sales"), false).await;

    for id in &c.approvals {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    settle(&c.runtime, 2).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(replies.len(), 2, "both re-issues answered");
    assert!(
        replies.iter().all(|r| r.starts_with("sales|")),
        "the continuation must land in the channel the approval was raised in, got {replies:?}"
    );
}

/// **Issue #1092.** A workflow node's parked call, once approved, answers
/// on its run — never as a direct message from the teammate that ran it.
///
/// This is the wiring test for `continuation_fallback_chat_id`: the unit
/// tests pin what the fallback *returns*, and this pins that
/// `publish_continuation` actually uses it, through a real park, a real
/// resolve and the journal the console reads back.
///
/// The assertion is written against the agent id rather than only for the
/// run id, because that is the regression: the leak put the re-issued
/// turn's narration into `chat/history?desk=<teammate>`, where it rendered
/// as an unprompted DM.
#[tokio::test]
async fn a_workflow_parks_continuation_answers_on_the_run_not_in_a_dm() {
    let home_dir = home();
    let c =
        multi_park_company_run(home_dir.path(), 1, None, false, Some("run-1092"), None).await;

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(replies.len(), 1, "the re-issue answered once");
    let (chat_id, _) = replies[0].split_once('|').expect("chat_id|text");
    assert_eq!(
        chat_id, "run-1092",
        "a workflow park's continuation belongs to its run, got {replies:?}"
    );
    // The regression, stated as itself: before this fix the fallback was
    // the answering teammate's own id, so this is what the leaked row held.
    assert_ne!(
        chat_id, "ceo",
        "the re-issue must not be journaled as a DM from the teammate that ran it"
    );
}

/// **Codex P1 (pass 2).** A continuation's reply is journaled through
/// `publish_continuation`, not the `/chat` turn — so a mention an agent
/// types back in an approval follow-up used to render as a chip and
/// nothing else: no badge, no durable row, exactly the person it is meant
/// to reach (offline when the reply lands) getting neither.
///
/// Both paths file through the same writer now; this pins that an `@user`
/// in a continuation reply lands as a mention notification whose audience
/// carries the person named, under the chat the continuation answered in.
#[tokio::test]
async fn a_continuation_reply_that_mentions_a_user_files_a_notification() {
    let home_dir = home();
    let c = multi_park_company_run(
        home_dir.path(),
        1,
        Some("sales"),
        false,
        None,
        Some("@harness-admin"),
    )
    .await;

    let users = c
        .runtime
        .users()
        .list_users(&CompanyId::new("acme"))
        .await
        .unwrap();
    let admin = users
        .iter()
        .find(|u| u.email == "harness-admin@example.test")
        .expect("the fixed admin is seeded");
    assert_eq!(
        admin.status,
        crate::ports::users::UserStatus::Active,
        "the admin must be an active, mentionable target"
    );

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;
    // The notification is filed inside `publish_continuation`, after the
    // reply is journaled — `settle` only waits for the reply. A loaded CI
    // runner can reach this point before the notification append finishes,
    // so poll for it (issue #1665, Codex P1 regression).
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = c
                .runtime
                .notifications()
                .list(&CompanyId::new("acme"), &admin.id)
                .await
                .unwrap();
            if notes.iter().any(|n| n.notification.kind == "mention") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the mention notification never appeared");

    let notes = c
        .runtime
        .notifications()
        .list(&CompanyId::new("acme"), &admin.id)
        .await
        .unwrap();
    let mentions: Vec<_> = notes
        .into_iter()
        .filter(|n| n.notification.kind == "mention")
        .collect();
    assert_eq!(
        mentions.len(),
        1,
        "the continuation's mention must badge the person it names"
    );
    let note = &mentions[0].notification;
    assert_eq!(note.context.as_deref(), Some("sales"));
    assert_eq!(
        note.title, "Someone mentioned you in sales",
        "a continuation has no author, so the generic label is the honest one"
    );
    assert!(
        note.audience
            .as_ref()
            .is_some_and(|a| a.contains(&admin.id)),
        "the named user must be in the notification's audience"
    );
}

/// **Issue #379's routing, re-homed (issue #469).** The continuation
/// resumes in the thread the sign-off was raised in — and in no other.
///
/// Asserted in **both directions**, because either alone would pass on a
/// mistake. A desk channel's request and a direct message to that channel's
/// lead are answered by the same teammate, so a reply keyed on the agent
/// lands a channel's continuation in a private line nobody is watching, and
/// a reply keyed on the channel does the reverse.
///
/// This used to be pinned inside the harness brain, against a hand-built
/// grant. It moved here with the journaling: the thread comes off the park
/// record now, so the strong version of the test is the one that lets a real
/// turn stamp it and a real resolve read it back.
#[tokio::test]
async fn a_continuation_resumes_in_the_thread_it_was_raised_in_and_no_other() {
    async fn threads_for(chat: &str) -> Vec<String> {
        let home_dir = home();
        let c = multi_park_company(home_dir.path(), 1, Some(chat), false).await;
        let response = c
            .app
            .clone()
            .oneshot(approve_detached(&c.approvals[0]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        settle(&c.runtime, 1).await;
        agent_replies(&c.runtime)
            .await
            .into_iter()
            .map(|r| r.split('|').next().unwrap().to_string())
            .collect()
    }

    // Raised in a desk channel: the continuation belongs to the channel.
    let desk = threads_for("desk-finance").await;
    assert_eq!(desk, vec!["desk-finance".to_string()]);
    assert_ne!(
        desk[0], "ceo",
        "a channel's approval must not resume in the desk lead's private DM"
    );

    // Raised in a direct message with that same lead: the mirror image.
    let dm = threads_for("ceo").await;
    assert_eq!(dm, vec!["ceo".to_string()]);
    assert_ne!(
        dm[0], "desk-finance",
        "a private line's approval must not resume in the desk channel"
    );
}

/// A single-approval turn is unchanged: it continues on that one decision,
/// exactly as it did before the gate existed.
#[tokio::test]
async fn a_lone_sign_off_still_continues_on_its_own_decision() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 1, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1
    );
    assert_eq!(agent_replies(&c.runtime).await.len(), 1);
}

/// **Defect 4.** A continuation that fails tells the person waiting for it.
///
/// The verdict and the grant are already durable at this point, so the
/// failure is recoverable — but only for somebody who knows it happened.
/// Before this the entire report was one `tracing::error!`: the agent was
/// not told the outcome, and neither was the operator, who saw an approval
/// they had granted produce nothing and had no way to tell a slow turn from
/// a dead one.
#[tokio::test]
async fn a_failed_continuation_tells_the_operator() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 2, Some("sales"), true).await;

    for id in &c.approvals {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the verdict is durable regardless of what the turn then does"
        );
    }
    settle(&c.runtime, 1).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(
        replies.len(),
        1,
        "the operator is told exactly once that the work did not resume, got {replies:?}"
    );
    assert!(
        replies[0].starts_with("sales|"),
        "and told in the thread they approved in, got {replies:?}"
    );
    assert!(
        replies[0].contains("approving again is safe"),
        "the notice has to say what to do about it, got {replies:?}"
    );
    // Issue #966, asserted on the journaled row rather than on the
    // constructor: this drives the real approve path, so it pins that
    // `announce_continuation_failure` *calls* the named notice. Asserting
    // the constructor alone leaves the call site free to go back to an
    // inline `AgentReply` authored by the operator channel — a correct
    // system row byte-identical to one the pre-#885 defect damaged.
    let authors = agent_reply_authors(&c.runtime).await;
    assert_eq!(
        authors,
        vec![crate::ports::SYSTEM_AUTHOR.to_string()],
        "the runtime authored this notice, so it must not be stored under its destination"
    );
}

/// Codex review finding: a stream that errors mid-read used to fall
/// straight through to extraction on whatever partial bytes it had
/// collected. This pins the fix directly against a synthetic stream,
/// without needing a real workspace store behind it — a chunk, then an
/// error, must discard everything read so far rather than handing back
/// a truncated payload that looks complete.
#[tokio::test]
async fn drain_bounded_discards_everything_on_a_mid_stream_error() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> = vec![
        Ok(Bytes::from_static(b"the first chunk read fine")),
        Err(crate::error::OpenCompanyError::Store(
            "transient read failure".to_string(),
        )),
    ];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(drain_bounded(synthetic, 1_000_000).await, None);
}

/// The success twin: a stream with no error drains to its bytes, in
/// order, across however many chunks it arrives in.
#[tokio::test]
async fn drain_bounded_concatenates_every_chunk_when_the_stream_never_errors() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> = vec![
        Ok(Bytes::from_static(b"hello ")),
        Ok(Bytes::from_static(b"world")),
    ];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(
        drain_bounded(synthetic, 1_000_000).await,
        Some(b"hello world".to_vec())
    );
}

/// A stream that never errors but exceeds the cap is also discarded, not
/// truncated — the belt-and-braces the doc comment describes.
#[tokio::test]
async fn drain_bounded_discards_when_the_stream_exceeds_the_cap() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> =
        vec![Ok(Bytes::from_static(b"way more than the cap allows"))];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(drain_bounded(synthetic, 4).await, None);
}

/// A brain whose every reply names `@everyone` — the fixed shape for
/// proving an agent reply's mentions file a notification, same as an
/// operator message's already does.
struct MentioningReplyBrain;

#[async_trait::async_trait]
impl crate::ports::brain::Brain for MentioningReplyBrain {
    async fn run_cycle(
        &self,
        req: crate::ports::types::CycleRequest,
        _host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        let mut channel_responses = Vec::new();
        for event in &req.events {
            if matches!(event, CompanyEvent::OperatorMessage { .. }) {
                channel_responses.push(crate::ports::types::OutboundMessage {
                    message_id: None,
                    task_id: None,
                    outputs: Vec::new(),
                    channel: "operator".into(),
                    agent: None,
                    text: "cc @everyone on this".into(),
                    steps: Vec::new(),
                    reply_to: None,
                    mentions: Vec::new(),
                });
            }
        }
        Ok(crate::ports::types::CycleResult {
            channel_responses,
            new_traces: vec![crate::ports::types::CompressedTrace::now(
                &req.cycle_id,
                "mentioning reply",
            )],
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }
}

/// **The Codex P1 finding:** `journal_chat_replies` resolved an agent
/// reply's mentions and stored them on `CompanyEvent::AgentReply`, but never
/// called `notify_mentions` — so an `@user` an agent typed *back* rendered
/// as a chip and left the named person with no durable notification and no
/// rail badge, unlike the operator's own message a few lines above it in
/// the very same function. Missing it worst for exactly the person it is
/// meant to reach: offline when the reply lands.
#[tokio::test]
async fn a_mention_in_an_agent_reply_notifies_the_person_it_names() {
    let home_dir = home();
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(Arc::new(MentioningReplyBrain)),
    )
    .await;
    // A second person for `@everyone` to reach — the sender is always
    // excluded from their own broadcast, so proving this needs somebody
    // else on the roster.
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let app = router(state.clone());
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    let member_id = runtime
        .users()
        .list_users(&id)
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.email == "harness-member@example.test")
        .expect("seeded member")
        .id;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"status?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let notified = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = runtime.notifications().list(&id, &member_id).await.unwrap();
            if !notes.is_empty() {
                return notes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the mentioned member was notified");

    assert_eq!(notified.len(), 1);
    assert_eq!(
        notified[0].notification.kind, "mention",
        "the reply's @everyone mention has to file the same kind of row an \
         operator message's does"
    );
}

/// **The Codex P1 finding:** the context a DM mention stores was decided by
/// the human user directory, but a DM's thread id is a roster teammate's
/// agent id — which no user record has — so a mention in a normal DM stored
/// the bare id. The console's rail keys a DM by `dm:<teammate-id>` (and the
/// console sends that bare id as the `chat` for a DM), so no rail row
/// displayed the badge and opening the DM could neither match nor clear it.
#[tokio::test]
async fn a_mention_in_a_dm_stores_the_console_dm_channel_id() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    // A second person for the broadcast to reach — the author is always
    // excluded from their own `@everyone`.
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let app = router(state.clone());
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    let member_id = runtime
        .users()
        .list_users(&id)
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.email == "harness-member@example.test")
        .expect("seeded member")
        .id;

    // A message addressed to the `designer` DM thread — the bare roster
    // teammate id, exactly what the console sends for a DM.
    let response = app
        .clone()
        .oneshot(chat_to("cc @everyone on this", Some("designer")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let notified = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = runtime.notifications().list(&id, &member_id).await.unwrap();
            if !notes.is_empty() {
                return notes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the mentioned member was notified");

    // The offline echo brain answers the same text, so `@everyone` may land
    // twice — once for the operator's message, once for the echoed reply.
    // The count is incidental; the invariant is that *every* mention filed
    // out of this exchange is keyed to the console's `dm:designer` channel,
    // not the bare roster thread id.
    assert!(!notified.is_empty(), "the mentioned member was notified");
    let contexts: Vec<_> = notified
        .iter()
        .map(|n| n.notification.context.as_deref())
        .collect();
    assert!(
        contexts.iter().all(|c| *c == Some("dm:designer")),
        "every mention in a DM has to store the console's DM channel id, \
         not the bare roster thread id — got {contexts:?}"
    );
}

/// [`mention_context`] canonicalizes a **`dm:`-prefixed** noncanonical key
/// too. An API client can address a DM with the console's channel shape but
/// a noncanonical payload — `dm:BACKEND_ENGINEER` for the teammate whose id
/// is `backend_engineer`. The routing resolves that case-insensitively, so
/// the stored context has to carry the canonical agent id: filing the raw
/// key under `dm:BACKEND_ENGINEER` badges a rail channel that does not
/// exist, and opening the actual DM can never clear it. Pre-fix, the
/// `dm:`-prefixed branch returned the key verbatim and bypassed
/// `assignee::resolve` entirely.
#[tokio::test]
async fn mention_context_canonicalizes_prefixed_dm_keys() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A case-variant of the teammate's id, carrying the `dm:` prefix the
    // console mints.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:BACKEND_ENGINEER")
            .await,
        "dm:backend_engineer",
        "a `dm:`-prefixed noncanonical teammate key has to store dm:<agent-id>"
    );
    // The already-canonical shape stays unchanged — the resolution must
    // not move a key that was already right.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a canonical dm:<teammate-id> key is kept as-is"
    );
    // A `dm:` key whose bare half names a desk (the desk-first ordering the
    // routing uses) files under the desk id, not a nonexistent `dm:<desk>`.
    assert_eq!(
        runtime.mention_context(&id, &[], "dm:Engineering").await,
        "engineering",
        "a `dm:` key that resolves to a desk has to store the desk id"
    );
}

/// A desk id that collides with a **human user id** still files under the
/// desk. `assignee::resolve`'s desk-first ordering — the same one
/// `responder_for` uses — outranks the user directory, and the directory
/// must not get a say ahead of it. Pre-fix, a `users` pre-check ran before
/// the resolution and returned `dm:<id>` for any bare key matching a human,
/// so a mention aimed at a desk whose id happened to match a human id would
/// badge a nonexistent DM channel and could never be cleared from the desk
/// it was meant for.
#[tokio::test]
async fn mention_context_a_human_id_matching_a_desk_id_stays_a_desk() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A human whose id collides with the `engineering` desk's id. The human
    // directory must not win: the message is aimed at the desk.
    let human = crate::ports::users::UserRecord {
        id: "engineering".to_string(),
        email: "human@example.test".to_string(),
        display_name: None,
        avatar: None,
        role: crate::ports::users::UserRole::Member,
        status: crate::ports::users::UserStatus::Active,
        password_hash: None,
        must_change_password: false,
        created_at_millis: crate::ports::now_millis(),
        last_seen_at_millis: None,
        updated_at_millis: crate::ports::now_millis(),
    };

    assert_eq!(
        runtime
            .mention_context(&id, std::slice::from_ref(&human), "engineering")
            .await,
        "engineering",
        "a desk id that matches a human id files under the desk, not dm:<id>"
    );
    assert_eq!(
        runtime
            .mention_context(&id, std::slice::from_ref(&human), "dm:engineering")
            .await,
        "engineering",
        "the same collision through a dm:-prefixed key still files under the desk"
    );
    // A DM the human is actually a teammate of still badges as a DM.
    assert_eq!(
        runtime
            .mention_context(&id, &[human], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a real DM channel is unaffected by the collision guard"
    );
}

/// [`mention_context`] resolves a `dm:`-prefixed key **as sent** before
/// stripping the prefix, so a desk literally named `dm:engineering` keeps
/// that id. Pre-fix, the unconditional strip resolved `engineering` instead
/// and filed the badge under the wrong transcript — the exact claim
/// [`assignee::dm_key`]'s contract warns about.
#[tokio::test]
async fn mention_context_a_desk_literally_named_dm_prefix_keeps_its_id() {
    let home_dir = home();
    let state = state_with_dm_prefixed_desk(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // The literal `dm:engineering` desk resolves as sent; stripping would
    // misroute to the plain `engineering` desk.
    assert_eq!(
        runtime.mention_context(&id, &[], "dm:engineering").await,
        "dm:engineering",
        "a desk literally named dm:<…> keeps its id — the raw key resolves first"
    );
    // The un-prefixed desk is untouched by the collision.
    assert_eq!(
        runtime.mention_context(&id, &[], "engineering").await,
        "engineering",
        "the un-prefixed desk still resolves to its own id"
    );
    // A genuine DM still re-keys onto the rail's DM channel.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a real DM channel is unaffected by the literal dm: desk"
    );
}

/// [`mention_context`] stores the **canonical** id for a key typed in a
/// noncanonical shape — a desk by its display name, a teammate by a
/// case-variant of their id. `assignee::resolve` already returns canonical
/// ids (issue #214); storing the raw key instead would file the badge under
/// a channel id the rail never has, so it could neither render nor clear.
#[tokio::test]
async fn mention_context_stores_canonical_ids_for_noncanonical_keys() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A desk addressed by its display name files under the desk's id —
    // `"Engineering"` names the desk whose id is `engineering`.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a desk named by its display name has to store the desk id, not the raw key"
    );
    // A teammate addressed by a case-variant of their id files under the
    // canonical agent id, re-keyed into the console's DM channel space.
    assert_eq!(
        runtime.mention_context(&id, &[], "BACKEND_ENGINEER").await,
        "dm:backend_engineer",
        "a teammate named by a noncanonical key has to store dm:<agent-id>"
    );
}

/// [`mention_context`] files a mention in the General desk — the default an
/// unaddressed message lands in — under the console's canonical main-thread
/// id even when this company has no desk named/id `General`. This fixture's
/// only desk is `engineering`, so every general-chat spelling would
/// otherwise fall through to the raw string and badge a rail row that does
/// not exist (issue #1665 follow-up).
#[tokio::test]
async fn mention_context_maps_unresolvable_general_spellings_to_main() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    for general in ["General", "general", "main", ""] {
        assert_eq!(
            runtime.mention_context(&id, &[], general).await,
            crate::server::chat_history::MAIN_THREAD_ID,
            "a mention in the General desk ({general:?}) has to store the console's \
             main-thread id, which the rail aliases onto its first rendered desk \
             channel"
        );
    }
    // A desk that does resolve keeps its canonical id — the general-chat
    // mapping must not swallow a real desk.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a real desk keeps its canonical id even when its name looks general"
    );
}

/// [`mention_context`] canonicalizes a **memberless** desk too. A desk that
/// exists but has nobody seated on it is still a real desk with a real rail
/// channel, so a key typed as its display name must file under its canonical
/// id: `"Sales"` has to badge `#sales`, and opening `#sales` has to clear it.
/// Pre-fix, `EmptyDesk` fell through the same wildcard as `Unknown` and
/// stored the raw key — a channel id no desk renders, so the badge was
/// invisible and could never clear.
#[tokio::test]
async fn mention_context_canonicalizes_a_memberless_desk() {
    let home_dir = home();
    let state = state_with_memberless_desk(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    assert_eq!(
        runtime.mention_context(&id, &[], "Sales").await,
        "sales",
        "a memberless desk named by its display name has to store the desk id, \
         not the raw key — the rail's channel id is `sales`"
    );
    // The desk that does have a lead keeps behaving as before.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a desk with a lead still stores its canonical id"
    );
}

/// Issue #1781 review (Codex P1): [`company_events`]'s periodic refresh
/// must re-derive admin access from the live user record, not keep
/// answering with whatever it was when the SSE stream opened. Proven
/// directly against [`refreshed_is_admin`] — the seam that refresh loop
/// calls on every tick — rather than the SSE handler itself, since the
/// handler's own timing (a real `EventSource`, a 60s interval) is not
/// what this bug is about.
#[tokio::test]
async fn refreshed_is_admin_reflects_a_mid_stream_demotion() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut user = crate::ports::users::UserRecord {
        id: "u1".to_string(),
        email: "admin@acme.test".to_string(),
        display_name: None,
        avatar: None,
        role: crate::ports::users::UserRole::Admin,
        status: crate::ports::users::UserStatus::Active,
        password_hash: None,
        must_change_password: false,
        created_at_millis: crate::ports::now_millis(),
        last_seen_at_millis: None,
        updated_at_millis: crate::ports::now_millis(),
    };
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();
    let actor = Actor {
        kind: ActorKind::User,
        id: user.id.clone(),
    };

    assert!(
        refreshed_is_admin(&runtime, Some(&actor), false).await,
        "an active admin's record must resolve to admin, even starting from a stale `false`"
    );

    // The demotion itself: same shape `PATCH …/users/{id}` writes, and —
    // critically — it does not touch sessions, so a connection opened
    // before this write stays open exactly as it would in production.
    user.role = crate::ports::users::UserRole::Member;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a demoted user's live record must flip a stale `true` to `false` — this is \
         exactly the check `company_events` failed to make before this fix, leaking the \
         owner-fallback admin-only report to a demoted viewer for the rest of their stream"
    );

    // Suspension revokes admin the same way, even if role were untouched.
    user.role = crate::ports::users::UserRole::Admin;
    user.status = crate::ports::users::UserStatus::Suspended;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a suspended admin must not keep admin-only visibility either"
    );
}

/// Issue #1781 review, Codex P1 second follow-up: a human actor whose
/// current role cannot be confirmed — `Ok(None)` because the user record
/// has gone missing, folded in here with a genuine store error since both
/// hit the same match arm — must resolve to `false`, not `previous`.
///
/// `previous: true` here stands in for exactly the dangerous case: a
/// cached "was admin" value from before whatever made this actor
/// unconfirmable, revalidated at the one call site
/// (`is_admin_for_item`) that gates the admin-only owner-fallback report
/// on this result directly. Before this fix, an actor deleted out from
/// under an open SSE stream — or a transient read failure landing at the
/// exact moment a report needed gating — fell back to `previous` and kept
/// leaking the report, silently, for as long as the failure (or the
/// missing record) persisted.
#[tokio::test]
async fn refreshed_is_admin_fails_closed_when_the_user_record_cannot_be_found() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    // Never upserted — `get_user` answers `Ok(None)`, the "record has
    // gone missing" half of the case this proves.
    let actor = Actor {
        kind: ActorKind::User,
        id: "ghost".to_string(),
    };

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a human actor with no resolvable user record must read as not-admin \
         even when the cached value being revalidated was `true` — trusting \
         `previous` here is exactly the fail-open gap this fix closes"
    );
}

/// Issue #1781 review, Codex P1 follow-up: even with the periodic refresh
/// the test above covers, `company_events` still only re-checked on its
/// own `LABEL_REFRESH_EVERY` (60s) tick — a demotion landing right after
/// one tick left an open SSE stream projecting an owner-fallback report
/// under a stale cached `true` for up to another 60s. `is_admin_for_item`
/// is the fix: it revalidates fresh for that one content class instead of
/// trusting `cached`, no matter how long ago the last periodic tick was —
/// proven here by feeding it a `cached: true` that is already wrong the
/// instant this call happens, with no `sleep` at all.
///
/// The second half is the other side of the same fix: an *ordinary* event
/// must keep using `cached` untouched, or every SSE item would pay a
/// store read regardless of content — the whole reason the fix is scoped
/// to the owner-fallback content class rather than revalidating every
/// item.
#[tokio::test]
async fn is_admin_for_item_revalidates_only_the_owner_fallback_report() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut user = crate::ports::users::UserRecord {
        id: "u1".to_string(),
        email: "admin@acme.test".to_string(),
        display_name: None,
        avatar: None,
        role: crate::ports::users::UserRole::Admin,
        status: crate::ports::users::UserStatus::Active,
        password_hash: None,
        must_change_password: false,
        created_at_millis: crate::ports::now_millis(),
        last_seen_at_millis: None,
        updated_at_millis: crate::ports::now_millis(),
    };
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();
    let actor = Actor {
        kind: ActorKind::User,
        id: user.id.clone(),
    };

    // The demotion: no wait, no periodic tick — the very next item must
    // already see it for the gated content class.
    user.role = crate::ports::users::UserRole::Member;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    let owner_fallback_item = EventStreamItem::Event(stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "operator".into(),
        agent_id: crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR.to_string(),
        text: "no admin has a mailbox".into(),
        steps: Vec::new(),
    }));
    assert!(
        !super::is_admin_for_item(&owner_fallback_item, &runtime, Some(&actor), true).await,
        "an owner-fallback report must revalidate fresh and see the demotion \
         immediately — a stale cached `true` must never leak this content, \
         regardless of when the last periodic refresh ran"
    );

    let ordinary_item = EventStreamItem::Event(stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "ordinary reply".into(),
        steps: Vec::new(),
    }));
    assert!(
        super::is_admin_for_item(&ordinary_item, &runtime, Some(&actor), true).await,
        "an ordinary event must keep using the cached snapshot untouched — \
         revalidating every item, not just the gated content class, would \
         add a store read to the hot path for no reason"
    );
}

/// The machine principal has no user record to look up — `actor: None` —
/// and [`ScopedCompany::is_admin`]'s own doc says it is unrestricted by
/// construction, so the refresh must leave it alone rather than treating
/// a missing actor as "look up nothing, therefore not admin".
#[tokio::test]
async fn refreshed_is_admin_leaves_the_machine_principal_unchanged() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    assert!(refreshed_is_admin(&runtime, None, true).await);
    assert!(!refreshed_is_admin(&runtime, None, false).await);
}

/// Two cards can be `in_review` on the same desk at once. Approving the
/// pill the operator actually clicked must move that card and leave the
/// other alone — resolving the desk's most-recently-updated card instead
/// (Codex #3903031183) moves the wrong one whenever the older pill is
/// clicked after a newer card has settled.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_settles_the_clicked_task_not_the_desks_latest() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    for (task_id, updated_at_millis) in [("t-old", 1u64), ("t-new", 2u64)] {
        runtime
            .tasks()
            .upsert(
                runtime.id(),
                &crate::ports::tasks::TaskRecord {
                    id: task_id.to_string(),
                    title: TaskTitle::authored("Ship it"),
                    note: None,
                    column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                    priority: "medium".to_string(),
                    assignee: "ceo".to_string(),
                    updated_at_millis,
                    origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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
    }

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let receipt = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-old".to_string(),
            decision: "approve".to_string(),
            note: None,
        }),
    )
    .await
    .expect("the clicked card is settled")
    .0;
    assert_eq!(receipt.task_id, "t-old");
    assert_eq!(receipt.column, crate::ports::tasks::COLUMN_DONE);

    let cards = runtime.tasks().list(runtime.id()).await.unwrap();
    let old = cards.iter().find(|t| t.id == "t-old").unwrap();
    let new = cards.iter().find(|t| t.id == "t-new").unwrap();
    assert_eq!(
        old.column,
        crate::ports::tasks::COLUMN_DONE,
        "the clicked pill's card must settle"
    );
    assert_eq!(
        new.column,
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "the desk's newer card must be untouched by a verdict on the older pill"
    );
}

/// A `task_id` naming a card outside the reviewed desk (or one that has
/// already left `in_review`) must not resolve to some other card in the
/// conversation — the request is rejected rather than silently falling
/// back to "whatever is in review here".
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_rejects_a_task_id_not_in_review_on_this_desk() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-review".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let err = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "does-not-exist".to_string(),
            decision: "approve".to_string(),
            note: None,
        }),
    )
    .await
    .expect_err("an unknown task id must not fall back to the desk's own card");
    assert_eq!(
        axum::response::IntoResponse::into_response(err).status(),
        StatusCode::NOT_FOUND
    );
}

/// `apply_review_decision`'s `Revise` arm through the HTTP handler: the
/// card re-enters `in_progress` with the operator's note appended, rather
/// than settling to `done` the way `Approve` does.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_revise_re_enters_in_progress_with_the_note() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: Some("[writer] first draft".to_string()),
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let receipt = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-1".to_string(),
            decision: "revise".to_string(),
            note: Some("tighten the intro".to_string()),
        }),
    )
    .await
    .expect("revise applies")
    .0;
    assert_eq!(receipt.task_id, "t-1");
    assert_eq!(receipt.column, crate::ports::tasks::COLUMN_IN_PROGRESS);

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    let note = after.note.expect("note");
    assert!(note.contains("tighten the intro"), "{note}");
}

/// A thread reply intercepted as review feedback re-dispatches its card
/// instead of answering with `responses` here. Codex #3903907771:
/// `ChatView.send` reads an empty `responses` as "the turn produced
/// nothing" and renders a synthetic "(no reply)" bubble underneath the
/// operator's own feedback, even though the card was re-dispatched and
/// will answer through its later relay. `reviewFeedbackApplied` is what
/// tells the console this empty `responses` is expected.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn thread_reply_review_feedback_marks_the_response_not_empty_handed() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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

    runtime
        .events()
        .append(
            runtime.id(),
            crate::ports::types::CompanyEvent::DeskTaskCompleted {
                task_id: "t-1".to_string(),
                desk: "ceo".to_string(),
                output: "done".to_string(),
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                artifact_ids: Vec::new(),
                origin_chat_id: Some("strategy".to_string()),
                origin_parent: None,
            },
        )
        .await
        .unwrap();
    let relay_seq = runtime
        .events()
        .append(
            runtime.id(),
            crate::ports::types::CompanyEvent::AgentReply {
                audience: Vec::new(),
                chat_id: "strategy".to_string(),
                agent_id: "ceo".to_string(),
                text: "Here is the draft.".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
            },
        )
        .await
        .unwrap();

    let message = ChatMessage {
        text: "needs another pass".to_string(),
        chat: Some("strategy".to_string()),
        parent: Some(relay_seq.value().to_string()),
        deliverable: None,
        detach: false,
        mentions: None,
        attachments: Vec::new(),
    };

    let outcome = chat_and_emit(&state, &id, runtime.clone(), message, None)
        .await
        .expect("review feedback applies");
    let ChatOk::Settled(body) = outcome else {
        panic!("a synchronous review-feedback intercept must not detach");
    };
    assert!(body.responses.is_empty());
    assert_eq!(
        body.review_feedback_applied,
        Some(true),
        "an empty `responses` here must be marked expected, not read as \
         a silent turn"
    );

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    assert_eq!(
        after.column,
        crate::ports::tasks::COLUMN_IN_PROGRESS,
        "the reply still re-dispatches the card"
    );
}

/// An unrecognized `decision` string rejects with `InvalidRequest` (400)
/// rather than falling through to either verdict.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_rejects_an_unknown_decision() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let err = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-1".to_string(),
            decision: "yeet".to_string(),
            note: None,
        }),
    )
    .await
    .expect_err("an unknown decision string must not settle the card");
    assert_eq!(
        axum::response::IntoResponse::into_response(err).status(),
        StatusCode::BAD_REQUEST
    );
}

/// `POST {scope}/chat/review` end to end through the real router: proves
/// the route is actually mounted by [`with_review_routes`] (not just that
/// the handler function works when called directly) and that the wire
/// body deserializes and settles the card via HTTP.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn chat_review_route_is_mounted_and_settles_via_http() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
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

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat/review")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "chatId": "strategy",
                        "taskId": "t-1",
                        "decision": "approve",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["taskId"], "t-1");
    assert_eq!(value["column"], "done");
}

/// No card is `in_review` on the desk at all — as opposed to a `taskId`
/// naming the wrong card, covered above — must also 404, through the same
/// HTTP path the console calls.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn chat_review_route_404s_when_no_card_is_in_review() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat/review")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "chatId": "strategy",
                        "taskId": "t-1",
                        "decision": "approve",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[cfg(feature = "openhuman")]
fn card_in_review(id: &str, chat_id: &str) -> crate::ports::tasks::TaskRecord {
    crate::ports::tasks::TaskRecord {
        id: id.to_string(),
        title: TaskTitle::authored("Ship it"),
        note: None,
        column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
        priority: "medium".to_string(),
        assignee: "ceo".to_string(),
        updated_at_millis: 1,
        origin: crate::ports::TaskOrigin::new(Some(chat_id.to_string()), None),
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

/// Two review verdicts racing the same `in_review` card (PR #1981 review
/// finding, Codex P1) must not both resolve it before either applies —
/// same `task_writes`-serialized load-modify-save shape
/// `add_desk_member_serializes_against_the_company_write_lock` proves
/// above, applied to `review_card`.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_serializes_against_the_task_writes_lock() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(runtime.id(), &card_in_review("t-1", "strategy"))
        .await
        .unwrap();

    let guard = runtime.task_writes.lock().await;

    let runtime_for_task = runtime.clone();
    let mut task = tokio::spawn(async move {
        let scope = ScopedCompany {
            runtime: runtime_for_task,
            actor: None,
            may_read_contents: true,
            is_admin: true,
        };
        review_card(
            scope,
            Json(ChatReviewRequest {
                chat_id: "strategy".to_string(),
                task_id: "t-1".to_string(),
                decision: "approve".to_string(),
                note: None,
            }),
        )
        .await
    });

    let raced_ahead = tokio::time::timeout(Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "review_card resolved and applied a verdict while task_writes was \
         held elsewhere — it is not serializing against concurrent board \
         writers"
    );

    drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("review_card never resumed after task_writes was released")
        .expect("review_card task panicked");
    assert!(result.is_ok());
}

/// The revalidation half of the same finding: a review reply parked on
/// `task_writes` while a second verdict already settled the card must see
/// the now-current column once it resumes, not the stale `in_review`
/// snapshot it would have clone from before it blocked — so it 404s
/// instead of silently re-applying on top of the settled card.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_404s_when_the_card_left_review_while_the_reply_was_in_flight() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(runtime.id(), &card_in_review("t-1", "strategy"))
        .await
        .unwrap();

    let guard = runtime.task_writes.lock().await;

    let runtime_for_task = runtime.clone();
    let mut task = tokio::spawn(async move {
        let scope = ScopedCompany {
            runtime: runtime_for_task,
            actor: None,
            may_read_contents: true,
            is_admin: true,
        };
        review_card(
            scope,
            Json(ChatReviewRequest {
                chat_id: "strategy".to_string(),
                task_id: "t-1".to_string(),
                decision: "approve".to_string(),
                note: None,
            }),
        )
        .await
    });
    let _ = tokio::time::timeout(Duration::from_millis(200), &mut task).await;

    let card = runtime
        .review_card_in_review("t-1", "strategy")
        .await
        .expect("task store lookup")
        .expect("card is still in_review before the lock is released");
    runtime
        .apply_review_decision(
            &card,
            crate::harness::built_in::lifecycle::ReviewDecision::Revise,
            Some("send it back"),
            None,
        )
        .await
        .unwrap();

    drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("review_card never resumed after task_writes was released")
        .expect("review_card task panicked");
    assert!(
        result.is_err(),
        "a review reply that had already resolved the card must not \
         silently re-apply its verdict once the card is no longer \
         in_review"
    );

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    assert_eq!(after.column, crate::ports::tasks::COLUMN_IN_PROGRESS);
    let note = after.note.expect("note");
    assert!(note.contains("send it back"), "{note}");
}

// -- Approval authority: deciding for the company, not addressing it -----

/// Both address forms. Every ops route is registered under two, and this
/// pair had already drifted apart: only the alias carried the
/// temporary-password refusal, so every assertion below runs against both.
const APPROVAL_SCOPES: [&str; 2] = ["/api/v1/companies/acme", "/api/v1/company"];

fn resolve_as(scope: &str, approval_id: &str, cookie: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}"))
        .header("content-type", "application/json");
    let builder = match cookie {
        Some(cookie) => builder.header("cookie", cookie),
        None => builder,
    };
    builder
        .body(Body::from(
            serde_json::json!({ "verdict": "deny" }).to_string(),
        ))
        .unwrap()
}

fn extend_as(scope: &str, approval_id: &str, cookie: Option<&str>) -> Request<Body> {
    let builder = Request::builder()
        .method("POST")
        .uri(format!("{scope}/approvals/{approval_id}/extend"));
    let builder = match cookie {
        Some(cookie) => builder.header("cookie", cookie),
        None => builder,
    };
    builder.body(Body::empty()).unwrap()
}

/// The sharpest case in this file. `may_read_approval_contents` already
/// refuses a member the payload and the amount an approval carries, so
/// before this guard a member could approve a payment they were forbidden
/// to look at.
///
/// The approval id is deliberately one that does not exist: authority is
/// settled before the approval is resolved, so the answer must be `403` and
/// not the `404` a permitted caller would get.
#[tokio::test]
async fn a_member_may_not_resolve_an_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        let denied = app
            .clone()
            .oneshot(resolve_as(scope, "appr-nobody-parked", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::FORBIDDEN,
            "{scope} let a member decide an approval"
        );
    }
}

/// Extending is the deadline's other side: an approval nobody decides
/// default-denies when its window runs out, so being able to push that
/// window out indefinitely is a decision about the effect, made for the
/// company. It is held to the same authority as deciding it outright.
#[tokio::test]
async fn a_member_may_not_extend_an_approval_deadline() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = park_for_extend(&runtime, "appr-member-ext", 1_000).await;
    let cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        let denied = app
            .clone()
            .oneshot(extend_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::FORBIDDEN,
            "{scope} let a member extend an approval deadline"
        );
    }
}

/// The other half of the guard: refusing a member must not also refuse the
/// admin the routes exist for, under either address form.
#[tokio::test]
async fn an_admin_may_still_resolve_an_approval() {
    for scope in APPROVAL_SCOPES {
        let home_dir = home();
        let state = state_with_company(home_dir.path(), "running").await;
        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let id = park_for_extend(&runtime, "appr-admin-resolve", 1_000).await;
        let cookie = crate::server::test_support::fixed_cookie("acme");
        let app = router(state);

        let allowed = app
            .oneshot(resolve_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            allowed.status(),
            StatusCode::OK,
            "{scope} refused an admin the decision"
        );
    }
}

#[tokio::test]
async fn an_admin_may_still_extend_an_approval_deadline() {
    for scope in APPROVAL_SCOPES {
        let home_dir = home();
        let state = state_with_company(home_dir.path(), "running").await;
        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let id = park_for_extend(&runtime, "appr-admin-ext", 1_000).await;
        let cookie = crate::server::test_support::fixed_cookie("acme");
        let app = router(state);

        let allowed = app
            .oneshot(extend_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            allowed.status(),
            StatusCode::OK,
            "{scope} refused an admin the extension"
        );
    }
}

/// No credential at all is `401`, not `403` — the authority guard must not
/// turn an anonymous request into a role decision.
#[tokio::test]
async fn an_unauthenticated_caller_cannot_decide_or_extend_an_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        for request in [
            resolve_as(scope, "appr-anon", None),
            extend_as(scope, "appr-anon", None),
        ] {
            let uri = request.uri().to_string();
            let denied = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                denied.status(),
                StatusCode::UNAUTHORIZED,
                "{uri} answered an anonymous caller with {}",
                denied.status()
            );
        }
    }
}

/// The second defect these routes carried: the temporary-password boundary
/// lived only on the single-company alias, so an admin who had never set a
/// password could decide and extend every approval through the `{id}` form.
///
/// An admin is the right principal to prove it with — the role check passes,
/// so a refusal here can only be the password boundary.
#[tokio::test]
async fn an_admin_on_a_temporary_password_may_not_decide_or_extend() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let cookie = crate::server::test_support::seed_temp_password_admin(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = park_for_extend(&runtime, "appr-temp-pass", 1_000).await;
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        for request in [
            resolve_as(scope, id.as_ref(), Some(&cookie)),
            extend_as(scope, id.as_ref(), Some(&cookie)),
        ] {
            let uri = request.uri().to_string();
            let denied = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                denied.status(),
                StatusCode::FORBIDDEN,
                "{uri} served an admin who has not set a password"
            );
            assert_eq!(
                body_json(denied).await["code"],
                "password_change_required",
                "{uri} refused for the wrong reason"
            );
        }
    }
}

// -- Deciding an approval: which states admit it, and what a failure costs --

/// STATE. `run_resolve` asks `ensure_running` before it touches the gate,
/// and the ordering is the guarantee: a company that has stopped accepting
/// work must refuse the decision *and leave the approval parked*, so the
/// operator still has a card to decide once it is running again.
///
/// A refusal that consumed the park would be worse than no refusal at all —
/// the effect would be neither approved nor decidable.
#[tokio::test]
async fn resolving_on_a_paused_company_is_refused_and_leaves_the_approval_parked() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-paused", crate::ports::now_millis()).await;
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let app = router(state);

    let lifecycle = |verb: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/companies/acme/{verb}"))
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap()
    };

    let paused = app.clone().oneshot(lifecycle("pause")).await.unwrap();
    assert_eq!(paused.status(), StatusCode::OK, "the company is now paused");

    for verdict in ["approve", "deny"] {
        let refused = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/company/approvals/{approval}"))
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "verdict": verdict }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            refused.status(),
            StatusCode::CONFLICT,
            "a paused company answered a {verdict} instead of refusing it"
        );
    }

    assert!(
        runtime.pending_approvals().iter().any(|p| p.id == approval),
        "the refusal must leave the approval decidable, not spend it"
    );
    assert_eq!(
        runtime.grants.live_count(),
        0,
        "and it must mint nothing on the way out"
    );

    let resumed = app.clone().oneshot(lifecycle("resume")).await.unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);
    let allowed = app
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "the same decision lands once the company is running again"
    );
}

/// CONC. Two operators on the same card — or one on a double click — reach
/// this route at the same time. The approval may settle once and buy one
/// permission; the loser must be told it was already decided rather than
/// minting a second grant against the same effect.
///
/// Distinct from [`a_second_resolve_reports_already_resolved_and_mints_nothing`],
/// which sends its second request only after the first has fully settled:
/// that one passes even if the parked-set take is a non-atomic
/// check-then-remove, because there is no window for the two to overlap in.
/// These two are in flight together.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_simultaneous_resolves_settle_once_and_mint_one_permission() {
    let home_dir = home();
    let c = stalled_company(home_dir.path()).await;
    c.release.notify_one();

    let approve = || {
        resolve_request(
            &c.approval_id,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        )
    };
    // Spawned onto a multi-threaded runtime, so these genuinely overlap
    // rather than being polled to completion one at a time. Eight rather
    // than two because the window a lost take opens is narrow: one pair can
    // miss it by scheduling luck, and a race this test cannot lose is worth
    // more than a tidier number.
    let racers: Vec<_> = (0..8)
        .map(|_| tokio::spawn(c.app.clone().oneshot(approve())))
        .collect();

    let mut settled = Vec::new();
    for racer in racers {
        let response = racer
            .await
            .expect("the request task did not panic")
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        settled.push(
            body["alreadyResolved"]
                .as_bool()
                .unwrap_or_else(|| panic!("a receipt says whether it settled: {body}")),
        );
    }
    assert_eq!(
        settled.iter().filter(|already| !**already).count(),
        1,
        "exactly one simultaneous resolve may settle the approval, got {settled:?}"
    );

    assert!(await_continuation(&c.runtime).await);
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "simultaneous approves must not buy more than one permission"
    );
}

/// FAIL. On the synchronous shape the operator waits for the follow-up
/// cycle, so a cycle that falls over is theirs to hear about: the request
/// answers an error rather than a success over nothing.
///
/// And the verdict is durable regardless — it is settled inline, before the
/// cycle is ever spawned. The pairing is the point. An error that also lost
/// the decision would leave the operator re-approving something already
/// approved; an error swallowed into a `200` would leave them believing work
/// resumed that never did.
#[tokio::test]
async fn a_synchronous_resolve_reports_a_failed_follow_up_and_keeps_the_verdict() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 1, Some("sales"), true).await;
    let approval = c.approvals[0].clone();

    let response = c
        .app
        .clone()
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a continuation that fell over must reach the operator waiting on it"
    );

    assert!(
        !c.runtime
            .pending_approvals()
            .iter()
            .any(|p| p.id == approval),
        "the verdict is settled before the cycle runs, so a failed cycle cannot un-decide it"
    );
    assert_eq!(c.runtime.grants.live_count(), 1);

    let again = c
        .app
        .clone()
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        body_json(again).await["alreadyResolved"],
        true,
        "re-deciding after the failure must say it was already decided"
    );
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "and must not buy a second permission"
    );
}

// -- resolve_attachments: IDOR-safe re-resolution, the attachment cap, --
// -- bad-id/folder refusal, and dedup (issue #1682) ----------------------

fn attachment_binary_node(id: &str, name: &str, mime: &str) -> WorkspaceNode {
    WorkspaceNode {
        id: id.to_string(),
        name: name.to_string(),
        kind: NodeKind::File,
        parent_id: None,
        updated_at_millis: 1_700_000_000_000,
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: Some(mime.to_string()),
        size: None,
        sha256: None,
        adopted: false,
    }
}

fn attachment_folder_node(id: &str, name: &str) -> WorkspaceNode {
    WorkspaceNode {
        id: id.to_string(),
        name: name.to_string(),
        kind: NodeKind::Folder,
        parent_id: None,
        updated_at_millis: 1_700_000_000_000,
        created_by: WorkspaceOrigin::Operator,
        updated_by: WorkspaceOrigin::Operator,
        mime: None,
        size: None,
        sha256: None,
        adopted: false,
    }
}

fn attachment_note_node(id: &str, name: &str) -> WorkspaceNode {
    WorkspaceNode {
        kind: NodeKind::File,
        ..attachment_folder_node(id, name)
    }
}

/// Two companies on one host, each with its own signed-in admin — the
/// shape a cross-company (IDOR) question needs, since a single-company
/// host cannot tell "refused for crossing a boundary" from "there was
/// nothing else to reach". Mirrors
/// `graphql::bridge_scope_test::state_with_two_companies`.
async fn state_with_two_companies(home: &std::path::Path) -> AppState {
    use crate::ports::store::CompanyStore;
    let store = FsCompanyStore::new(home.to_path_buf());
    let state = AppState::new(AppConfig::default());
    for name in ["acme", "globex"] {
        let id = CompanyId::new(name);
        let m = manifest();
        store
            .save(&CompanyRecord {
                overlay_desk_hive: Vec::new(),
                overlay_retired_agents: Vec::new(),
                overlay_agent_edits: Vec::new(),
                id: id.clone(),
                manifest: m.clone(),
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
        let runtime = RuntimeBuilder::new(home.to_path_buf(), m)
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        state.registry().insert(id, Arc::new(runtime));
        crate::server::test_support::seed_fixed_admin(&state, name).await;
    }
    state
}

fn chat_with_attachments(company: &str, attachments: Vec<String>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/v1/companies/{company}/chat"))
        .header("cookie", crate::server::test_support::fixed_cookie(company))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "text": "see attached", "attachments": attachments })
                .to_string(),
        ))
        .unwrap()
}

async fn last_operator_message_attachments(
    runtime: &Arc<CompanyRuntime>,
    company: &CompanyId,
) -> Vec<Attachment> {
    let events = runtime
        .events()
        .read_from(company, EventSeq::new(0), usize::MAX)
        .await
        .unwrap();
    events
        .into_iter()
        .rev()
        .find_map(|stored| match stored.event {
            CompanyEvent::OperatorMessage { attachments, .. } => Some(attachments),
            _ => None,
        })
        .expect("an operator message was journaled")
}

/// AUTH (IDOR). The client sends a `node_id` only; the host re-resolves it
/// within the *addressed* company's own tree rather than trusting the
/// caller. A real node minted under the exact same id in a different
/// company must not resolve through this one — proving the lookup is
/// scoped per company, not a global id space a guessable ULID could walk.
#[tokio::test]
async fn a_chat_attachment_cannot_cross_a_company_boundary() {
    let home_dir = home();
    let state = state_with_two_companies(home_dir.path()).await;
    let acme = state.registry().get(&CompanyId::new("acme")).unwrap();
    let globex = state.registry().get(&CompanyId::new("globex")).unwrap();

    // The exact same node id, minted for real, but only in globex.
    let shared_id = "n-cross-company";
    globex
        .workspace()
        .create_binary(
            &CompanyId::new("globex"),
            &attachment_binary_node(shared_id, "globex-only.png", "image/png"),
            b"globex bytes",
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(chat_with_attachments("acme", vec![shared_id.to_string()]))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a node real in another company must not resolve through this one's chat"
    );
    assert!(
        acme.events()
            .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
            .await
            .unwrap()
            .iter()
            .all(|stored| !matches!(stored.event, CompanyEvent::OperatorMessage { .. })),
        "a refused attachment must not journal a message with the wrong list"
    );
}

/// INPUT. An id naming nothing in this company's tree, and an id naming a
/// folder rather than a file, are both `400`s — but a genuine non-binary
/// **file** (a text note) is not: only the shape actually rejected is
/// rejected.
#[tokio::test]
async fn a_chat_attachment_refuses_an_unknown_id_and_a_folder_but_admits_a_note() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    runtime
        .workspace()
        .create(&id, &attachment_folder_node("f-1", "reports"), None)
        .await
        .unwrap();
    runtime
        .workspace()
        .create(
            &id,
            &attachment_note_node("n-1", "notes.md"),
            Some("just a note"),
        )
        .await
        .unwrap();

    let app = router(state);

    let unknown = app
        .clone()
        .oneshot(chat_with_attachments("acme", vec!["does-not-exist".into()]))
        .await
        .unwrap();
    assert_eq!(
        unknown.status(),
        StatusCode::BAD_REQUEST,
        "an id naming nothing in the tree must be refused"
    );

    let folder = app
        .clone()
        .oneshot(chat_with_attachments("acme", vec!["f-1".into()]))
        .await
        .unwrap();
    assert_eq!(
        folder.status(),
        StatusCode::BAD_REQUEST,
        "a folder id must be refused — it is not a file"
    );

    let admitted = app
        .oneshot(chat_with_attachments("acme", vec!["n-1".into()]))
        .await
        .unwrap();
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "a genuine non-binary file (a note) must still resolve"
    );
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].node_id, "n-1");
}

/// LIMIT. `MAX_CHAT_ATTACHMENTS` (20) is a hard cap on one message: one
/// over is refused before any tree scan or extraction runs, and exactly
/// at the cap is still ordinary, successful traffic.
#[tokio::test]
async fn a_chat_message_may_carry_at_most_twenty_attachments() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    let mut ids = Vec::new();
    for n in 0..21 {
        let node_id = format!("n-{n}");
        runtime
            .workspace()
            .create_binary(
                &id,
                &attachment_binary_node(
                    &node_id,
                    &format!("f{n}.bin"),
                    "application/octet-stream",
                ),
                b"x",
            )
            .await
            .unwrap();
        ids.push(node_id);
    }

    let app = router(state);

    let over_cap = app
        .clone()
        .oneshot(chat_with_attachments("acme", ids.clone()))
        .await
        .unwrap();
    assert_eq!(
        over_cap.status(),
        StatusCode::BAD_REQUEST,
        "21 attachments must be refused before any of them are resolved"
    );

    let at_cap = ids[..20].to_vec();
    let ok = app
        .oneshot(chat_with_attachments("acme", at_cap))
        .await
        .unwrap();
    assert_eq!(
        ok.status(),
        StatusCode::OK,
        "exactly 20 attachments is still ordinary traffic, not the refused shape"
    );
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(attachments.len(), 20);
}

/// BOUND. A repeated id collapses to exactly one resolved attachment — and
/// the cap is measured against the *raw* list the client sent, before
/// dedup, so a client cannot smuggle an over-cap request by repeating one
/// id past the limit and relying on dedup to shrink it back down.
#[tokio::test]
async fn a_chat_attachment_id_repeated_resolves_once_and_the_cap_counts_raw_entries() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    runtime
        .workspace()
        .create_binary(
            &id,
            &attachment_binary_node("n-dup", "one.png", "image/png"),
            b"one",
        )
        .await
        .unwrap();

    let app = router(state);

    // 21 copies of the same id: one unique attachment after dedup, but the
    // raw count is still over MAX_CHAT_ATTACHMENTS.
    let over_cap_by_repetition = vec!["n-dup".to_string(); 21];
    let refused = app
        .clone()
        .oneshot(chat_with_attachments("acme", over_cap_by_repetition))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "the cap must count the raw list the client sent, not the deduplicated one"
    );

    // Comfortably under the cap, repeated three times: dedup must collapse
    // it to exactly one resolved attachment.
    let repeated = vec!["n-dup".to_string(); 3];
    let ok = app
        .oneshot(chat_with_attachments("acme", repeated))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(
        attachments.len(),
        1,
        "a repeated id must resolve to exactly one attachment, not one per repetition"
    );
    assert_eq!(attachments[0].node_id, "n-dup");
}
