//! Shared fixtures and helpers for the `ops` write-plane test files split out
//! of the original `write_test.rs` (issue tracked by the mechanical test
//! split). Every sibling `write_*_tests.rs` / `ops_put_*_tests.rs` module
//! pulls these in with `use super::write_test_support::*;`.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::tasks::{TaskRecord, TaskTitle};
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

pub(super) fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("opencompany-ops-")
        .tempdir()
        .expect("tempdir")
}

pub(super) fn manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n",
    )
    .unwrap()
}

pub(super) fn provisioned_names(tree: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = tree
        .as_array()
        .expect("the tree read is an array")
        .iter()
        .map(|node| node["name"].as_str().unwrap_or_default().to_string())
        .collect();
    names.sort();
    names
}

pub(super) async fn state_with_company(home: &std::path::Path) -> AppState {
    state_with_quota(home, crate::runtime::WorkspaceQuota::default()).await
}

pub(super) async fn state_with_quota(
    home: &std::path::Path,
    quota: crate::runtime::WorkspaceQuota,
) -> AppState {
    state_with(home, quota, None).await
}

pub(super) async fn state_with_workspace(
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

pub(super) async fn state_with(
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

pub(super) fn repo_skills_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../companies")
}

pub(super) async fn state_with_registry(home: &std::path::Path) -> AppState {
    // `with_skills_root` consumes and returns the state, so the registered
    // company and seeded admin move along with it.
    state_with_company(home)
        .await
        .with_skills_root(repo_skills_root())
}

pub(super) async fn persisted_skills(
    state: &AppState,
) -> Vec<crate::ports::skills_state::SkillState> {
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("company");
    runtime.skills().list(runtime.id()).await.expect("deltas")
}

pub(super) async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_auth(state, method, uri, body, None).await
}

pub(super) async fn send_auth(
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

pub(super) fn swept_names(list: &serde_json::Value) -> Vec<String> {
    let mut names: Vec<String> = list
        .as_array()
        .unwrap_or_else(|| panic!("the sweep answers with a list, got {list}"))
        .iter()
        .map(|folder| folder["name"].as_str().unwrap_or_default().to_string())
        .collect();
    names.sort();
    names
}

pub(super) async fn workspace_changes(
    runtime: &std::sync::Arc<crate::runtime::CompanyRuntime>,
) -> Vec<(String, String)> {
    use crate::ports::types::CompanyEvent;
    runtime
        .events()
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.event {
            CompanyEvent::WorkspaceChanged { node_id, change } => Some((node_id, change)),
            _ => None,
        })
        .collect()
}

pub(super) async fn journal_len(
    runtime: &std::sync::Arc<crate::company::runtime::CompanyRuntime>,
) -> usize {
    runtime
        .events()
        .read_from(
            runtime.id(),
            crate::ports::types::EventSeq::new(0),
            usize::MAX,
        )
        .await
        .unwrap()
        .len()
}

pub(super) async fn append_mail(
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

pub(super) fn mcp_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n[[mcp_server]]\nname = \"docs\"\nendpoint = \"https://docs.example/mcp\"\n",
    )
    .unwrap()
}

pub(super) async fn state_with_manifest(
    home: &std::path::Path,
    manifest: CompanyManifest,
) -> AppState {
    state_with_manifest_and_defaults(home, manifest, Vec::new()).await
}

pub(super) async fn state_with_manifest_and_defaults(
    home: &std::path::Path,
    manifest: CompanyManifest,
    defaults: Vec<crate::company::McpServer>,
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
        .with_default_mcp_servers(defaults)
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

pub(super) async fn state_with_manifest_and_overlays(
    home: &std::path::Path,
    manifest: CompanyManifest,
    overlay_agents: Vec<crate::ports::types::OverlayAgent>,
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
            manifest: manifest.clone(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents,
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
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

pub(super) async fn state_with_source_dir(
    home: &std::path::Path,
    seed_dir: &std::path::Path,
    manifest: CompanyManifest,
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
        .with_seed_dir(seed_dir.to_path_buf())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

pub(super) fn workflow_body(id: &str) -> Value {
    json!({
        "id": id,
        "name": id,
        "description": "A tiny test graph.",
        "nodes": [
            {"id": "start", "kind": "trigger", "name": "Start"},
            {"id": "worker", "kind": "agent", "name": "Worker", "agent": "ceo"},
            {"id": "done", "kind": "output", "name": "Done"},
        ],
        "edges": [
            {"from": "start", "to": "worker"},
            {"from": "worker", "to": "done", "label": "ok"},
        ],
    })
}

pub(super) fn discussion_card(id: &str, title: &str) -> TaskRecord {
    TaskRecord {
        id: id.into(),
        title: TaskTitle::authored(title),
        note: None,
        column: "todo".into(),
        priority: "medium".into(),
        assignee: "ceo".into(),
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
    }
}

pub(super) async fn dispatched_task(
    state: &AppState,
    company: &CompanyId,
) -> (std::sync::Arc<crate::CompanyRuntime>, u64) {
    use crate::ports::types::CompanyEvent;

    let runtime = state.registry().get(company).unwrap();
    runtime
        .tasks()
        .upsert(
            company,
            &TaskRecord {
                id: "t-1".into(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: "in_progress".into(),
                priority: "medium".into(),
                assignee: "ceo".into(),
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
    runtime
        .events()
        .append(
            company,
            CompanyEvent::TaskDispatched {
                task_id: "t-1".into(),
                run_id: None,
            },
        )
        .await
        .unwrap();
    let dispatched_at = runtime
        .events()
        .read_from(company, crate::ports::types::EventSeq::new(0), 64)
        .await
        .unwrap()
        .last()
        .unwrap()
        .at_millis;
    (runtime, dispatched_at)
}

pub(super) fn parked_effect() -> crate::ports::types::Effect {
    use crate::ports::types::{Effect, EffectGroup};
    Effect {
        kind: "filing.submit".into(),
        group: EffectGroup::Sign,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::Value::Null,
        agent: None,
        run_id: None,
    }
}

pub(super) fn only_approval(body: &Value) -> Value {
    let rows: Vec<Value> = body["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "approval")
        .cloned()
        .collect();
    assert_eq!(rows.len(), 1, "expected exactly one approval row: {rows:?}");
    rows[0].clone()
}

pub(super) async fn send_cookie(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: &str,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
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


pub(super) async fn seed_proposal_card(state: &AppState, ops: Value) -> String {
    seed_proposal_card_assigned(state, ops, "ceo").await
}

pub(super) async fn seed_proposal_card_assigned(
    state: &AppState,
    ops: Value,
    assignee: &str,
) -> String {
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

pub(super) fn digest_ops(schedule: Option<&str>) -> Value {
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

pub(super) fn desk_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"engineering\"\nname = \"Engineering\"\nmembers = [\"ceo\"]\n\
         [policy]\nmode = \"full\"\n",
    )
    .unwrap()
}

pub(super) fn digest_ops_posting_to(target: &str) -> Value {
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

pub(super) async fn upload_file(
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

pub(super) async fn blob_response(state: &AppState, id: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/company/workspace/blob/{id}"))
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap();
    router(state.clone()).oneshot(request).await.unwrap()
}

pub(super) fn assert_not_executable(response: &axum::response::Response, context: &str) {
    let content_type = response.headers()["content-type"].to_str().unwrap();
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    let nosniff = response
        .headers()
        .get("x-content-type-options")
        .map(|v| v.to_str().unwrap().to_string());

    assert!(
        disposition.starts_with("attachment;"),
        "{context}: a browser must download this, not render it — got {disposition:?}"
    );
    assert_eq!(
        nosniff.as_deref(),
        Some("nosniff"),
        "{context}: without nosniff the type below is a suggestion"
    );
    for executable in [
        "text/html",
        "image/svg+xml",
        "application/xhtml+xml",
        "text/xml",
    ] {
        assert!(
            !content_type.starts_with(executable),
            "{context}: served as {content_type:?}, which a browser parses into a \
             document with a script context"
        );
    }
}

pub(super) const OVERSIZE_BOUNDARY: &str = "----opencompany647boundary";

pub(super) async fn post_upload(state: &AppState, body: Body) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/company/workspace/upload")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={OVERSIZE_BOUNDARY}"),
        )
        .body(body)
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let out = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value = serde_json::from_slice(&out).unwrap_or(Value::Null);
    (status, value)
}

pub(super) fn streamed_multipart(prefix: Vec<u8>, payload: usize, suffix: Vec<u8>) -> Body {
    const FRAME: usize = 1024 * 1024;
    let prefix = std::sync::Arc::new(prefix);
    let suffix = std::sync::Arc::new(suffix);
    let filler = bytes::Bytes::from(vec![0u8; FRAME]);
    let frames = payload.div_ceil(FRAME);

    let stream = futures::stream::unfold(0usize, move |step| {
        let prefix = prefix.clone();
        let suffix = suffix.clone();
        let filler = filler.clone();
        async move {
            tokio::task::yield_now().await;
            let frame = if step == 0 {
                bytes::Bytes::from(prefix.as_ref().clone())
            } else if step <= frames {
                filler.slice(..(payload - (step - 1) * FRAME).min(FRAME))
            } else if step == frames + 1 {
                bytes::Bytes::from(suffix.as_ref().clone())
            } else {
                return None;
            };
            Some((Ok::<_, std::io::Error>(frame), step + 1))
        }
    });
    Body::from_stream(stream)
}

pub(super) fn file_part_prefix(filename: &str) -> Vec<u8> {
    format!(
        "--{OVERSIZE_BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes()
}

pub(super) async fn tree_names(state: &AppState) -> Vec<String> {
    let (status, tree) = send(state, "GET", "/api/v1/company/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    provisioned_names(&tree)
}

pub(super) fn text_file_part_prefix(filename: &str) -> Vec<u8> {
    format!(
        "--{OVERSIZE_BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n"
    )
    .into_bytes()
}

pub(super) async fn detail_as(state: &AppState, id: &str, cookie: String) -> Value {
    let request = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/company/tasks/{id}"))
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub(super) async fn chat_upload(
    state: &AppState,
    filename: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> (StatusCode, Value) {
    const BOUNDARY: &str = "----opencompany1682boundary";
    let mut body: Vec<u8> = Vec::new();
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
        .uri("/api/v1/company/chat/upload")
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

