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

/// The Brain's "Last updated" stat must move when *agents* write memory, not
/// only when the operator hand-authors a fact.
///
/// The reported bug (#153): agent memory and task outcomes land exclusively in
/// the `ContextStore`, and the stat was computed from the `FactStore` alone —
/// so a company whose agents were actively remembering, but whose operator had
/// never added a fact, showed "—" forever.
#[tokio::test]
async fn memory_stats_last_updated_covers_agent_written_context() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // A brand-new company remembers nothing: the stat is genuinely empty, and
    // "—" is the honest rendering.
    let (status, stats) = send(&state, "GET", "/api/v1/company/memory/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["facts"], 0);
    assert_eq!(stats["totalItems"], 0);
    assert_eq!(stats["teammateMemory"], 0);
    assert_eq!(
        stats["lastUpdatedAtMillis"], 0,
        "no memory of any kind yet, so the stat has nothing to report"
    );

    // Now an agent writes memory — no operator fact anywhere in sight. This is
    // the exact state that used to pin the stat at 0.
    let before = crate::ports::now_millis();
    for (label, body) in [
        ("agent-ceo/notes", "the launch slipped to Friday"),
        ("task-outcome/agent-ceo", "Task: ship it\nOutcome: done"),
    ] {
        runtime
            .context
            .put(
                runtime.id(),
                ContextChunk {
                    label: label.to_string(),
                    body: body.to_string(),
                },
            )
            .await
            .unwrap();
    }

    let (status, stats) = send(&state, "GET", "/api/v1/company/memory/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["facts"], 0, "still no operator facts");
    assert_eq!(stats["totalItems"], 2);
    assert_eq!(stats["teammateMemory"], 1);
    assert_eq!(stats["taskOutcomes"], 1);
    assert_eq!(stats["documentMemory"], 0);
    assert_eq!(
        stats["factsUpdatedAtMillis"], 0,
        "the facts-only figure is unchanged — it is simply not the whole story"
    );
    let last_updated = stats["lastUpdatedAtMillis"].as_u64().unwrap();
    assert!(
        last_updated >= before,
        "agent-written memory must move the Brain's Last updated stat, got {last_updated}"
    );

    // An operator fact newer than any chunk takes over the stat.
    runtime
        .facts()
        .upsert(
            runtime.id(),
            &FactRecord {
                id: "f-future".into(),
                kind: FactKind::Fact,
                title: "Board meeting".into(),
                body: "moved to Monday".into(),
                source: "You".into(),
                updated_at_millis: last_updated + 60_000,
            },
        )
        .await
        .unwrap();
    let (status, stats) = send(&state, "GET", "/api/v1/company/memory/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        stats["lastUpdatedAtMillis"],
        last_updated + 60_000,
        "the stat is the max across every memory source, whichever is freshest"
    );
    assert_eq!(stats["totalItems"], 3);

    // The list surfaces the same stamps per row, so a context card no longer
    // renders "—" while the header claims recent activity.
    let (status, rows) = send(&state, "GET", "/api/v1/company/memory", None).await;
    assert_eq!(status, StatusCode::OK);
    let rows = rows["items"].as_array().unwrap();
    let context_rows: Vec<&Value> = rows.iter().filter(|r| r["origin"] != "fact").collect();
    assert_eq!(context_rows.len(), 2);
    assert!(
        context_rows
            .iter()
            .all(|r| r["updatedAt"].as_u64().unwrap() >= before),
        "each agent-written row carries the time it was stored"
    );
}

/// End-to-end proof that the dual-write closes the manual-ingest loop: an
/// operator note written over HTTP is retrieved by the harness's ContextStore
/// search and rendered by `memory_loop::inject` into the augmented prompt. Gated
/// on `openhuman` because `memory_loop` is only compiled under that feature.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn memory_operator_fact_is_injected_into_the_agent_turn() {
    use crate::harness::memory_loop;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    // Operator adds a note through the console write path.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/memory",
        Some(json!({"kind": "reference", "title": "Launch plan", "body": "we ship on Friday at noon"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The harness retrieve step searches the ContextStore; the mirror lands
    // there, so a relevant next-turn message recalls it and `inject` renders it
    // into the augmented prompt — the closed loop, end to end.
    // The fs ContextStore search is substring-based, so query a token that
    // appears verbatim in the stored `title\nbody` mirror.
    let hits = runtime
        .context
        .search(runtime.id(), "Friday", memory_loop::RETRIEVE_TOP_K)
        .await
        .unwrap();
    assert!(!hits.is_empty(), "the operator note must be retrievable");
    let augmented = memory_loop::inject("when do we ship?", &hits);
    assert!(augmented.contains("Relevant prior work"));
    assert!(augmented.contains("we ship on Friday at noon"));
    assert!(augmented.trim_end().ends_with("when do we ship?"));
}

/// Two-company isolation over HTTP: company B never sees company A's facts, and
/// a tenant token may not address a company it does not own (403) — the same
/// scoped-auth boundary the credential route enforces.
#[tokio::test]
async fn memory_is_isolated_between_companies() {
    use crate::server::platform_auth::{
        PlatformAuthConfig, PlatformClaims, UnsignedTenantVerifier,
    };
    use std::collections::HashSet;

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let verifier = std::sync::Arc::new(UnsignedTenantVerifier::new("plat-secret"));
    let state = AppState::new(AppConfig::default())
        .with_home(home.clone())
        .with_platform_auth(PlatformAuthConfig::new(verifier));

    for name in ["a", "b"] {
        let id = CompanyId::new(name);
        let runtime = RuntimeBuilder::new(home.clone(), manifest())
            .with_id(id.clone())
            .build()
            .await
            .unwrap();
        state
            .registry()
            .insert(id.clone(), std::sync::Arc::new(runtime));
        state.set_owner(id.clone(), format!("tenant:{name}"));
    }

    let token = |tenant: &str| {
        UnsignedTenantVerifier::tenant_token(&PlatformClaims {
            tenant: tenant.to_string(),
            scopes: HashSet::from(["operator".to_string()]),
            companies: None,
        })
    };

    // Company A's owner writes a fact to A.
    let (status, _) = send_auth(
        &state,
        "POST",
        "/api/v1/companies/a/memory",
        Some(json!({"kind": "fact", "title": "A secret", "body": "A body"})),
        Some(&token("tenant:a")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Company B's owner sees an empty memory — A's fact is invisible to B.
    let (status, list_b) = send_auth(
        &state,
        "GET",
        "/api/v1/companies/b/memory",
        None,
        Some(&token("tenant:b")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list_b["items"].as_array().unwrap().len(), 0);

    // A's own memory holds exactly the one fact.
    let (status, list_a) = send_auth(
        &state,
        "GET",
        "/api/v1/companies/a/memory",
        None,
        Some(&token("tenant:a")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list_a["items"].as_array().unwrap().len(), 1);

    // A's token may not address B's memory at all — 403 (scoped auth).
    let (status, _) = send_auth(
        &state,
        "GET",
        "/api/v1/companies/b/memory",
        None,
        Some(&token("tenant:a")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn workspace_create_write_move_and_cycle_rejection() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, folder) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "Brand", "kind": "folder"})),
    )
    .await;
    let folder_id = folder["id"].as_str().unwrap().to_string();

    let (status, file) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "voice.md", "kind": "file", "parentId": folder_id, "content": "# Voice"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let file_id = file["id"].as_str().unwrap().to_string();

    // Overwrite content.
    let (status, ack) = send(
        &state,
        "PUT",
        &format!("/api/v1/company/workspace/file/{file_id}"),
        Some(json!({"content": "# Voice v2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(ack["updatedAt"].is_number());

    // Explicit `"parentId": null` moves the file back to the workspace root.
    let (status, moved) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/workspace/{file_id}"),
        Some(json!({"parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        moved.get("parentId").is_none(),
        "node moved to root has no parentId"
    );

    // Cycle rejection: move a folder under its own child.
    let (_, child) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "Sub", "kind": "folder", "parentId": folder_id})),
    )
    .await;
    let child_id = child["id"].as_str().unwrap().to_string();
    let (status, body) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/workspace/{folder_id}"),
        Some(json!({"parentId": child_id})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");

    // Recursive delete.
    let (status, _) = send(
        &state,
        "DELETE",
        &format!("/api/v1/company/workspace/{folder_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// Issue #666 applies to every way the filesystem path can change, not only to
/// creates. A rename that could alias a sibling is refused before either the
/// index or either file body moves.
#[tokio::test]
async fn workspace_rename_cannot_claim_a_siblings_physical_path() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (_, first) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "first.md", "kind": "file", "content": "first body"})),
    )
    .await;
    let first_id = first["id"].as_str().unwrap().to_string();
    let (_, second) = send(
        &state,
        "POST",
        "/api/v1/company/workspace",
        Some(json!({"name": "second.md", "kind": "file", "content": "second body"})),
    )
    .await;
    let second_id = second["id"].as_str().unwrap().to_string();

    let (status, refusal) = send(
        &state,
        "PATCH",
        &format!("/api/v1/company/workspace/{second_id}"),
        Some(json!({"name": "first.md"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
    assert_eq!(refusal["code"], "conflict", "{refusal}");

    for (id, name, content) in [
        (&first_id, "first.md", "first body"),
        (&second_id, "second.md", "second body"),
    ] {
        let (status, file) = send(
            &state,
            "GET",
            &format!("/api/v1/company/workspace/file/{id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{file}");
        assert_eq!(file["name"], name, "the refused rename changed metadata");
        assert_eq!(
            file["content"], content,
            "the refused rename moved or overwrote a sibling body"
        );
    }
}
