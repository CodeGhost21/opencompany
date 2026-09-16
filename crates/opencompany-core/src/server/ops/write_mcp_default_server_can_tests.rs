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

/// An install default is disabled by its *first* runtime override: no prior
/// runtime entry exists to patch, so `update_server` must fall back to the
/// default declaration as its patch base rather than 404 (issue #527).
#[tokio::test]
async fn mcp_default_server_can_be_disabled_with_its_first_override() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let default = crate::company::McpServer {
        name: "deepwiki".to_string(),
        endpoint: "https://deepwiki.example/mcp".to_string(),
        ..Default::default()
    };
    let state = state_with_manifest_and_defaults(&home, manifest(), vec![default]).await;

    // Cold, the default is visible and badged `default`.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["name"], "deepwiki");
    assert_eq!(list[0]["source"], "default");

    // The first override disables it — the override persists alongside the
    // default, keeping the effective body but flipping `enabled` off.
    let (status, updated) = send(
        &state,
        "PUT",
        "/api/v1/company/mcp/servers/deepwiki",
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["server"]["enabled"], false);
    assert_eq!(
        updated["server"]["source"], "default",
        "an override inherits the default badge, so delete still refuses it"
    );

    // Delete still refuses: the declaration lives in the install config, and the
    // disable override is the supported toggle.
    let (status, _) = send(
        &state,
        "DELETE",
        "/api/v1/company/mcp/servers/deepwiki",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The disable took: listing reflects `enabled: false`.
    let (_, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(list[0]["enabled"], false);
}

/// Without the `openhuman` feature there is no MCP transport, so live discovery
/// is "not wired". (Under the feature it would attempt a real network call.)
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn mcp_discovery_is_not_wired_without_the_feature() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, mcp_manifest()).await;
    let (status, body) = send(
        &state,
        "GET",
        "/api/v1/company/mcp/servers/docs/tools",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_wired");
}

/// A `user:pass@host` endpoint smuggles a credential into the URL — rejected as
/// a 400 (the error-hardening cell's validate-on-add).
#[tokio::test]
async fn mcp_userinfo_endpoint_is_rejected() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({ "name": "creds", "endpoint": "https://user:pass@host/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// A query-parameter credential (BrowserBase style) round-trips write-only:
/// `authConfigured` flips true, the value never appears in the response, and a
/// non-secret id left in the endpoint URL raises the non-blocking advisory.
#[tokio::test]
async fn mcp_query_param_auth_round_trips_write_only_with_advisory() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, added) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({
            "name": "browserbase",
            // A secret-looking query param triggers the advisory; the real
            // credential rides write-only as a query-parameter auth.
            "endpoint": "https://api.browserbase.com/mcp?apiKey=leftover",
            "authKind": "query_param",
            "paramName": "apiKey",
            "token": "qp-secret-xyz"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(added["server"]["authConfigured"], true);
    assert!(
        added["warning"].as_str().is_some(),
        "a secret-looking endpoint query raises the advisory: {added}"
    );
    assert!(
        !serde_json::to_string(&added)
            .unwrap()
            .contains("qp-secret-xyz"),
        "the query-parameter credential leaked into the response"
    );

    // A query_param auth WITHOUT a paramName is a 400.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({
            "name": "noparam",
            "endpoint": "https://host/mcp",
            "authKind": "query_param",
            "token": "x"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Workflow creator (issue #69)
// ---------------------------------------------------------------------------

/// Boots an fs-backed company with a writable source directory (a `seed_dir`)
/// — the workflow creator writes `workflows/<id>.toml` under it, mirroring how
/// a real `companies/<name>` checkout is wired via `--company`.
async fn state_with_source_dir(
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

/// A valid graph body: a trigger → an agent node naming the roster's `ceo` →
/// an output. `$id` becomes both the workflow id and its display name.
fn workflow_body(id: &str) -> Value {
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

/// Issue #168: the create path persists the graph **on the record**, never in
/// the company source tree (which is a read-only mount in hosted mode), and
/// both read routes serve it from there.
#[tokio::test]
async fn workflow_create_persists_on_the_record_appends_enabled_and_is_listed() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let seed_dir = home.join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let state = state_with_source_dir(&home, &seed_dir, manifest()).await;

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(workflow_body("greet")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["id"], "greet");
    assert_eq!(created["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(created["edges"].as_array().unwrap().len(), 2);

    // Nothing was written into the company source tree — the read-only mount in
    // hosted mode, and the whole reason #168 failed with EROFS.
    let path = seed_dir.join("workflows").join("greet.toml");
    assert!(!path.exists(), "the source tree must not be written to");

    // The body and the enabled id both landed on the operator's live record —
    // the version-controlled seed dir's own `company.toml` was never touched
    // (there isn't one here; only the store's copy is checked).
    use crate::ports::CompanyStore;
    let store = FsCompanyStore::new(home.to_path_buf());
    let record = store.load(&CompanyId::new("acme")).await.unwrap().unwrap();
    assert_eq!(record.manifest.workflows.enabled, vec!["greet".to_string()]);
    assert_eq!(record.overlay_workflows.len(), 1);
    assert_eq!(record.overlay_workflows[0].id, "greet");
    assert!(record.overlay_workflows[0].toml.contains("agent = \"ceo\""));

    // `GET …/workflows` (seed ∪ overlay) now lists it.
    let (status, list) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    // The company's own graphs; the baseline is listed in every company.
    // Id heuristic, not provenance — `greet` never collides with a global id
    // here, so this is safe; see
    // `workflow_create_of_an_id_matching_a_global_wins_by_content` for the
    // colliding case, asserted by content rather than this filter.
    let rows: Vec<&serde_json::Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or_default();
            !crate::globals::workflows().iter().any(|w| w.id == id)
        })
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "greet");

    // `GET …/workflows/{wid}` round-trips the full graph too.
    let (status, graph) = send(&state, "GET", "/api/v1/company/workflows/greet", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(graph["name"], "greet");
}

/// A company workflow whose id matches a global's must win — checked by its
/// own content (the name it was created with), not by an id-membership
/// filter, which would misclassify this exact row as "the baseline's" because
/// the ids match. `create_company_workflow` does not special-case global ids
/// (only seed files, overlays and `[workflows].enabled` reserve one), so
/// creating over a global id is exactly this: the company's overlay
/// definition of that id supersedes the global on every read.
#[tokio::test]
async fn workflow_create_of_an_id_matching_a_global_wins_by_content() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let seed_dir = home.join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let state = state_with_source_dir(&home, &seed_dir, manifest()).await;
    let taken = crate::globals::workflows()[0].id.clone();

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(workflow_body(&taken)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(created["id"], taken);

    let (status, list) = send(&state, "GET", "/api/v1/company/workflows", None).await;
    assert_eq!(status, StatusCode::OK);
    let matching: Vec<&serde_json::Value> = list
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["id"] == taken.as_str())
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "the shadowed global must not be listed alongside the override: {list}"
    );
    assert_eq!(
        matching[0]["name"], taken,
        "the company's own definition (named after its id, per `workflow_body`) must win"
    );

    let (status, graph) = send(
        &state,
        "GET",
        &format!("/api/v1/company/workflows/{taken}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(graph["name"], taken);
}

#[tokio::test]
async fn workflow_create_duplicate_id_is_conflict() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let seed_dir = home.join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let state = state_with_source_dir(&home, &seed_dir, manifest()).await;

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(workflow_body("greet")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(workflow_body("greet")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict");
}

#[tokio::test]
async fn workflow_create_rejects_bad_edges_missing_agent_and_no_trigger() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let seed_dir = home.join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let state = state_with_source_dir(&home, &seed_dir, manifest()).await;

    // An edge referencing a node id that doesn't exist.
    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(json!({
            "id": "bad-edge",
            "name": "Bad edge",
            "nodes": [{"id": "start", "kind": "trigger", "name": "Start"}],
            "edges": [{"from": "start", "to": "ghost"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Issue #1016: a dangling edge is now a structured `workflow_invalid` whose
    // `problems` array names the endpoint and the field, so the console can
    // highlight the id the author wrote.
    assert_eq!(body["code"], "workflow_invalid");
    assert_eq!(body["problems"][0]["node_id"], "ghost");
    assert_eq!(body["problems"][0]["field"], "to");

    // An agent node naming a teammate not on the roster.
    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(json!({
            "id": "bad-agent",
            "name": "Bad agent",
            "nodes": [
                {"id": "start", "kind": "trigger", "name": "Start"},
                {"id": "worker", "kind": "agent", "name": "Worker", "agent": "ghost"},
            ],
            "edges": [{"from": "start", "to": "worker"}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");
    assert!(body["error"].as_str().unwrap().contains("roster"), "{body}");

    // No trigger node at all.
    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(json!({
            "id": "no-trigger",
            "name": "No trigger",
            "nodes": [{"id": "only", "kind": "output", "name": "Only"}],
            "edges": [],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_request");

    // None of the rejected attempts left a file behind.
    assert!(
        !seed_dir.join("workflows").is_dir() || {
            std::fs::read_dir(seed_dir.join("workflows"))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true)
        }
    );
}

#[tokio::test]
async fn workflow_create_without_source_dir_succeeds() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    // `state_with_company` boots with no `seed_dir`, so the company has no
    // source directory at all — the platform-provisioned-mode case. Issue #168:
    // creation used to be refused here with a 400; the body now lands on the
    // record, so it succeeds and reads back.
    let state = state_with_company(&home).await;

    let (status, created) = send(
        &state,
        "POST",
        "/api/v1/company/workflows",
        Some(workflow_body("greet")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {created}");
    assert_eq!(created["id"], "greet");

    let (status, graph) = send(&state, "GET", "/api/v1/company/workflows/greet", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 3);
}

/// Without the `openhuman` feature the on-demand Test route is "not wired".
#[cfg(not(feature = "openhuman"))]
#[tokio::test]
async fn mcp_test_route_is_not_wired_without_the_feature() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;
    send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({ "name": "notion", "endpoint": "https://notion.example/mcp" })),
    )
    .await;
    let (status, body) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers/notion/test",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_wired");
}
