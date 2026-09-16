fn own_rows(listed: &serde_json::Value) -> Vec<&serde_json::Value> {
    listed
        .as_array()
        .expect("array response")
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            !crate::globals::workflows().iter().any(|w| w.id == id)
        })
        .collect()
}

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::{
    CompanyEvent, DEFAULT_RUN_LIMIT, MAX_RUN_ARTIFACTS, WorkflowNodeStatus, WorkflowRunOutcome,
    WorkflowRunVerdict, select_run_page,
};
use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-workflows-hosted-")
        .tempdir()
        .expect("tempdir")
}

/// A manifest declaring one enabled workflow — mirrors what a
/// platform tenant provisions with, minus any `workflows/` directory
/// on disk (there isn't one: hosted tenants have no source dir).
fn manifest_with_enabled() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n[workflows]\nenabled = [\"demo\"]\n",
    )
    .unwrap()
}

/// Builds a running company whose runtime has **no source directory**
/// (built without `with_seed_dir`, matching how the platform builds a
/// provisioned tenant) but whose persisted record declares an enabled
/// workflow — the exact hosted-mode gap #70 reports.
async fn state_with_hosted_company(home: &std::path::Path) -> AppState {
    state_with_hosted_company_lifecycle(home, "running").await
}

/// The same fixture at a chosen lifecycle, so a paused company is
/// reachable without a second copy of the record literal.
async fn state_with_hosted_company_lifecycle(home: &std::path::Path, lifecycle: &str) -> AppState {
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

/// Builds a board card, optionally stamped with the run that opened it
/// (`origin_run_id`) — the field [`run_artifacts`] joins on (issue
/// #1684). Everything else is the neutral shape the board's own tests
/// use.
fn run_card(id: &str, title: &str, origin_run_id: Option<&str>) -> crate::ports::TaskRecord {
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

/// A published artifact on `task_id`: one agent version, a `source`
/// path, stamped `updated_at_millis` so the newest-first sort is
/// testable.
fn published(
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

/// The join: a run's files are the artifacts of every card it opened,
/// and only those. Two cards stamped with the run (one carrying two
/// files, one carrying one) contribute three rows; a card of a
/// *different* run and a chat card with no `origin_run_id` contribute
/// none. Newest-`updatedAtMillis` first.
#[tokio::test]
async fn run_artifacts_joins_cards_by_origin_run_id() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");

    for card in [
        run_card("t-a", "Launch spec", Some("run-1")),
        run_card("t-b", "Press kit", Some("run-1")),
        run_card("t-c", "Someone else", Some("run-2")),
        run_card("t-chat", "A chat card", None),
    ] {
        runtime.tasks().upsert(&id, &card).await.expect("seed card");
    }
    for art in [
        published("art-a1", "t-a", "Spec v1", "specs/launch.md", 30),
        published("art-a2", "t-a", "Spec appendix", "specs/appendix.md", 10),
        published("art-b1", "t-b", "Press release", "press/release.md", 20),
        // Belongs to run-2's card — must NOT appear under run-1.
        published("art-c1", "t-c", "Other run's file", "other.md", 99),
        // Belongs to a chat card with no origin run — must NOT appear.
        published("art-chat", "t-chat", "Chat reply", "chat.md", 99),
    ] {
        crate::ports::ArtifactStore::upsert(runtime.artifacts().as_ref(), &id, &art)
            .await
            .expect("seed artifact");
    }

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-1/artifacts",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let rows = body["files"].as_array().expect("files array");
    assert_eq!(rows.len(), 3, "only run-1's cards' files: {body}");

    // Newest-first: art-a1 (30), art-b1 (20), art-a2 (10).
    assert_eq!(rows[0]["artifactId"], "art-a1", "{body}");
    assert_eq!(rows[0]["taskId"], "t-a");
    assert_eq!(rows[0]["taskTitle"], "Launch spec");
    assert_eq!(rows[0]["title"], "Spec v1");
    assert_eq!(rows[0]["kind"], "markdown");
    assert_eq!(rows[0]["source"], "specs/launch.md");
    assert_eq!(rows[0]["latestVersion"], 1);
    assert_eq!(rows[1]["artifactId"], "art-b1", "{body}");
    assert_eq!(rows[2]["artifactId"], "art-a2", "{body}");

    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r["artifactId"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&"art-c1") && !ids.contains(&"art-chat"),
        "another run's and a chat card's files must not appear: {body}"
    );
}

/// The contract difference from `output`'s 404: a run that opened no
/// cards (or whose cards published nothing) answers `200 { files: [] }`,
/// not a 404. A fileless run is normal, not an error.
#[tokio::test]
async fn run_artifacts_empty_run_returns_200_empty() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, _id) = hosted_state(&home).await;

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-nothing/artifacts",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a run with no files is 200, never 404"
    );
    let body = json_body(response).await;
    assert_eq!(
        body["files"].as_array().expect("files array").len(),
        0,
        "{body}"
    );
}

/// `latestVersion` pins the newest revision, so the deep-link opens the
/// version an operator edit produced, not v1. A v1(agent)+v2(operator)
/// artifact reports `latestVersion == 2`.
#[tokio::test]
async fn run_artifacts_latest_version_is_pinned() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");

    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-1")))
        .await
        .expect("seed card");
    let mut art = published("art-a1", "t-a", "Spec", "specs/launch.md", 5);
    art.push_version(
        "the operator's rewrite",
        crate::ports::artifacts::ArtifactAuthor::Operator,
        "ceo",
        6,
        Some("operator edit before approval".to_string()),
    );
    crate::ports::ArtifactStore::upsert(runtime.artifacts().as_ref(), &id, &art)
        .await
        .expect("seed artifact");

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-1/artifacts",
            None,
        ))
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(body["files"][0]["latestVersion"], 2, "{body}");
    assert_eq!(body["files"][0]["updatedAtMillis"], 6, "{body}");
}

/// `workspaceNodeId` rides through from the newest revision when the
/// file was mirrored into the tree (issue #552), so the console can
/// offer the `#/workspace/<id>` link — and is absent otherwise.
#[tokio::test]
async fn run_artifacts_carries_workspace_node_when_mirrored() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");

    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-1")))
        .await
        .expect("seed card");
    let mut mirrored = published("art-a1", "t-a", "Spec", "specs/launch.md", 30);
    mirrored.stamp_workspace_node("node-9");
    let bare = published("art-a2", "t-a", "Note", "specs/note.md", 10);
    for art in [&mirrored, &bare] {
        crate::ports::ArtifactStore::upsert(runtime.artifacts().as_ref(), &id, art)
            .await
            .expect("seed artifact");
    }

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-1/artifacts",
            None,
        ))
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(body["files"][0]["artifactId"], "art-a1", "{body}");
    assert_eq!(body["files"][0]["workspaceNodeId"], "node-9", "{body}");
    assert!(
        body["files"][1]["workspaceNodeId"].is_null(),
        "an unmirrored file omits the workspace link: {body}"
    );
}

/// A card opened by run A and re-owned by run B keeps `origin_run_id ==
/// A` (the field is stamped once, at creation), so its files list under
/// A — the OPENING run — and not under B. Documents the deliberate
/// provenance choice.
#[tokio::test]
async fn run_artifacts_reowned_card_lists_under_opening_run() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");

    // Card was opened by run-A; a later run-B re-owned it but the field
    // still records the opener.
    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-A")))
        .await
        .expect("seed card");
    crate::ports::ArtifactStore::upsert(
        runtime.artifacts().as_ref(),
        &id,
        &published("art-a1", "t-a", "Spec", "specs/launch.md", 30),
    )
    .await
    .expect("seed artifact");

    // Under the opener: present.
    let under_a = json_body(
        router(state.clone())
            .oneshot(request(
                "GET",
                "/api/v1/company/workflows/runs/run-A/artifacts",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(under_a["files"].as_array().unwrap().len(), 1, "{under_a}");
    assert_eq!(under_a["files"][0]["artifactId"], "art-a1");

    // Under the re-owner: nothing — provenance is the opening run.
    let under_b = json_body(
        router(state)
            .oneshot(request(
                "GET",
                "/api/v1/company/workflows/runs/run-B/artifacts",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        under_b["files"].as_array().unwrap().len(),
        0,
        "a re-owner does not inherit the file: {under_b}"
    );
}

/// A legacy record with `source == None` (a pre-#244 auto-captured chat
/// reply) is still returned — with `source` absent — so the console can
/// label it rather than the history silently dropping it.
#[tokio::test]
async fn run_artifacts_includes_legacy_source_none_labeled() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");

    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-1")))
        .await
        .expect("seed card");
    // No `.with_source(..)` — a legacy record.
    let legacy = crate::ports::ArtifactRecord::new(
        "art-legacy",
        "t-a",
        "Auto-captured reply",
        crate::ports::ArtifactKind::Text,
        "an old chat reply",
        "assistant",
        12,
    );
    crate::ports::ArtifactStore::upsert(runtime.artifacts().as_ref(), &id, &legacy)
        .await
        .expect("seed artifact");

    let body = json_body(
        router(state)
            .oneshot(request(
                "GET",
                "/api/v1/company/workflows/runs/run-1/artifacts",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(body["files"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["files"][0]["artifactId"], "art-legacy");
    assert!(
        body["files"][0]["source"].is_null(),
        "a legacy record's absent source stays absent on the wire: {body}"
    );
}

/// The route resolves to `run_artifacts`, not the dynamic
/// `/workflows/{wid}` graph read — the static-before-dynamic slot holds
/// (mirrors `run_history_is_not_shadowed_by_the_graph_read`).
#[tokio::test]
async fn run_artifacts_route_is_not_shadowed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");
    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-1")))
        .await
        .expect("seed card");
    crate::ports::ArtifactStore::upsert(
        runtime.artifacts().as_ref(),
        &id,
        &published("art-a1", "t-a", "Spec", "specs/launch.md", 30),
    )
    .await
    .expect("seed artifact");

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-1/artifacts",
            None,
        ))
        .await
        .unwrap();
    // A graph read would 404 an unknown `wid`; this returns the files
    // array, proving the four-segment static route won.
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["files"][0]["artifactId"], "art-a1", "{body}");
}

/// A run whose file count passes [`MAX_RUN_ARTIFACTS`] reports
/// `truncated: true` instead of silently dropping the older rows — so
/// the console can label the list "newest 500 shown" rather than
/// presenting it as exhaustive. The newest rows survive the cut.
#[tokio::test]
async fn run_artifacts_exposes_truncation_at_the_cap() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;
    let runtime = state.registry().get(&id).expect("registered");
    runtime
        .tasks()
        .upsert(&id, &run_card("t-a", "Launch spec", Some("run-1")))
        .await
        .expect("seed card");
    // One past the cap: 501 files, `at_millis` increasing with the id
    // so the newest (art-500) is what the cut must keep.
    for i in 0..=MAX_RUN_ARTIFACTS {
        crate::ports::ArtifactStore::upsert(
            runtime.artifacts().as_ref(),
            &id,
            &published(
                &format!("art-{i:03}"),
                "t-a",
                &format!("File {i}"),
                &format!("files/{i}.md"),
                i as u64,
            ),
        )
        .await
        .expect("seed artifact");
    }

    let response = router(state)
        .oneshot(request(
            "GET",
            "/api/v1/company/workflows/runs/run-1/artifacts",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let files = body["files"].as_array().expect("files array");
    assert_eq!(files.len(), MAX_RUN_ARTIFACTS, "{body}");
    assert_eq!(body["truncated"], true, "{body}");
    assert_eq!(
        files[0]["artifactId"], "art-500",
        "the newest file survives the cut: {body}"
    );
    assert!(
        files.iter().all(|f| f["artifactId"] != "art-000"),
        "the oldest file is what the cap drops: {body}"
    );
}

/// A run that died outright reads back with its reason. This is the
/// outcome that previously left nothing behind but a host-stdout warning.
#[tokio::test]
async fn run_history_carries_a_failed_run_error() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let (state, _store, id) = hosted_state(&home).await;

    journal_run(
        &state,
        &id,
        "digest",
        true,
        Vec::new(),
        Some("no inference source for agent node `worker`"),
    )
    .await;

    let response = router(state)
        .oneshot(request("GET", "/api/v1/company/workflows/runs", None))
        .await
        .unwrap();
    let body = json_body(response).await;
    assert_eq!(
        body["runs"][0]["error"],
        "no inference source for agent node `worker`"
    );
    assert_eq!(body["runs"][0]["deliveries"].as_array().unwrap().len(), 0);
}

// ── Issue #371: the per-node progress fold ─────────────────────────

/// Journals a `WorkflowRunStarted`, the way the runner does before the
/// engine call.
async fn journal_start(
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

/// Journals one `WorkflowNodeFinished`, the way the run observer does.
async fn journal_node(
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

/// Journals one `WorkflowNodeStarted`, the way the run observer does
/// immediately before a node's first attempt (issue #382).
async fn journal_node_started(
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

/// Journals a finished outcome carrying a run id, the way every entry
/// point does post-#371.
async fn journal_finish(
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
