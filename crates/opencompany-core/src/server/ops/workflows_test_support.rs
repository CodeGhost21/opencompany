//! Shared fixtures and helpers for the `workflows` hosted-mode test files.
//!
//! This module recovers the helpers that used to live once, at the top of
//! the original `mod tests { mod hosted_mode { ... } }` block, before that
//! block was split into sibling `workflows_hosted_*_tests.rs` files. Many of
//! those helpers are used by tests that now live in several different
//! files, so they are collected here instead of being copy-pasted anywhere
//! that still needs them.

use super::*;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

pub(super) fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-workflows-hosted-")
        .tempdir()
        .expect("tempdir")
}

/// A manifest declaring one enabled workflow — mirrors what a
/// platform tenant provisions with, minus any `workflows/` directory
/// on disk (there isn't one: hosted tenants have no source dir).
pub(super) fn manifest_with_enabled() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[workflows]\nenabled = [\"demo\"]\n",
    )
    .unwrap()
}

/// Builds a running company whose runtime has **no source directory**
/// (built without `with_seed_dir`, matching how the platform builds a
/// provisioned tenant) but whose persisted record declares an enabled
/// workflow — the exact hosted-mode gap #70 reports.
pub(super) async fn state_with_hosted_company(home: &std::path::Path) -> AppState {
    state_with_hosted_company_lifecycle(home, "running").await
}

/// The same fixture at a chosen lifecycle, so a paused company is
/// reachable without a second copy of the record literal.
pub(super) async fn state_with_hosted_company_lifecycle(
    home: &std::path::Path,
    lifecycle: &str,
) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: manifest_with_enabled(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest_with_enabled())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    assert!(
        runtime.source_dir().is_none(),
        "test setup must simulate hosted mode: no source dir"
    );
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

pub(super) fn empty_manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
}

pub(super) async fn hosted_state(
    home: &std::path::Path,
) -> (AppState, FsCompanyStore, CompanyId) {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: empty_manifest(),
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
    let state = state_over(home, &id, true).await;
    (state, store, id)
}

pub(super) async fn state_over(
    home: &std::path::Path,
    id: &CompanyId,
    seed_admin: bool,
) -> AppState {
    let runtime = RuntimeBuilder::new(home.to_path_buf(), empty_manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    assert!(
        runtime.source_dir().is_none(),
        "test setup must simulate hosted mode: no source dir"
    );
    let state = AppState::new(AppConfig::default());
    state
        .registry()
        .insert(id.clone(), std::sync::Arc::new(runtime));
    if seed_admin {
        crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    }
    state
}

pub(super) fn create_body() -> serde_json::Value {
    serde_json::json!({
        "id": "greeter",
        "name": "Greeter",
        "description": "Say hi.",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
    })
}

pub(super) fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", crate::server::test_support::fixed_cookie("acme"));
    match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&json).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

pub(super) async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

pub(super) async fn post_validate(
    state: &AppState,
    body: serde_json::Value,
) -> axum::response::Response {
    router(state.clone())
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/validate",
            Some(body),
        ))
        .await
        .unwrap()
}

pub(super) async fn post_create_on(
    state: &AppState,
    body: serde_json::Value,
) -> axum::response::Response {
    router(state.clone())
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap()
}

pub(super) fn body_with_an_unreachable_node() -> serde_json::Value {
    let mut body = create_body();
    body["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(
            { "id": "orphan", "kind": "output", "name": "Orphan" }
        ));
    body
}

pub(super) fn body_with_condition(label: &str, on_error: Option<&str>) -> serde_json::Value {
    let mut gate = serde_json::json!({
        "id": "gate",
        "kind": "condition",
        "name": "Gate",
        "config": { "field": "=item.approved" }
    });
    if let Some(on_error) = on_error {
        gate["onError"] = serde_json::Value::String(on_error.to_string());
    }
    serde_json::json!({
        "id": "greeter",
        "name": "Greeter",
        "description": "Say hi.",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            gate,
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [
            { "from": "start", "to": "gate" },
            { "from": "gate", "to": "done", "label": label }
        ]
    })
}

pub(super) async fn seeded_state(home: &std::path::Path) -> (AppState, tempfile::TempDir) {
    let source = tempfile::Builder::new()
        .prefix("oc-workflows-source-")
        .tempdir()
        .expect("tempdir");
    let workflows = source.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(
        workflows.join("child.toml"),
        "id = \"child\"\nname = \"Child\"\n[[node]]\nid = \"start\"\n\
         kind = \"trigger\"\nname = \"Start\"\n",
    )
    .unwrap();

    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            id: id.clone(),
            manifest: empty_manifest(),
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            overlay_retired_agents: Vec::new(),
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
            setup: Default::default(),
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        })
        .await
        .unwrap();
    let runtime = RuntimeBuilder::new(home.to_path_buf(), empty_manifest())
        .with_id(id.clone())
        .with_seed_dir(source.path())
        .build()
        .await
        .unwrap();
    assert!(
        runtime.source_dir().is_some(),
        "this fixture only proves anything with a source directory"
    );
    let state = AppState::new(AppConfig::default());
    state
        .registry()
        .insert(id.clone(), std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    (state, source)
}

pub(super) fn body_with_sub_workflow() -> serde_json::Value {
    serde_json::json!({
        "id": "parent",
        "name": "Parent",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start" },
            {
                "id": "child_run",
                "kind": "sub_workflow",
                "name": "Run the child",
                "config": { "workflow_id": "child" }
            }
        ],
        "edges": [ { "from": "start", "to": "child_run" } ]
    })
}

pub(super) fn desk_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n\
         [[group_chat]]\nid = \"engineering\"\nname = \"Engineering\"\nmembers = [\"ceo\"]\n",
    )
    .unwrap()
}

pub(super) async fn desk_state(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: id.clone(),
            manifest: desk_manifest(),
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), desk_manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    assert_eq!(
        runtime.deliverable_channel_ids(),
        vec!["operator".to_string(), "engineering".to_string()],
        "the fixture must have the operator channel plus exactly one desk channel, or \
         these tests prove nothing"
    );
    let state = AppState::new(AppConfig::default());
    state
        .registry()
        .insert(id.clone(), std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

pub(super) fn body_with_destination(kind: &str, target: Option<&str>) -> serde_json::Value {
    let mut destination = serde_json::json!({ "kind": kind });
    if let Some(target) = target {
        destination["target"] = serde_json::Value::String(target.to_string());
    }
    let mut body = create_body();
    body["nodes"][1]["destination"] = destination;
    body
}

pub(super) async fn post_create(
    state: AppState,
    body: serde_json::Value,
) -> axum::response::Response {
    router(state)
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap()
}

pub(super) fn body_with_postcondition() -> serde_json::Value {
    let mut body = create_body();
    body["nodes"][1]["kind"] = serde_json::json!("agent");
    body["nodes"][1]["agent"] = serde_json::json!("ceo");
    body["nodes"][1]["postcondition"] = serde_json::json!({ "require": "non_empty" });
    body
}

pub(super) async fn journal_run_with_id(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    error: &str,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowRunFinished {
                workflow_id: workflow_id.to_string(),
                scheduled: false,
                run_id: Some(run_id.to_string()),
                deliveries: Vec::new(),
                pending_approvals: Vec::new(),
                error: Some(error.to_string()),
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                // Added by #881/#880 after this fixture was written. A
                // failed run parks nothing and blocks nothing, so both
                // are empty here — see `Settled::from`'s Err arm, which
                // makes the same choice for the same reason.
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            },
        )
        .await
        .expect("append");
}

pub(super) fn scheduled_create_body() -> serde_json::Value {
    serde_json::json!({
        "id": "digest",
        "name": "Digest",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * *" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
    })
}

pub(super) async fn journal_run(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    scheduled: bool,
    deliveries: Vec<crate::ports::DeliveryReport>,
    error: Option<&str>,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowRunFinished {
                workflow_id: workflow_id.to_string(),
                scheduled,
                run_id: None,
                deliveries,
                pending_approvals: Vec::new(),
                error: error.map(str::to_string),
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            },
        )
        .await
        .expect("append");
}

pub(super) fn sent_row(node: &str) -> crate::ports::DeliveryReport {
    crate::ports::DeliveryReport {
        node: node.to_string(),
        kind: "owner".to_string(),
        target: Some("ada@example.com".to_string()),
        status: crate::ports::DeliveryStatus::Sent,
        detail: "emailed the company's admin".to_string(),
        reason: crate::ports::DeliveryReason::OwnerEmailed,
    }
}

pub(super) fn undelivered_row(node: &str) -> crate::ports::DeliveryReport {
    crate::ports::DeliveryReport {
        node: node.to_string(),
        kind: "email".to_string(),
        target: Some("ada@example.com".to_string()),
        status: crate::ports::DeliveryStatus::Skipped,
        detail: "this recipient has never written to the company".to_string(),
        reason: crate::ports::DeliveryReason::RecipientNotEstablished,
    }
}

pub(super) fn run_card(
    id: &str,
    title: &str,
    origin_run_id: Option<&str>,
) -> crate::ports::TaskRecord {
    crate::ports::TaskRecord {
        id: id.into(),
        title: crate::ports::tasks::TaskTitle::authored(title),
        note: None,
        column: "in_review".into(),
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
        origin_run_id: origin_run_id.map(str::to_string),
        origin_workflow_id: None,
        origin_message_seq: None,
        bounced: None,
    }
}

pub(super) fn published(
    id: &str,
    task_id: &str,
    title: &str,
    source: &str,
    at_millis: u64,
) -> crate::ports::ArtifactRecord {
    let mut rec = crate::ports::ArtifactRecord::new(
        id,
        task_id,
        title,
        crate::ports::ArtifactKind::Markdown,
        "the agent's draft",
        "ceo",
        at_millis,
    )
    .with_source(source);
    rec.updated_at_millis = at_millis;
    rec
}

pub(super) async fn journal_start(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    scheduled: bool,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowRunStarted {
                workflow_id: workflow_id.to_string(),
                run_id: run_id.to_string(),
                scheduled,
                started_by: None,
                resume_semantic: None,
            },
        )
        .await
        .expect("append");
}

pub(super) async fn journal_node(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    node_id: &str,
    status: WorkflowNodeStatus,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowNodeFinished {
                workflow_id: workflow_id.to_string(),
                run_id: run_id.to_string(),
                node_id: node_id.to_string(),
                status,
                elapsed_ms: 42,
                diagnostics: Vec::new(),
                agent_run_id: None,
            },
        )
        .await
        .expect("append");
}

pub(super) async fn journal_node_started(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    node_id: &str,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowNodeStarted {
                workflow_id: workflow_id.to_string(),
                run_id: run_id.to_string(),
                node_id: node_id.to_string(),
            },
        )
        .await
        .expect("append");
}

pub(super) async fn journal_finish(
    state: &AppState,
    id: &CompanyId,
    workflow_id: &str,
    run_id: &str,
    scheduled: bool,
    error: Option<&str>,
) {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .append(
            id,
            CompanyEvent::WorkflowRunFinished {
                workflow_id: workflow_id.to_string(),
                scheduled,
                run_id: Some(run_id.to_string()),
                deliveries: Vec::new(),
                pending_approvals: Vec::new(),
                error: error.map(str::to_string),
                cancelled: false,
                notices: Vec::new(),
                board: Vec::new(),
                blocked_nodes: Vec::new(),
                approvals: Vec::new(),
            },
        )
        .await
        .expect("append");
}

pub(super) struct FinishesDuringTheRead {
    pub(super) inner: std::sync::Arc<dyn crate::ports::EventLog>,
    pub(super) finish: std::sync::Mutex<Option<(CompanyId, CompanyEvent)>>,
}

impl crate::ports::EventLog for FinishesDuringTheRead {
    async fn append(
        &self,
        id: &CompanyId,
        event: CompanyEvent,
    ) -> crate::Result<crate::ports::types::EventSeq> {
        self.inner.append(id, event).await
    }

    async fn read_from(
        &self,
        id: &CompanyId,
        seq: crate::ports::types::EventSeq,
        limit: usize,
    ) -> crate::Result<Vec<crate::ports::types::StoredEvent>> {
        let snapshot = self.inner.read_from(id, seq, limit).await?;
        // Taken out under the lock, so the append happens once however
        // many readers race here.
        let pending = self.finish.lock().expect("poisoned").take();
        if let Some((company, event)) = pending {
            self.inner.append(&company, event).await?;
        }
        Ok(snapshot)
    }

    fn subscribe(
        &self,
        id: &CompanyId,
    ) -> futures::stream::BoxStream<'static, crate::ports::events::EventStreamItem> {
        self.inner.subscribe(id)
    }
}

pub(super) fn page_run(seq: u64, at_millis: u64) -> WorkflowRunOutcome {
    WorkflowRunOutcome {
        seq,
        at_millis,
        workflow_id: "wf".to_string(),
        scheduled: false,
        run_id: Some(format!("run-{seq}")),
        resume_semantic: None,
        deliveries: Vec::new(),
        pending_approvals: Vec::new(),
        error: None,
        nodes: Vec::new(),
        started_nodes: Vec::new(),
        started_at_millis: Some(at_millis),
        running: false,
        cancelled: false,
        notices: Vec::new(),
        board: Vec::new(),
        blocked_nodes: Vec::new(),
        approvals: Vec::new(),
        degraded: false,
        stranded_approvals: 0,
        verdict: WorkflowRunVerdict::Ok,
    }
}

pub(super) fn page(
    journal: &[(u64, u64)],
    before_seq: Option<u64>,
    limit: usize,
) -> (Vec<WorkflowRunOutcome>, bool, Option<u64>) {
    let candidates: Vec<WorkflowRunOutcome> = journal
        .iter()
        .filter(|(seq, _)| before_seq.is_none_or(|bound| *seq < bound))
        .map(|(seq, at_millis)| page_run(*seq, *at_millis))
        .collect();
    select_run_page(candidates, limit)
}

pub(super) const REGRESSED: [(u64, u64); 5] = [
    (10, 1_000),
    (20, 2_000),
    (30, 3_000),
    // NTP correction / VM resume / an operator setting the date: the
    // append order is unchanged, the timestamp goes backwards.
    (40, 1_500),
    (50, 5_000),
];

pub(super) const AFTER: u64 = 1_785_672_000_000;

pub(super) async fn preview(state: &AppState, expr: &str) -> serde_json::Value {
    json_body(
        router(state.clone())
            .oneshot(request(
                "POST",
                "/api/v1/company/workflows/cron/preview",
                Some(serde_json::json!({ "expr": expr, "after": AFTER })),
            ))
            .await
            .unwrap(),
    )
    .await
}

pub(super) async fn create_greeter(state: &AppState) -> String {
    let response = router(state.clone())
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows",
            Some(create_body()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let created = json_body(response).await;
    // A freshly created overlay graph is editable and carries a token.
    assert_eq!(created["editable"], true, "{created}");
    created["version"]
        .as_str()
        .unwrap_or_else(|| panic!("create must return a version token: {created}"))
        .to_string()
}

pub(super) fn edited_body(expected_version: Option<&str>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "id": "greeter",
        "name": "Greeter",
        "description": "Say hi, every morning.",
        "nodes": [
            { "id": "start", "kind": "trigger", "name": "Start", "schedule": "0 9 * * *" },
            { "id": "done", "kind": "output", "name": "Report" }
        ],
        "edges": [ { "from": "start", "to": "done", "label": "ok" } ]
    });
    if let Some(v) = expected_version {
        body["expectedVersion"] = serde_json::json!(v);
    }
    body
}

pub(super) async fn create_then_edit_greeter(state: &AppState) -> String {
    let version = create_greeter(state).await;
    let response = router(state.clone())
        .oneshot(request(
            "PUT",
            "/api/v1/company/workflows/greeter",
            Some(edited_body(Some(&version))),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["version"]
        .as_str()
        .expect("new token")
        .to_string()
}

pub(super) async fn finishes_for(
    state: &AppState,
    id: &CompanyId,
    run_id: &str,
) -> Vec<(Option<String>, bool)> {
    let runtime = state.registry().get(id).expect("registered");
    runtime
        .events()
        .read_from(id, crate::ports::types::EventSeq::new(0), usize::MAX)
        .await
        .expect("read")
        .into_iter()
        .filter_map(|s| match s.event {
            CompanyEvent::WorkflowRunFinished {
                run_id: Some(rid),
                error,
                cancelled,
                ..
            } if rid == run_id => Some((error, cancelled)),
            _ => None,
        })
        .collect()
}
