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

/// Nearly every note in the tree is an ordinary note, not a deliverable.
/// Saving one must append nothing anywhere — the reverse lookup answering
/// "no artifact owns this" is the common case, and deliberately silent.
#[tokio::test]
async fn saving_an_unpublished_note_appends_no_artifact_version() {
    use crate::ports::artifacts::{ArtifactKind, ArtifactRecord, ArtifactStore};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = state.registry().list()[0].clone();
    let runtime = state.registry().get(&company).expect("company");

    // A published artifact exists, but points at a DIFFERENT node.
    let mut published = ArtifactRecord::new(
        "art-1",
        "t-1",
        "Launch spec",
        ArtifactKind::Markdown,
        "deliverable",
        "ceo",
        1,
    );
    published.stamp_workspace_node("some-other-node");
    ArtifactStore::upsert(runtime.artifacts().as_ref(), &company, &published)
        .await
        .expect("seed");

    let (_, note) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "notes.md", "kind": "file", "content": "just a note"})),
    )
    .await;
    let node_id = note["id"].as_str().unwrap().to_string();

    let (status, _) = send(
        &state,
        "PUT",
        &format!("/api/v1/company/workspace/file/{node_id}"),
        Some(json!({"content": "still just a note"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, artifact) = send(&state, "GET", "/api/v1/company/artifacts/art-1", None).await;
    assert_eq!(
        artifact["versions"].as_array().unwrap().len(),
        1,
        "an ordinary note's save must not touch an unrelated artifact"
    );
}

/// The other direction of the same invariant: appending a version through the
/// Artifacts tab must push the new body into the deliverable's workspace note,
/// or the tree keeps serving a draft the history has superseded.
#[tokio::test]
async fn appending_an_artifact_version_updates_its_workspace_note() {
    use crate::ports::artifacts::{ArtifactKind, ArtifactRecord, ArtifactStore};
    use crate::ports::workspace::WorkspaceStore;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = state.registry().list()[0].clone();
    let runtime = state.registry().get(&company).expect("company");

    let (_, note) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "launch.md", "kind": "file", "content": "v1"})),
    )
    .await;
    let node_id = note["id"].as_str().unwrap().to_string();

    let mut published = ArtifactRecord::new(
        "art-1",
        "t-1",
        "Launch spec",
        ArtifactKind::Markdown,
        "v1",
        "ceo",
        1,
    )
    .with_source("launch.md");
    published.stamp_workspace_node(&node_id);
    ArtifactStore::upsert(runtime.artifacts().as_ref(), &company, &published)
        .await
        .expect("seed");

    let (status, appended) = send(
        &state,
        "POST",
        "/api/v1/company/artifacts/art-1/versions",
        Some(json!({"body": "v2, edited in the Artifacts tab"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        appended["versions"][1]["workspaceNodeId"], node_id,
        "the appended version keeps naming the node it lives in"
    );

    let (_, body) = WorkspaceStore::read(runtime.workspace().as_ref(), &company, &node_id)
        .await
        .unwrap()
        .expect("the note exists");
    assert_eq!(
        body, "v2, edited in the Artifacts tab",
        "the shared tree must not keep serving a superseded draft"
    );
}

/// An artifact with no workspace note — a legacy capture, or one recorded
/// while no tree was wired — appends exactly as it always did, with no node
/// write attempted and nothing invented for it.
#[tokio::test]
async fn appending_to_an_unmirrored_artifact_touches_no_note() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/artifacts",
        Some(json!({"taskId": "t-1", "title": "Draft", "body": "v1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();

    let (status, appended) = send(
        &state,
        "POST",
        &format!("/api/v1/company/artifacts/{id}/versions"),
        Some(json!({"body": "v2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(appended["versions"].as_array().unwrap().len(), 2);
    assert!(
        appended["versions"][1].get("workspaceNodeId").is_none(),
        "nothing may invent a node for an artifact that has none"
    );
}

/// An artifact store with one chosen fault, so a test can ask for exactly the
/// failure it means: unreadable (`list`) or unwritable (`upsert`).
struct FaultyArtifacts {
    listed: Vec<crate::ports::artifacts::ArtifactRecord>,
    list_fails: bool,
    upsert_fails: bool,
}

#[async_trait::async_trait]
impl crate::ports::artifacts::ArtifactStore for FaultyArtifacts {
    async fn list(
        &self,
        _: &CompanyId,
        _: Option<&str>,
    ) -> crate::Result<Vec<crate::ports::artifacts::ArtifactRecord>> {
        if self.list_fails {
            return Err(crate::error::OpenCompanyError::Store(
                "the artifact store is down".into(),
            ));
        }
        Ok(self.listed.clone())
    }
    async fn get(
        &self,
        _: &CompanyId,
        _: &str,
    ) -> crate::Result<Option<crate::ports::artifacts::ArtifactRecord>> {
        Ok(None)
    }
    async fn upsert(
        &self,
        _: &CompanyId,
        _: &crate::ports::artifacts::ArtifactRecord,
    ) -> crate::Result<()> {
        if self.upsert_fails {
            return Err(crate::error::OpenCompanyError::Store(
                "the disk is full".into(),
            ));
        }
        Ok(())
    }
    async fn delete(&self, _: &CompanyId, _: &str) -> crate::Result<bool> {
        Ok(false)
    }
}

/// [`state_with_company`] with the artifact store swapped for a faulty one, so
/// the workspace `PUT` can be exercised against a store that will not answer.
async fn state_with_faulty_artifacts(
    home: &std::path::Path,
    artifacts: FaultyArtifacts,
) -> (AppState, CompanyId) {
    let state = state_with_company(home).await;
    let company = state.registry().list()[0].clone();
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(company.clone())
        .with_artifacts(std::sync::Arc::new(artifacts))
        .build()
        .await
        .expect("runtime");
    // `insert` replaces, so the routes now resolve through the faulty store
    // while the seeded admin on `state` carries over untouched.
    state
        .registry()
        .insert(company.clone(), std::sync::Arc::new(runtime));
    (state, company)
}

/// Issue #552 made every note save consult the artifact store, and an ordinary
/// note must not inherit that store's health.
///
/// Nearly the whole tree is ordinary notes. They own no artifact chain, and
/// their save touches the artifact store for one reason only — to ask whether
/// they are a deliverable. When that question cannot be answered, refusing the
/// save would discard an operator's typing to protect a chain the note does not
/// have.
#[tokio::test]
async fn an_ordinary_note_still_saves_when_the_artifact_store_cannot_be_read() {
    use crate::ports::workspace::WorkspaceStore;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, company) = state_with_faulty_artifacts(
        &home,
        FaultyArtifacts {
            listed: Vec::new(),
            list_fails: true,
            upsert_fails: false,
        },
    )
    .await;
    let runtime = state.registry().get(&company).expect("company");

    let (_, note) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "notes.md", "kind": "file", "content": "just a note"})),
    )
    .await;
    let node_id = note["id"].as_str().expect("node id").to_string();

    let (status, _) = send(
        &state,
        "PUT",
        &format!("/api/v1/company/workspace/file/{node_id}"),
        Some(json!({"content": "the operator kept typing"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an unreadable artifact store must not reject a plain note's save"
    );

    let (_, body) = WorkspaceStore::read(runtime.workspace().as_ref(), &company, &node_id)
        .await
        .unwrap()
        .expect("the note still exists");
    assert_eq!(
        body, "the operator kept typing",
        "the edit must actually land, not merely report success"
    );
}

/// The other direction, and the one the availability fix must not have cost:
/// once the store *has* answered and named this node a published deliverable,
/// a version that cannot be recorded still refuses the save.
///
/// This is the fail-closed guarantee the module exists for. A node written
/// behind a version that was never appended is the silent, permanent direction
/// — `human_edit_diff` would answer for a draft the operator had already
/// rewritten.
#[tokio::test]
async fn a_published_note_refuses_the_save_when_its_version_cannot_be_recorded() {
    use crate::ports::artifacts::{ArtifactKind, ArtifactRecord};
    use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();

    // The store answers the lookup — this node IS a deliverable — but refuses
    // the append.
    let mut published = ArtifactRecord::new(
        "art-1",
        "t-1",
        "Launch spec",
        ArtifactKind::Markdown,
        "the agent's draft",
        "ceo",
        1,
    );
    published.stamp_workspace_node("node-published");
    let (state, company) = state_with_faulty_artifacts(
        &home,
        FaultyArtifacts {
            listed: vec![published],
            list_fails: false,
            upsert_fails: true,
        },
    )
    .await;
    let runtime = state.registry().get(&company).expect("company");

    // The node the artifact points at, created directly so its id is the one
    // the record was stamped with.
    WorkspaceStore::create(
        runtime.workspace().as_ref(),
        &company,
        &WorkspaceNode {
            id: "node-published".to_string(),
            name: "launch.md".to_string(),
            kind: NodeKind::File,
            parent_id: None,
            updated_at_millis: 1,
            created_by: WorkspaceOrigin::Operator,
            updated_by: WorkspaceOrigin::Operator,
            mime: None,
            size: None,
            sha256: None,
            adopted: false,
        },
        Some("the agent's draft"),
    )
    .await
    .expect("seed the node");

    let (status, _) = send(
        &state,
        "PUT",
        "/api/v1/company/workspace/file/node-published",
        Some(json!({"content": "the operator's rewrite"})),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a deliverable whose version cannot be recorded must not have its node written"
    );

    let (_, body) = WorkspaceStore::read(runtime.workspace().as_ref(), &company, "node-published")
        .await
        .unwrap()
        .expect("the note still exists");
    assert_eq!(
        body, "the agent's draft",
        "the node must be untouched — writing it would strand the chain behind it"
    );
}

// ---------------------------------------------------------------------------
// The plan → workflow bridge: apply / reject a proposal (issue #580)
// ---------------------------------------------------------------------------

/// Seeds a card sitting In Review with a `workflow` deliverable and the given
/// proposal graph, straight through the task store (the builder pass that would
/// normally mint it is behind the `openhuman` feature). Returns the card id.
async fn seed_proposal_card(state: &AppState, ops: Value) -> String {
    seed_proposal_card_assigned(state, ops, "ceo").await
}

/// [`seed_proposal_card`], with the assignee set to whatever the caller
/// passes rather than the hardcoded `"ceo"` — for proving the owning-desk
/// default against a card assigned directly to a desk (issue #1882 review),
/// where `assignee` is the desk's own canonical id rather than a teammate's.
async fn seed_proposal_card_assigned(state: &AppState, ops: Value, assignee: &str) -> String {
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("company");
    let id = crate::ports::generate_id();
    let record = TaskRecord {
        id: id.clone(),
        title: TaskTitle::authored("Automate the weekly digest"),
        note: None,
        column: "in_review".to_string(),
        priority: "medium".to_string(),
        assignee: assignee.to_string(),
        updated_at_millis: 1,
        origin: None,
        parent_task_id: None,
        output: None,
        plan: None,
        planning_attempts: Vec::new(),
        deliverable: crate::ports::tasks::TaskDeliverable::Workflow,
        workflow_proposal: Some(crate::ports::tasks::TaskWorkflowProposal {
            summary: "Email the digest".to_string(),
            ops,
            generated_at_millis: 1,
            run_id: "run-build-1".to_string(),
        }),
        origin_run_id: None,
        origin_workflow_id: None,
        origin_message_seq: None,
        bounced: None,
    };
    runtime
        .tasks()
        .upsert(runtime.id(), &record)
        .await
        .expect("seed the proposal card");
    id
}

/// A valid two-node graph (trigger → agent) whose agent names a real roster
/// teammate. `schedule` arms the trigger when `Some`.
fn digest_ops(schedule: Option<&str>) -> Value {
    let mut trigger = json!({ "id": "start", "kind": "trigger", "name": "Start" });
    if let Some(cron) = schedule {
        trigger["schedule"] = json!(cron);
    }
    json!({
        "id": "weekly-digest",
        "name": "Weekly digest",
        "description": "Email the weekly digest",
        "nodes": [
            trigger,
            { "id": "write", "kind": "agent", "name": "Draft it", "agent": "ceo" }
        ],
        "edges": [{ "from": "start", "to": "write" }]
    })
}

/// Applying a manual-trigger proposal creates the workflow, stamps the card's
/// output link to the build attempt, finishes the card in Done, and clears the
/// proposal — the whole happy path in one assertion set.
#[tokio::test]
async fn applying_a_proposal_creates_the_workflow_and_finishes_the_card() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let id = seed_proposal_card(&state, digest_ops(None)).await;

    let (status, card) = send(
        &state,
        "POST",
        &format!("/api/v1/company/tasks/{id}/workflow-proposal/apply"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{card}");
    // Done is reached — the create path is the human approval the epic requires.
    assert_eq!(card["column"], "done");
    // The proposal is consumed, and the card links to the workflow it created and
    // to the attempt that built it (issue #339).
    assert!(card.get("workflowProposal").is_none(), "{card}");
    assert_eq!(card["output"]["runId"], "run-build-1");
    assert_eq!(
        card["output"]["workflows"][0]["workflowId"],
        "weekly-digest"
    );
    assert_eq!(card["output"]["workflows"][0]["action"], "created");

    // The workflow now exists in the company's list — and, with no schedule, it
    // is armed (nothing to disarm).
    let (status, workflows) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    let created = workflows
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == "weekly-digest")
        .expect("the created workflow is listed");
    assert_eq!(created["enabled"], true, "a manual trigger is not disarmed");
}

/// Issue #1862 prerequisite: a proposal that names no `ownerDesk` defaults to
/// the proposing card's assignee's desk. `seed_proposal_card` assigns the
/// card to `ceo`, and `desk_manifest` seats `ceo` on the `engineering` desk —
/// so the created workflow must come out owned by `engineering` even though
/// `digest_ops` never mentions it.
///
/// This reads the default back off the persisted overlay TOML directly,
/// rather than the `GET …/workflows/{id}` response (which now also projects
/// `ownerDesk`, see `WorkflowGraph::owner_desk`) — pinning the actual stored
/// effect of the defaulting logic, independent of the read projection.
#[tokio::test]
async fn applying_a_proposal_defaults_the_owner_desk_from_the_assignees_desk() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, desk_manifest()).await;
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
        file.owner_desk.as_deref(),
        Some("engineering"),
        "the assignee's desk fills the omitted owner_desk"
    );
}

