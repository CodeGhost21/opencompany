//! Integration tests for the `ops` write plane: tasks, memory, workspace,
//! skills, team, inbox-read, and desk chat — exercised end-to-end over the
//! router against a real fs-backed company.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::tests_mcp_add_probes_without::discussion_card;
use super::write_test_support::*;
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

/// #358: a posted message can be withdrawn, and the text stops being served.
///
/// The shape the issue asks for, asserted end to end over the real HTTP stack:
/// the row survives (position, author, time), the text does not, the withdrawal
/// is attributed, the journal keeps both events, and nothing about a message
/// nobody withdrew changes.
#[tokio::test]
async fn a_withdrawn_discussion_message_stops_being_served() {
    use crate::ports::types::CompanyEvent;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    runtime
        .tasks()
        .upsert(&company, &discussion_card("t-1", "Ship it"))
        .await
        .unwrap();

    const SECRET: &str = "sk-live-0000-DO-NOT-KEEP";
    let (status, posted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-1/discussion",
        Some(json!({ "text": format!("blocked on the API key: {SECRET}") })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let seq = posted["seq"].as_u64().expect("the post carries its seq");

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-1/discussion",
        Some(json!({ "text": "rotated it, we are unblocked" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, withdrawn) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/t-1/discussion/{seq}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(withdrawn["redacted"], true);
    assert_eq!(
        withdrawn["redactedBy"], "Harness Admin",
        "a withdrawal nobody's name is on is a message that can vanish quietly"
    );
    assert_eq!(
        withdrawn["seq"], seq,
        "the row keeps its place in the thread"
    );

    // The reload: what every reader of this card now gets.
    let (status, body) = send(&state, "GET", "/api/v1/company/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    let thread = body["discussion"].as_array().expect("discussion array");
    assert_eq!(thread.len(), 2, "the row is withdrawn, not deleted: {body}");
    assert_eq!(thread[0]["seq"], seq);
    assert_eq!(thread[0]["redacted"], true);
    assert_eq!(thread[0]["redactedBy"], "Harness Admin");
    assert_eq!(
        thread[0]["author"], "Harness Admin",
        "the poster is still named"
    );
    assert_eq!(
        thread[1]["text"], "rotated it, we are unblocked",
        "withdrawing one message must not touch another"
    );
    assert!(
        thread[1].get("redacted").is_none(),
        "an ordinary row must keep the shape a pre-#358 console renders: {thread:?}"
    );
    assert!(
        !serde_json::to_string(&body).unwrap().contains(SECRET),
        "the withdrawn text is still being served on the detail read: {body}"
    );

    // The journal keeps both events: the post's existence is a fact, and the
    // withdrawal is a second fact about it.
    let events = runtime
        .events()
        .read_from(&company, crate::ports::types::EventSeq::new(0), usize::MAX)
        .await
        .unwrap();
    assert!(
        events.iter().any(|e| matches!(
            &e.event,
            CompanyEvent::TaskDiscussionRedacted { task_id, seq: s, .. }
                if task_id == "t-1" && *s == seq
        )),
        "the withdrawal was not journaled"
    );

    // Idempotent: asking twice is not an error, and does not grow the journal.
    let before = events.len();
    let (status, again) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/t-1/discussion/{seq}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["redacted"], true);
    let after = runtime
        .events()
        .read_from(&company, crate::ports::types::EventSeq::new(0), usize::MAX)
        .await
        .unwrap()
        .len();
    assert_eq!(
        before, after,
        "a repeated withdrawal appended a second tombstone"
    );
}

/// #358: a `seq` that is not a discussion post on *this* card is a `404`, not a
/// tombstone written into the journal against something else.
#[tokio::test]
async fn withdrawing_something_that_is_not_this_cards_post_is_refused() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    for card in [
        discussion_card("t-1", "Ship it"),
        discussion_card("t-other", "Unrelated"),
    ] {
        runtime.tasks().upsert(&company, &card).await.unwrap();
    }

    let (_, posted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-other/discussion",
        Some(json!({ "text": "another card's message" })),
    )
    .await;
    let seq = posted["seq"].as_u64().unwrap();

    // Another card's post, addressed through this card.
    let (status, _) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/t-1/discussion/{seq}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A sequence position that holds no event at all.
    let (status, _) = send(
        &state,
        "DELETE",
        "/api/v1/company/tasks/t-1/discussion/99999",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The other card's thread is untouched by either attempt.
    let (_, other) = send(&state, "GET", "/api/v1/company/tasks/t-other", None).await;
    let thread = other["discussion"].as_array().unwrap();
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0]["text"], "another card's message");
    assert!(thread[0].get("redacted").is_none());
}

/// #358 + #335's paging: a withdrawal is applied even when the tombstone sits
/// *newer* than the cursor the caller is paging back through.
///
/// The trap this pins: the discussion arm skips events at or after
/// `discussionBefore`, and a tombstone is always newer than the post it
/// withdraws. Applying the cursor to tombstones too would serve the original
/// text to anybody who scrolled far enough back — the one reader most likely to
/// be looking for it.
#[tokio::test]
async fn a_withdrawal_survives_paging_back_past_it() {
    use crate::ports::types::CompanyEvent;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    runtime
        .tasks()
        .upsert(&company, &discussion_card("t-1", "Ship it"))
        .await
        .unwrap();

    const SECRET: &str = "sk-live-PAGED-BACK";
    let (_, posted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-1/discussion",
        Some(json!({ "text": SECRET })),
    )
    .await;
    let seq = posted["seq"].as_u64().unwrap();

    // Enough newer posts that the first one falls off the first page.
    for n in 0..60 {
        runtime
            .events()
            .append(
                &company,
                CompanyEvent::TaskDiscussionPosted {
                    task_id: "t-1".into(),
                    text: format!("message {n}"),
                    by: None,
                },
            )
            .await
            .unwrap();
    }

    let (status, _) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/tasks/t-1/discussion/{seq}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Page back to the start of the thread, past the tombstone's own position.
    let (_, first) = send(&state, "GET", "/api/v1/company/tasks/t-1", None).await;
    let oldest_on_page = first["discussion"].as_array().unwrap()[0]["seq"]
        .as_u64()
        .unwrap();
    let (status, older) = send(
        &state,
        "GET",
        &format!("/api/v1/company/tasks/t-1?discussionBefore={oldest_on_page}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !serde_json::to_string(&older).unwrap().contains(SECRET),
        "paging back served the withdrawn text: {older}"
    );
    let row = older["discussion"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["seq"] == seq)
        .expect("the withdrawn row is on the older page");
    assert_eq!(row["redacted"], true);
}

/// #335: the per-task Discussion tab's whole contract — a post persists, reads
/// back on the card's own detail, and belongs to exactly one card.
///
/// The acceptance criterion is "posts survive a reload and are visible from
/// another browser", which is the same thing as: the message lives in the
/// company journal, not in the posting session. So the assertions are made
/// through a *second, independent request* rather than off the POST's echo.
///
/// The scoping half matters as much: the journal is company-scoped, so a fold
/// that forgot to compare `task_id` would show every card the same thread.
#[tokio::test]
async fn task_discussion_posts_persist_and_are_scoped_to_their_card() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();

    for card in [
        discussion_card("t-1", "Ship it"),
        discussion_card("t-other", "Unrelated"),
        discussion_card("t-quiet", "Nobody has said anything"),
    ] {
        runtime.tasks().upsert(&company, &card).await.unwrap();
    }

    // Surrounding whitespace is trimmed, and the poster is named from the
    // roster — never by user id, and never by email address.
    let (status, posted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-1/discussion",
        Some(json!({ "text": "  blocked on the API key  " })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(posted["text"], "blocked on the API key");
    assert_eq!(posted["author"], "Harness Admin");

    for (task, text) in [
        ("t-1", "unblocked, the key was rotated"),
        ("t-other", "someone else's thread"),
    ] {
        let (status, _) = send(
            &state,
            "POST",
            &format!("/api/v1/company/tasks/{task}/discussion"),
            Some(json!({ "text": text })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // The reload: a fresh read of the card, which reaches the journal rather
    // than anything the posting request kept.
    let (status, body) = send(&state, "GET", "/api/v1/company/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    let thread = body["discussion"].as_array().expect("discussion array");
    let texts: Vec<&str> = thread.iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        vec!["blocked on the API key", "unblocked, the key was rotated"],
        "the thread reads back oldest-first"
    );
    assert!(
        thread[0]["seq"].as_u64().unwrap() < thread[1]["seq"].as_u64().unwrap(),
        "seq is the thread's strict order: {thread:?}"
    );
    assert!(
        !serde_json::to_string(&body["discussion"])
            .unwrap()
            .contains("someone else's thread"),
        "another card's message leaked onto this thread"
    );

    // The two projections stay apart: a discussion post is not a run event, so
    // it must not appear on the timeline the Timeline tab renders.
    assert!(
        body["timeline"].as_array().unwrap().is_empty(),
        "a discussion post must not land on the run timeline: {body}"
    );

    // The other card sees only its own message, and a card nobody has posted on
    // reads back an empty thread — what keeps the tab's empty state honest.
    let (_, other) = send(&state, "GET", "/api/v1/company/tasks/t-other", None).await;
    let other_thread = other["discussion"].as_array().unwrap();
    assert_eq!(other_thread.len(), 1);
    assert_eq!(other_thread[0]["text"], "someone else's thread");

    let (_, quiet) = send(&state, "GET", "/api/v1/company/tasks/t-quiet", None).await;
    assert_eq!(quiet["discussion"].as_array().unwrap().len(), 0);

    // Both scope forms serve the same thread.
    let (status, scoped) = send(&state, "GET", "/api/v1/companies/acme/tasks/t-1", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scoped["discussion"].as_array().unwrap().len(), 2);
}

/// #335: what the write boundary refuses, and what it forgives.
///
/// An empty message is refused because there is no delete in v1 — a blank row
/// would be permanent noise. An unknown card is refused because the post would
/// otherwise be journaled somewhere no read surface can reach. An over-long
/// message is *not* refused: it is truncated, so a long paste still posts.
#[tokio::test]
async fn task_discussion_rejects_an_empty_message_and_an_unknown_card() {
    use crate::ports::tasks::MAX_DISCUSSION_CHARS;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    runtime
        .tasks()
        .upsert(&company, &discussion_card("t-1", "Ship it"))
        .await
        .unwrap();

    for text in ["", "   \n\t "] {
        let (status, _) = send(
            &state,
            "POST",
            "/api/v1/company/tasks/t-1/discussion",
            Some(json!({ "text": text })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "empty text: {text:?}");
    }

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/nope/discussion",
        Some(json!({ "text": "into the void" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A long paste posts, capped on a character boundary.
    let long = "é".repeat(MAX_DISCUSSION_CHARS + 500);
    let (status, posted) = send(
        &state,
        "POST",
        "/api/v1/company/tasks/t-1/discussion",
        Some(json!({ "text": long })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        posted["text"].as_str().unwrap().chars().count(),
        MAX_DISCUSSION_CHARS
    );

    // Only the accepted post is on the thread: the three refusals journaled
    // nothing.
    let (_, body) = send(&state, "GET", "/api/v1/company/tasks/t-1", None).await;
    assert_eq!(body["discussion"].as_array().unwrap().len(), 1);
}
