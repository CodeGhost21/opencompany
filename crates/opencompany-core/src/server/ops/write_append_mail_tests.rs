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

/// Appends one received email to `inbox`, for the read-surface tests below.
async fn append_mail(
    runtime: &crate::company::runtime::CompanyRuntime,
    inbox: &str,
    id: &str,
    subject: &str,
    at_millis: u64,
) {
    use crate::ports::inbox::EmailRecord;
    runtime
        .inbox()
        .append(
            runtime.id(),
            &EmailRecord {
                id: id.into(),
                inbox: inbox.into(),
                from_name: format!("{inbox} correspondent"),
                from_email: format!("{inbox}-sender@x.test"),
                subject: subject.into(),
                body: format!("body for {subject}"),
                at_millis,
                read: false,
                outbound: false,
            },
        )
        .await
        .unwrap();
}

/// The regression for issue #173: two teammates' inboxes must read back as two
/// *different* sets of mail. The console used to render a client-side fixture —
/// the same four invented emails for everybody — because no per-agent read was
/// reachable over REST at all.
#[tokio::test]
async fn inbox_reads_are_per_agent_and_never_shared() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // Enable two inboxes and file distinct mail in each. Inbox keys are agent
    // ids; `cto` is an operator-added teammate as far as the toggle cares, so it
    // takes its own key without a manifest entry.
    for agent in ["ceo", "cto"] {
        let (status, _) = send(
            &state,
            "PUT",
            &format!("/api/v1/company/team/{agent}/inbox"),
            Some(json!({"enabled": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    append_mail(&runtime, "ceo", "c1", "board deck", 10).await;
    append_mail(&runtime, "ceo", "c2", "investor intro", 20).await;
    append_mail(&runtime, "cto", "t1", "on-call rotation", 30).await;

    // The roster lists both, each with its own unread count.
    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let ceo = rows.iter().find(|r| r["key"] == "ceo").unwrap();
    let cto = rows.iter().find(|r| r["key"] == "cto").unwrap();
    assert_eq!(ceo["enabled"], true);
    assert_eq!(ceo["unread"], 2);
    assert_eq!(cto["unread"], 1);

    // Each inbox reads back only its own mail — the shared-fixture bug. The
    // route serves store (append) order; the console sorts newest-first.
    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes/ceo/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    let ceo_subjects: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["subject"].as_str().unwrap())
        .collect();
    assert_eq!(ceo_subjects, vec!["board deck", "investor intro"]);

    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes/cto/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["subject"], "on-call rotation");
    assert_eq!(items[0]["fromEmail"], "cto-sender@x.test");
    assert_eq!(items[0]["inbox"], "cto");
}

/// An inbox nobody has mail in — or that does not exist at all — reads as an
/// empty list rather than a 404. An enabled-but-empty inbox is a legitimate
/// state, and the console must render it as such rather than as an error.
#[tokio::test]
async fn inbox_messages_soft_fail_on_unknown_key() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    append_mail(&runtime, "ceo", "m0", "mail 0", 1).await;

    let (status, body) = send(
        &state,
        "GET",
        "/api/v1/company/inboxes/nobody/messages",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_array().unwrap().is_empty());

    // …and the inbox that *does* hold mail is unaffected by that read.
    let (_, body) = send(&state, "GET", "/api/v1/company/inboxes/ceo/messages", None).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}

/// An inbox switched on but never written to is still listed, so the console can
/// show it the moment the Team toggle flips — and `GET …/team` reports the same
/// enabled state, so the toggle isn't a client-side guess.
#[tokio::test]
async fn team_read_reports_inbox_enabled_and_empty_inbox_is_listed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Before the toggle: no inbox on the roster, and nothing listed.
    let (status, roster) = send(&state, "GET", "/api/v1/company/team", None).await;
    assert_eq!(status, StatusCode::OK);
    let ceo = roster
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "ceo")
        .unwrap()
        .clone();
    assert_eq!(ceo["inboxEnabled"], false);
    let (_, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    assert!(body.as_array().unwrap().is_empty());

    // Toggle it on: listed with zero mail, and the roster agrees.
    let (status, _) = send(
        &state,
        "PUT",
        "/api/v1/company/team/ceo/inbox",
        Some(json!({"enabled": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    let rows = body.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["key"], "ceo");
    assert_eq!(rows[0]["enabled"], true);
    assert_eq!(rows[0]["unread"], 0);
    // The manifest role is the display name until a domain gives it an address.
    assert_eq!(rows[0]["name"], "Chief");

    let (_, roster) = send(&state, "GET", "/api/v1/company/team", None).await;
    let ceo = roster
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "ceo")
        .unwrap()
        .clone();
    assert_eq!(ceo["inboxEnabled"], true);

    // Toggling back off keeps the inbox listed but disabled — the console
    // filters on `enabled`, so it drops out of the selector without losing mail.
    let (status, _) = send(
        &state,
        "PUT",
        "/api/v1/company/team/ceo/inbox",
        Some(json!({"enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    assert_eq!(body.as_array().unwrap()[0]["enabled"], false);
}

/// Mail that arrives through the ingest webhook is exactly what the console's
/// read surface returns — the end-to-end path issue #173's repro step 4 walked.
#[tokio::test]
async fn ingested_mail_shows_up_on_the_console_read_surface() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // Straight into the store, as `file_and_notify` does for a verified payload
    // (the HMAC path itself is covered in `ops::test`).
    append_mail(&runtime, "ceo", "ingested-1", "hello from outside", 42).await;

    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes/ceo/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "ingested-1");
    assert_eq!(items[0]["subject"], "hello from outside");
    assert_eq!(items[0]["read"], false);
    assert_eq!(items[0]["outbound"], false);

    // Reading it drops the unread count the selector badges.
    let (_, body) = send(
        &state,
        "POST",
        "/api/v1/company/inboxes/ceo/read",
        Some(json!({"ids": ["ingested-1"]})),
    )
    .await;
    assert_eq!(body["unread"], 0);
    let (_, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    assert_eq!(body.as_array().unwrap()[0]["unread"], 0);
}

#[tokio::test]
async fn inbox_list_and_messages_project_store() {
    use crate::ports::inbox::EmailRecord;
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    // One inbound (unread) + one outbound reply in inbox "ceo".
    runtime
        .inbox()
        .append(
            runtime.id(),
            &EmailRecord {
                id: "in1".into(),
                inbox: "ceo".into(),
                from_name: "Priya".into(),
                from_email: "p@x.test".into(),
                subject: "hi".into(),
                body: "hello world".into(),
                at_millis: 1,
                read: false,
                outbound: false,
            },
        )
        .await
        .unwrap();
    runtime
        .inbox()
        .append(
            runtime.id(),
            &EmailRecord {
                id: "out1".into(),
                inbox: "ceo".into(),
                from_name: String::new(),
                from_email: "ceo@acme.test".into(),
                subject: "re: hi".into(),
                body: "reply".into(),
                at_millis: 2,
                read: false,
                outbound: true,
            },
        )
        .await
        .unwrap();

    // GET /inboxes surfaces the message-bearing inbox; outbound doesn't count toward unread.
    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes", None).await;
    assert_eq!(status, StatusCode::OK);
    let ceo = body
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["key"] == "ceo")
        .expect("ceo inbox listed");
    assert_eq!(ceo["unread"], 1);

    // GET messages returns both, camelCase, oldest first.
    let (status, body) = send(&state, "GET", "/api/v1/company/inboxes/ceo/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    let msgs = body.as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["id"], "in1");
    assert_eq!(msgs[0]["fromEmail"], "p@x.test");
    assert_eq!(msgs[1]["outbound"], true);
}

/// Issue #416, the half that holds in every build: a question asked on a
/// workflow copilot thread must not leave work on the company's board.
///
/// The copilot is a conversation *about a graph*, and its questions are phrased
/// at the graph — "add a node that emails the report". The chat route's
/// deterministic intent detector reads that as a request to the company and
/// opens a `todo` card, which is the same class of over-reach this issue is
/// about, reached from the route rather than from the model. The control half
/// matters as much as the confined half: the identical sentence on an ordinary
/// thread still opens its card, so this narrows the copilot rather than
/// disabling a feature.
#[tokio::test]
async fn a_copilot_thread_question_opens_no_board_card() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Deliberately the most actionable phrasing the copilot invites.
    let ask = "build a node that emails the weekly report";

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({"message": ask, "chat": "workflow-copilot:weekly_report"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    assert_eq!(status, StatusCode::OK);
    // Strict on the shape, like the control half below: `is_none_or` would pass
    // on a body that is not a list at all, so a route that started answering an
    // error object would go green here and panic there — reported as a failure
    // of the control rather than of the thing under test.
    let cards = board.as_array().expect("the board lists cards");
    assert!(
        cards.is_empty(),
        "a copilot question left work on the board: {board}"
    );

    // The same rule reaches the other deterministic side effect a chat turn
    // has: a complaint phrase on a copilot thread is the operator correcting a
    // conversation about their graph, not feedback about the company's work.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({
            "message": "no, that is wrong, this node keeps failing",
            "chat": "workflow-copilot:weekly_report",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, filed) = send(&state, "GET", "/api/v1/company/feedback", None).await;
    assert_eq!(status, StatusCode::OK);
    let items = filed.as_array().expect("the feedback list is an array");
    assert!(
        items.is_empty(),
        "a copilot correction filed company feedback: {filed}"
    );

    // Control: the same sentence on the ordinary thread still opens a card, so
    // the suppression is scoped to the copilot and not a regression of #246.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({"message": ask})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    assert_eq!(status, StatusCode::OK);
    let cards = board.as_array().expect("the board lists cards");
    assert_eq!(
        cards.len(),
        1,
        "the ordinary thread must still open exactly one card: {board}"
    );
}

/// Issue #267: a question about the board's own state is answered, not carded.
///
/// This is the exact message that produced one of the six dead `backlog` cards
/// on a live company. The route now triages it as `Answer`, so the
/// deterministic card path stands down — and the reply still comes back OK,
/// because triage decides what gets *written*, never whether the operator gets
/// an answer.
///
/// The control half is the point: the same route, one sentence later, still
/// opens a card for a real instruction. A test that only proved the question
/// wrote nothing would also pass on a route that had stopped carding entirely.
#[tokio::test]
async fn a_question_about_the_board_opens_no_card() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    for ask in [
        "what is there in the tasks list?",
        "Tell what is there in the tasks list",
        "list the tasks",
        "show me the board",
    ] {
        let (status, body) = send(
            &state,
            "POST",
            "/api/v1/company/chat",
            Some(json!({ "message": ask })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "asking `{ask}` must still answer");
        assert!(body["responses"].is_array(), "no reply for `{ask}`: {body}");

        let (status, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
        assert_eq!(status, StatusCode::OK);
        let cards = board.as_array().expect("the board lists cards");
        assert!(
            cards.is_empty(),
            "the question `{ask}` left work on the board: {board}"
        );
    }

    // Control: a real instruction on the same route still opens exactly one
    // card, so this narrows the detector rather than switching it off.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({"message": "build the landing page"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, board) = send(&state, "GET", "/api/v1/company/tasks", None).await;
    assert_eq!(status, StatusCode::OK);
    let cards = board.as_array().expect("the board lists cards");
    assert_eq!(
        cards.len(),
        1,
        "an instruction must still open exactly one card: {board}"
    );
}

#[tokio::test]
async fn chat_accepts_desk_id_and_replies() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({"message": "hello", "chat": "Creative studio"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["responses"].is_array());
}

#[tokio::test]
async fn credential_route_rejects_foreign_tenant() {
    use crate::server::platform_auth::{
        PlatformAuthConfig, PlatformClaims, UnsignedTenantVerifier,
    };
    use std::collections::HashSet;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    // Platform mode: `acme` is owned by `tenant:acme`.
    let verifier = std::sync::Arc::new(UnsignedTenantVerifier::new("plat-secret"));
    let state = AppState::new(AppConfig::default())
        .with_home(home.clone())
        .with_platform_auth(PlatformAuthConfig::new(verifier));
    let id = CompanyId::new("acme");
    let runtime = RuntimeBuilder::new(home.clone(), manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    state
        .registry()
        .insert(id.clone(), std::sync::Arc::new(runtime));
    state.set_owner(id.clone(), "tenant:acme");

    let token = |tenant: &str| {
        UnsignedTenantVerifier::tenant_token(&PlatformClaims {
            tenant: tenant.to_string(),
            scopes: HashSet::from(["operator".to_string()]),
            companies: None,
        })
    };

    // A foreign tenant cannot set acme's domain (credential route is scoped).
    let (status, _) = send_auth(
        &state,
        "PUT",
        "/api/v1/companies/acme/domain",
        Some(json!({"domain": "acme.test"})),
        Some(&token("tenant:evil")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owning tenant succeeds.
    let (status, _) = send_auth(
        &state,
        "PUT",
        "/api/v1/companies/acme/domain",
        Some(json!({"domain": "acme.test"})),
        Some(&token("tenant:acme")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn unknown_company_scope_is_404() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/companies/ghost/tasks",
        Some(json!({"title": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// MCP servers (issue #50)
// ---------------------------------------------------------------------------

