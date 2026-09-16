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

/// **Regression, issue #1882 review — blank must default the same as absent.**
/// A stored proposal that names `ownerDesk` as a blank/whitespace string (a
/// builder pass that emits the key but leaves it empty, rather than omitting
/// it) must still fall through to the assignee-desk default. Before the fix,
/// `Some("   ")` passed the `is_none()` gate in `apply_workflow_proposal`, so
/// the default never ran and the blank string was persisted as the "owner"
/// instead.
#[tokio::test]
async fn applying_a_proposal_with_a_blank_owner_desk_still_defaults_it() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let mut ops = digest_ops(None);
    ops["ownerDesk"] = json!("   ");
    let id = seed_proposal_card(&state, ops).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");

    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.clone());
    let record = store.load(&CompanyId::new("acme")).await.unwrap().unwrap();
    let overlay = record
        .overlay_workflows
        .iter()
        .find(|w| w.id == "weekly-digest")
        .expect("the created workflow is saved as an overlay");
    let file = crate::company::parse_workflow(&overlay.toml).expect("saved TOML parses");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("engineering"),
        "a blank ownerDesk must default the same as an omitted one: {file:?}"
    );
}

/// **Regression, issue #1882 review — a desk-assigned card must default to
/// its own desk.** `runtime::assignee::AssigneeResolution::canonical` stores
/// a desk assignment as the desk's own canonical id, not a teammate id — so
/// `record.assignee` can BE `"engineering"` directly. Before the fix, the
/// defaulting fallback only checked desk MEMBERSHIP (`desk_of_member`), which
/// a desk id is never a member of, so a card already naming its owning desk
/// still produced an ownerless workflow.
#[tokio::test]
async fn applying_a_proposal_for_a_desk_assigned_card_defaults_to_that_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = seed_proposal_card_assigned(&state, digest_ops(None), "engineering").await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");

    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.clone());
    let record = store.load(&CompanyId::new("acme")).await.unwrap().unwrap();
    let overlay = record
        .overlay_workflows
        .iter()
        .find(|w| w.id == "weekly-digest")
        .expect("the created workflow is saved as an overlay");
    let file = crate::company::parse_workflow(&overlay.toml).expect("saved TOML parses");
    assert_eq!(
        file.owner_desk.as_deref(),
        Some("engineering"),
        "a card assigned straight to a desk must default owner_desk to that desk: {file:?}"
    );
}

/// **Regression, issue #1882 review — a multi-desk teammate must not default
/// an arbitrary owner.** `desk_of_member` returns the first desk in
/// `desk_ids` declaration order, which is fine for the informational message
/// it was written for (`unknown_desk_message`) but wrong for a value that
/// gets persisted: a proposal naming no `ownerDesk`, assigned to a teammate
/// who sits on two desks, has no basis for picking either one. Before the
/// fix, `apply_workflow_proposal`'s fallback used `desk_of_member` directly
/// and silently persisted `"engineering"` — the desk declared first in the
/// manifest — even though `ceo` sits on `legal` too. The fix must leave
/// `owner_desk` `None` rather than guess.
#[tokio::test]
async fn applying_a_proposal_for_a_multi_desk_assignee_leaves_owner_desk_unset() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"engineering\"\nname = \"Engineering\"\nmembers = [\"ceo\"]\n\
         [[group_chat]]\nid = \"legal\"\nname = \"Legal\"\nmembers = [\"ceo\"]\n\
         [policy]\nmode = \"full\"\n",
    )
    .unwrap();
    let state = state_with_manifest(&home, manifest).await;
    let id = seed_proposal_card(&state, digest_ops(None)).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");

    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.clone());
    let record = store.load(&CompanyId::new("acme")).await.unwrap().unwrap();
    let overlay = record
        .overlay_workflows
        .iter()
        .find(|w| w.id == "weekly-digest")
        .expect("the created workflow is saved as an overlay");
    let file = crate::company::parse_workflow(&overlay.toml).expect("saved TOML parses");
    assert_eq!(
        file.owner_desk, None,
        "a teammate on two desks gives no basis for picking either one: {file:?}"
    );
}

/// #276: applying a proposal whose trigger carries a schedule creates the
/// workflow **switched off** — armed only by a person, never by approving the
/// proposal.
#[tokio::test]
async fn applying_a_scheduled_proposal_lands_it_disarmed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let id = seed_proposal_card(&state, digest_ops(Some("0 9 * * 1"))).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["column"], "done");

    let (_status, workflows) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    let created = workflows
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "weekly-digest")
        .expect("the created workflow is listed");
    assert_eq!(
        created["enabled"], false,
        "a scheduled graph lands disarmed until a person arms it (#276)"
    );
}

/// Roster drift (the proposal names a teammate no longer on the roster) is
/// refused by the create's roster check: the card **stays In Review** with its
/// proposal intact, and the refusal is a 400 the operator sees.
#[tokio::test]
async fn a_proposal_that_fails_validation_keeps_the_card_in_review() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let ops = json!({
        "id": "weekly-digest",
        "name": "Weekly digest",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "write", "kind": "agent", "name": "Draft it", "agent": "ghost" }
        ],
        "edges": [{ "from": "start", "to": "write" }]
    });
    let id = seed_proposal_card(&state, ops).await;

    let (status, _body) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The card is untouched save for the reason on its note: still In Review,
    // still carrying the proposal to retry once the roster is fixed.
    let (_status, card) = send(&state, "GET", &format!("/api/v1/company/tasks/{id}"), None).await;
    assert_eq!(card["task"]["stage"], "in_review");
    assert!(card["task"].get("workflowProposal").is_some(), "{card}");

    // …and no workflow was created.
    let (_status, workflows) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    assert!(
        workflows
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["id"] != "weekly-digest"),
        "a refused proposal must not leave a workflow behind"
    );
}

/// A company with one desk, so its runtime deliverable set is exactly
/// `["engineering"]` — enough to tell a channel target that works from one that
/// does not (issue #1191).
fn desk_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"engineering\"\nname = \"Engineering\"\nmembers = [\"ceo\"]\n\
         [policy]\nmode = \"full\"\n",
    )
    .unwrap()
}

/// [`digest_ops`], with the output node posting its report to `target`.
fn digest_ops_posting_to(target: &str) -> Value {
    json!({
        "id": "weekly-digest",
        "name": "Weekly digest",
        "description": "Post the weekly digest",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "write", "kind": "agent", "name": "Draft it", "agent": "ceo" },
            {
                "id": "post_summary",
                "kind": "output",
                "name": "Post to engineering desk",
                "destination": { "kind": "channel", "target": target }
            }
        ],
        "edges": [
            { "from": "start", "to": "write" },
            { "from": "write", "to": "post_summary" }
        ]
    })
}

/// **The #1191 regression.** The builder appended `-desk` to a desk's display
/// name, so the proposal routed its report to `engineering-desk` — not a channel
/// this runtime can deliver to.
///
/// Apply used to persist it: the operator was told "Workflow created — the card
/// is done", the card flipped to Done, and the workflow that now existed could
/// never deliver and could not be saved again from the editor without first
/// fixing a destination the operator never chose. Apply is a save, and it is now
/// held to the save rule — with the located `workflow_invalid` envelope, so the
/// console can say WHICH node.
#[tokio::test]
async fn applying_a_proposal_with_an_unwired_channel_is_refused_and_keeps_the_card_in_review() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = seed_proposal_card(&state, digest_ops_posting_to("engineering-desk")).await;

    let (status, body) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "workflow_invalid", "{body}");
    let problem = body["problems"]
        .as_array()
        .unwrap_or_else(|| panic!("the refusal must carry a breakdown: {body}"))
        .iter()
        .find(|p| p["node_id"] == "post_summary")
        .unwrap_or_else(|| panic!("no problem names the output node: {body}"));
    assert_eq!(problem["field"], "destination.target", "{body}");
    assert!(
        problem["message"]
            .as_str()
            .unwrap_or_default()
            .contains("is not an automation delivery channel"),
        "{body}"
    );

    // The card is recoverable, exactly as it is for roster drift: still In
    // Review, still carrying its proposal, with the reason on its note.
    let (_status, card) = send(&state, "GET", &format!("/api/v1/company/tasks/{id}"), None).await;
    assert_eq!(card["task"]["stage"], "in_review", "{card}");
    assert!(card["task"].get("workflowProposal").is_some(), "{card}");
    assert!(
        card["task"]["note"]
            .as_str()
            .unwrap_or_default()
            .contains("still waiting for review"),
        "{card}"
    );

    // …and nothing was persisted.
    let (_status, workflows) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    assert!(
        workflows
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["id"] != "weekly-digest"),
        "a refused apply must not leave a workflow behind: {workflows}"
    );
}

/// The invariant the defect broke, stated directly: whatever apply persists,
/// the ordinary editor save route accepts back unchanged.
///
/// Before #1191 these two routes gave opposite answers to the same bytes —
/// apply created the graph and `PUT` refused it — so the operator's first edit
/// of a copilot-built workflow was blocked on a destination they never chose.
#[tokio::test]
async fn an_applied_proposal_can_be_saved_again_unchanged() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
    let id = seed_proposal_card(&state, digest_ops_posting_to("engineering")).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["column"], "done");

    // Read the created graph back and save it straight to the editor's route,
    // byte-for-byte, with its own version token.
    let (status, graph) = send(
        &state,
        "GET",
        "/api/v1/company/workflows/weekly-digest",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{graph}");

    let mut body = graph.clone();
    body["expectedVersion"] = graph["version"].clone();
    let (status, saved) = send(
        &state,
        "PUT",
        "/api/v1/company/workflows/weekly-digest",
        Some(body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "what apply persists, the editor must accept back unchanged: {saved}"
    );
}

/// Rejecting a proposal returns the card to To-do and clears the proposal
/// (decision D2c). The card keeps its `workflow` deliverable, so it can be built
/// again.
#[tokio::test]
async fn rejecting_a_proposal_returns_the_card_to_todo() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let id = seed_proposal_card(&state, digest_ops(None)).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/reject"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    assert_eq!(card["column"], "pending");
    assert!(card.get("workflowProposal").is_none(), "{card}");
    assert_eq!(
        card["deliverable"], "workflow",
        "reject keeps the deliverable"
    );
}

/// Applying or rejecting a card that has no proposal is a 400, not a silent
/// no-op — the operator asked for an action on something that is not there.
#[tokio::test]
async fn applying_with_no_proposal_is_a_bad_request() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let (_status, task) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({ "title": "Plain card" })),
    )
    .await;
    let id = task["id"].as_str().unwrap().to_string();

    for verb in ["apply", "reject"] {
        let (status, _body) = send(
            &state,
            "POST",
            &format!("/api/v1/company/tasks/{id}/workflow-proposal/{verb}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{verb} with no proposal");
    }
}

/// The create route accepts an explicit `deliverable`, and it round-trips on the
/// board read — the operator's once-vs-workflow choice (D2a), with `once` staying
/// off the wire so a plain card is byte-identical to a pre-#580 one.
#[tokio::test]
async fn a_card_can_be_created_as_a_workflow_deliverable() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, workflow_card) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({ "title": "Automate onboarding", "deliverable": "workflow" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(workflow_card["deliverable"], "workflow");

    let (_status, once_card) = send(
        &state,
        "POST",
        "/api/v1/company/tasks",
        Some(json!({ "title": "One-off note" })),
    )
    .await;
    assert!(
        once_card.get("deliverable").is_none(),
        "a once card stays off the wire: {once_card}"
    );

    // A patch can flip a once card to workflow before it is dragged into In
    // Progress.
    let id = once_card["id"].as_str().unwrap();
    let (status, flipped) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/tasks/{id}"),
        Some(json!({ "deliverable": "workflow" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(flipped["deliverable"], "workflow");
}

// ---------------------------------------------------------------------------
// Binary workspace nodes over HTTP (issue #553)
// ---------------------------------------------------------------------------

/// Sends a `multipart/form-data` upload with one file part and an optional
/// `parentId`, hand-rolling the body so the test exercises the real
/// `Multipart` extractor rather than a stub.
async fn upload_file(
    state: &AppState,
    filename: &str,
    content_type: Option<&str>,
    bytes: &[u8],
    parent_id: Option<&str>,
) -> (StatusCode, Value) {
    const BOUNDARY: &str = "----opencompany553boundary";
    let mut body: Vec<u8> = Vec::new();
    if let Some(parent) = parent_id {
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"parentId\"\r\n\r\n{parent}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    if let Some(ct) = content_type {
        body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
    }
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/company/workspace/upload")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let out = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = if out.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&out).unwrap_or(Value::Null)
    };
    (status, value)
}
