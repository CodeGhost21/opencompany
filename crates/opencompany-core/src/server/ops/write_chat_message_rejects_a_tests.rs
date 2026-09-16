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

/// A folder is still refused — and now the refusal says so, rather than
/// claiming a node the operator is looking at is absent from the workspace.
#[tokio::test]
async fn chat_message_rejects_a_folder_attachment() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, folder) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({ "name": "designs", "kind": "folder" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{folder}");
    let node_id = folder["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({ "message": "folder attached", "attachments": [node_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("folder"),
        "the refusal must name the real reason, got: {error}"
    );

    let (status, history) = send(&state, "GET", "/api/v1/company/chat/history", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        history
            .as_array()
            .expect("history is a list")
            .iter()
            .all(|m| m["text"] != "folder attached"),
        "a refused attachment message still reached the transcript: {history}"
    );
}

/// The download half: the blob route serves a prose note's bytes exactly, as a
/// neutralised download — never inline, never under a type a caller chose — so
/// the chip an attached note renders has a working download behind it. A
/// folder and an unknown id still 404 identically.
#[tokio::test]
async fn workspace_blob_serves_a_prose_note_as_a_download() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let content = "# Plan\n\nShip it.\n";
    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({ "name": "plan.md", "kind": "file", "content": content })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let node_id = created["id"].as_str().unwrap().to_string();

    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/company/workspace/blob/{node_id}"))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    assert_eq!(
        headers["content-type"], "application/octet-stream",
        "a note is served under a neutral type, not one a caller influenced"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;"),
        "a note must never be served inline on the console's origin"
    );
    assert_eq!(headers["content-length"], content.len().to_string());
    assert!(
        !headers.contains_key("etag"),
        "a prose note has no stored digest to answer a conditional request with"
    );
    let got = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8(got.to_vec()).unwrap(),
        content,
        "the note's bytes must survive the round trip"
    );

    // A folder and an id naming nothing still 404 identically.
    let (status, folder) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({ "name": "archive", "kind": "folder" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{folder}");
    for id in [folder["id"].as_str().unwrap(), "01JZZZNOTAREALNODE00000000"] {
        let request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/company/workspace/blob/{id}"))
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{id} must not be servable as a blob"
        );
    }
}

/// A note past the extraction cap still **attaches** — the reference is what
/// the operator asked for — it simply carries no extracted text, the same
/// answer an oversized binary gets.
#[tokio::test]
async fn chat_attachment_oversized_note_attaches_without_extracted_text() {
    use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");

    // Written straight to the store: the JSON create route's body limit is far
    // below the extraction cap, so this size cannot arrive through it.
    let huge = "x".repeat(5 * 1024 * 1024);
    WorkspaceStore::create(
        runtime.workspace().as_ref(),
        &company,
        &WorkspaceNode {
            id: "node-oversized-note".to_string(),
            name: "huge.md".to_string(),
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
        Some(&huge),
    )
    .await
    .expect("seed the oversized note");

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({ "message": "big note", "attachments": ["node-oversized-note"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let journaled = runtime
        .events()
        .read_from(runtime.id(), crate::ports::types::EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .find_map(|s| match s.event {
            crate::ports::types::CompanyEvent::OperatorMessage { attachments, .. }
                if !attachments.is_empty() =>
            {
                Some(attachments)
            }
            _ => None,
        })
        .expect("the message with an attachment is in the journal");
    assert_eq!(journaled.len(), 1);
    assert_eq!(journaled[0].size, huge.len() as u64);
    assert_eq!(
        journaled[0].extracted_text, None,
        "a note past the extraction cap attaches with no text, rather than not at all"
    );
}

/// A workspace that records how many bytes each unbounded [`WorkspaceStore::read`]
/// handed back, per node.
///
/// Wraps the permissive in-memory double and sits **under** the runtime's own
/// decorators, so what it records is what the request actually pulled through
/// the whole production stack — including whether a decorator forwarded
/// `read_capped` or let it fall back to reading.
struct RecordingReads {
    inner: std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>,
    read_bytes: std::sync::Mutex<std::collections::HashMap<String, u64>>,
}

impl RecordingReads {
    fn new(inner: std::sync::Arc<dyn crate::ports::workspace::WorkspaceStore>) -> Self {
        Self {
            inner,
            read_bytes: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn bytes_read(&self, id: &str) -> u64 {
        self.read_bytes
            .lock()
            .unwrap()
            .get(id)
            .copied()
            .unwrap_or(0)
    }
}

#[async_trait::async_trait]
impl crate::ports::workspace::WorkspaceStore for RecordingReads {
    async fn admit_upload(&self, company: &CompanyId, name: &str, len: u64) -> crate::Result<()> {
        self.inner.admit_upload(company, name, len).await
    }

    async fn tree(
        &self,
        company: &CompanyId,
    ) -> crate::Result<Vec<crate::ports::workspace::WorkspaceNode>> {
        self.inner.tree(company).await
    }

    async fn read(
        &self,
        company: &CompanyId,
        id: &str,
    ) -> crate::Result<Option<(crate::ports::workspace::WorkspaceNode, String)>> {
        let got = self.inner.read(company, id).await?;
        if let Some((_, body)) = &got {
            *self
                .read_bytes
                .lock()
                .unwrap()
                .entry(id.to_string())
                .or_default() += body.len() as u64;
        }
        Ok(got)
    }

    async fn read_capped(
        &self,
        company: &CompanyId,
        id: &str,
        max_bytes: u64,
    ) -> crate::Result<Option<(crate::ports::workspace::WorkspaceNode, String, u64)>> {
        self.inner.read_capped(company, id, max_bytes).await
    }

    async fn write_with_revision(
        &self,
        company: &CompanyId,
        id: &str,
        content: &str,
        author: crate::ports::workspace::WorkspaceOrigin,
        expected_updated_at: Option<u64>,
    ) -> crate::Result<crate::ports::workspace::WorkspaceNode> {
        self.inner
            .write_with_revision(company, id, content, author, expected_updated_at)
            .await
    }

    async fn create(
        &self,
        company: &CompanyId,
        node: &crate::ports::workspace::WorkspaceNode,
        content: Option<&str>,
    ) -> crate::Result<()> {
        self.inner.create(company, node, content).await
    }

    async fn adopt_or_create_folder(
        &self,
        company: &CompanyId,
        parent: Option<&str>,
        name: &str,
        origin: crate::ports::workspace::WorkspaceOrigin,
    ) -> crate::Result<crate::ports::workspace::FolderClaim> {
        self.inner
            .adopt_or_create_folder(company, parent, name, origin)
            .await
    }

    async fn create_binary(
        &self,
        company: &CompanyId,
        node: &crate::ports::workspace::WorkspaceNode,
        bytes: &[u8],
    ) -> crate::Result<crate::ports::workspace::WorkspaceNode> {
        self.inner.create_binary(company, node, bytes).await
    }

    async fn write_binary(
        &self,
        company: &CompanyId,
        id: &str,
        bytes: &[u8],
        mime: Option<&str>,
        author: crate::ports::workspace::WorkspaceOrigin,
    ) -> crate::Result<crate::ports::workspace::WorkspaceNode> {
        self.inner
            .write_binary(company, id, bytes, mime, author)
            .await
    }

    async fn read_bytes(
        &self,
        company: &CompanyId,
        id: &str,
    ) -> crate::Result<
        Option<(
            crate::ports::workspace::WorkspaceNode,
            crate::ports::workspace::BlobStream,
        )>,
    > {
        self.inner.read_bytes(company, id).await
    }

    async fn rename_move(
        &self,
        company: &CompanyId,
        id: &str,
        name: Option<&str>,
        parent: Option<Option<&str>>,
    ) -> crate::Result<crate::ports::workspace::WorkspaceNode> {
        self.inner.rename_move(company, id, name, parent).await
    }

    async fn swap_files(
        &self,
        company: &CompanyId,
        expected_id: Option<&str>,
        replacement_id: &str,
        name: &str,
    ) -> crate::Result<Option<crate::ports::workspace::WorkspaceNode>> {
        self.inner
            .swap_files(company, expected_id, replacement_id, name)
            .await
    }

    async fn delete(&self, company: &CompanyId, id: &str) -> crate::Result<bool> {
        self.inner.delete(company, id).await
    }

    async fn is_empty(&self, company: &CompanyId) -> crate::Result<bool> {
        self.inner.is_empty(company).await
    }
}

/// The ceiling on a prose attachment holds at read time, not after.
///
/// The binary half of this path has never had to buffer what it will discard:
/// `size` rides the node, so an over-cap payload is refused on metadata and
/// `read_bytes` is never called. A note carries no `size`, so the same
/// discipline needs the store to answer the length and withhold the body in one
/// step — otherwise the cap is applied to a `String` that has already been
/// allocated, which is the allocation the cap exists to prevent, and one
/// message may carry twenty of them.
///
/// Recorded through the whole runtime stack, so a decorator that stopped
/// forwarding `read_capped` fails here too.
#[tokio::test]
async fn an_over_cap_note_attachment_is_never_fully_read() {
    use crate::ports::workspace::{NodeKind, WorkspaceNode, WorkspaceOrigin, WorkspaceStore};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let recorder = std::sync::Arc::new(RecordingReads::new(std::sync::Arc::new(
        crate::company::workspace_repair::loose_store::LooseWorkspace::default(),
    )));
    let state = state_with_workspace(&home, recorder.clone()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");

    let huge = "x".repeat(5 * 1024 * 1024);
    WorkspaceStore::create(
        runtime.workspace().as_ref(),
        &company,
        &WorkspaceNode {
            id: "node-oversized".to_string(),
            name: "huge.md".to_string(),
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
        Some(&huge),
    )
    .await
    .expect("seed the oversized note");
    let seeded = recorder.bytes_read("node-oversized");

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({ "message": "big note", "attachments": ["node-oversized"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(
        recorder.bytes_read("node-oversized") - seeded,
        0,
        "resolving an over-cap attachment must not pull the note's body through \
         the unbounded read"
    );

    // And it still attaches, with the length the store measured and no text.
    let journaled = runtime
        .events()
        .read_from(runtime.id(), crate::ports::types::EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .find_map(|s| match s.event {
            crate::ports::types::CompanyEvent::OperatorMessage { attachments, .. }
                if !attachments.is_empty() =>
            {
                Some(attachments)
            }
            _ => None,
        })
        .expect("the message with an attachment is in the journal");
    assert_eq!(journaled.len(), 1);
    assert_eq!(journaled[0].size, huge.len() as u64);
    assert_eq!(journaled[0].extracted_text, None);
}

/// A note under the cap is read once and reaches the brain whole — the bound
/// above must not be paid for by an attachment that fits.
#[tokio::test]
async fn an_under_cap_note_attachment_still_carries_its_text() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let recorder = std::sync::Arc::new(RecordingReads::new(std::sync::Arc::new(
        crate::company::workspace_repair::loose_store::LooseWorkspace::default(),
    )));
    let state = state_with_workspace(&home, recorder.clone()).await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).expect("company");

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({
            "name": "brief.md",
            "kind": "file",
            "content": "Q3 revenue grew 12% year over year.",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let node_id = created["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/chat",
        Some(json!({ "message": "small note", "attachments": [node_id] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let journaled = runtime
        .events()
        .read_from(runtime.id(), crate::ports::types::EventSeq::new(0), 10_000)
        .await
        .unwrap()
        .into_iter()
        .find_map(|s| match s.event {
            crate::ports::types::CompanyEvent::OperatorMessage { attachments, .. }
                if !attachments.is_empty() =>
            {
                Some(attachments)
            }
            _ => None,
        })
        .expect("the message with an attachment is in the journal");
    assert_eq!(
        journaled[0].extracted_text.as_deref(),
        Some("Q3 revenue grew 12% year over year."),
    );
    assert_eq!(
        journaled[0].size,
        "Q3 revenue grew 12% year over year.".len() as u64
    );
}
