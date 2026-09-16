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

/// A manifest that declares one committed `[[mcp_server]]` — used to assert the
/// manifest-server guards (cannot delete; overridable).
fn mcp_manifest() -> CompanyManifest {
    toml::from_str(
        "[company]\nname = \"Acme\"\n[[agent]]\nid = \"ceo\"\nrole = \"Chief\"\n[policy]\nmode = \"full\"\n[[mcp_server]]\nname = \"docs\"\nendpoint = \"https://docs.example/mcp\"\n",
    )
    .unwrap()
}

/// Boots an fs-backed company from a caller-supplied manifest (mirrors
/// `state_with_company`, which pins the default manifest).
async fn state_with_manifest(home: &std::path::Path, manifest: CompanyManifest) -> AppState {
    state_with_manifest_and_defaults(home, manifest, Vec::new()).await
}

/// Like [`state_with_manifest`], but with install-wide default MCP servers
/// configured (issue #527), for asserting the default-override guards.
async fn state_with_manifest_and_defaults(
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

/// Like [`state_with_manifest`], but seeds operator-added overlay teammates too,
/// so a test can assert MCP reachability over the full runtime roster — manifest
/// agents plus overlay agents — the way `build_roster` composes it (issue #568).
async fn state_with_manifest_and_overlays(
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

#[tokio::test]
async fn mcp_servers_crud_round_trips_and_token_is_write_only() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    // Cold: no servers.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 0);

    // Add a runtime server WITH a token.
    let (status, added) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({
            "name": "notion",
            "endpoint": "https://notion.example/mcp",
            "token": "sk-write-only-abc",
            "allowedTools": ["search"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(added["server"]["name"], "notion");
    assert_eq!(added["server"]["source"], "runtime");
    assert_eq!(added["server"]["authConfigured"], true);
    // Issue #566: a mutating MCP change reaches agents on the company's next turn
    // (the effective set is re-fingerprinted every `HarnessPool::ensure` cycle), so
    // the note must state the no-restart contract outright — not merely avoid one
    // stale phrase. Asserting the positive claim rejects any "restart required"
    // variant too, which a bare `!contains("restart the company")` would let pass.
    let note = added["note"].as_str().unwrap();
    assert!(
        note.contains("next turn"),
        "note should promise next-turn pickup: {note}"
    );
    assert!(
        note.contains("no restart needed"),
        "mutating MCP response must state no restart is needed: {note}"
    );

    // The token must NOT appear anywhere in the add response.
    assert!(
        !serde_json::to_string(&added)
            .unwrap()
            .contains("sk-write-only-abc"),
        "add response leaked the token"
    );

    // GET reflects it, still without the token.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    let body = serde_json::to_string(&list).unwrap();
    assert!(body.contains("notion"));
    assert!(body.contains("\"authConfigured\":true"));
    assert!(!body.contains("sk-write-only-abc"), "list leaked the token");

    // Duplicate add is a 409.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({ "name": "notion", "endpoint": "https://notion.example/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Non-http endpoint is a 400.
    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({ "name": "bad", "endpoint": "ftp://x/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Disable via PUT.
    let (status, updated) = send(
        &state,
        "PUT",
        "/api/v1/company/mcp/servers/notion",
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["server"]["enabled"], false);
    assert_eq!(
        updated["server"]["authConfigured"], true,
        "token survives an update"
    );

    // Delete (runtime server) → 204, then it's gone.
    let (status, _) = send(&state, "DELETE", "/api/v1/company/mcp/servers/notion", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(list.as_array().unwrap().len(), 0);
}

/// Issue #1270: a build without the `mcp` feature must serve List A exactly as
/// before and answer the directory routes `not_wired`.
///
/// Gated on the absence of the feature rather than written once for both builds:
/// with `mcp` on, these routes reach a live registry and two upstream
/// directories over the network, which is not a thing a unit test may do. The
/// default `cargo test --locked` lane is what runs this, and it is the lane that
/// compiles the unwired half in the first place.
#[cfg(not(feature = "mcp"))]
#[tokio::test]
async fn without_the_mcp_feature_the_directory_is_not_wired_and_list_a_is_unchanged() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home).await;

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/mcp/servers",
        Some(json!({ "name": "notion", "endpoint": "https://notion.example/mcp" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // List A is served, and carries none of the registry-only keys.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["source"], "runtime");
    for key in ["serverId", "qualifiedName", "iconUrl", "transport"] {
        assert!(
            list[0].get(key).is_none(),
            "`{key}` must not appear without a registry install"
        );
    }

    // Every directory route answers the console's degrade signal.
    for (method, uri) in [
        ("GET", "/api/v1/company/mcp/registry/search?q=git"),
        (
            "GET",
            "/api/v1/company/mcp/registry/entry?qualifiedName=@a/b",
        ),
        ("POST", "/api/v1/company/mcp/registry/install"),
        ("POST", "/api/v1/company/mcp/registry/sid/connect"),
        ("POST", "/api/v1/company/mcp/registry/sid/disconnect"),
        ("PUT", "/api/v1/company/mcp/registry/sid/env"),
        ("DELETE", "/api/v1/company/mcp/registry/sid"),
    ] {
        let (status, body) = send(&state, method, uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        assert_eq!(body["code"], "not_wired", "{method} {uri}");
    }

    // And the List A delete still works with no install behind the row.
    let (status, _) = send(&state, "DELETE", "/api/v1/company/mcp/servers/notion", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn mcp_manifest_server_cannot_be_deleted_but_can_be_overridden() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, mcp_manifest()).await;

    // The manifest server shows up as `manifest`.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list[0]["name"], "docs");
    assert_eq!(list[0]["source"], "manifest");

    // Deleting a manifest server is a 409.
    let (status, _) = send(&state, "DELETE", "/api/v1/company/mcp/servers/docs", None).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // But it can be disabled via a runtime override — still badged manifest.
    let (status, updated) = send(
        &state,
        "PUT",
        "/api/v1/company/mcp/servers/docs",
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["server"]["source"], "manifest");
    assert_eq!(updated["server"]["enabled"], false);
    // The mutating response carries reachability too (issue #568), so the console
    // reflects who can reach the server right after an edit, not only on reload.
    assert!(
        updated["server"]["reachableBy"].is_array(),
        "a mutating response also carries reachableBy"
    );
}

/// Issue #568: each listed server carries the agents whose *effective* grants
/// reach it — over the full runtime roster, manifest agents plus overlay
/// teammates. With a company `allow = ["*", "mcp:*"]`, an agent that declares
/// no `tools` (and every overlay teammate, which has no tools row) inherits the
/// wildcard and explicit MCP grant and reaches every server; an agent that
/// narrows itself to `mcp:notion` reaches only that server.
#[tokio::test]
async fn mcp_reachability_lists_reaching_agents_including_overlay() {
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [\"*\", \"mcp:*\"]\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\ntools = [\"mcp:notion\"]\n\
         [[agent]]\nid = \"eng\"\nrole = \"Engineer\"\n[policy]\nmode = \"full\"\n\
         [[mcp_server]]\nname = \"notion\"\nendpoint = \"https://notion.example/mcp\"\n\
         [[mcp_server]]\nname = \"linear\"\nendpoint = \"https://linear.example/mcp\"\n",
    )
    .unwrap();
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    // A minted id, exactly as `POST …/team` gives an operator-added teammate —
    // the shape that used to reach the console's "Reachable by" line raw (#931).
    let overlay = crate::ports::types::OverlayAgent {
        provider: None,
        id: "019fa75dbc9b-000000000001".to_string(),
        name: "Helper".to_string(),
        role: "Assistant".to_string(),
        description: None,
        tools: None,
        model: None,
        harness: None,
    };
    let state = state_with_manifest_and_overlays(&home, manifest, vec![overlay]).await;

    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    let reach = |name: &str| -> Vec<(String, String)> {
        let row = list
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("server `{name}` is listed"));
        let mut agents: Vec<(String, String)> = row["reachableBy"]
            .as_array()
            .expect("reachableBy serializes as an array")
            .iter()
            .map(|v| {
                (
                    v["id"].as_str().unwrap().to_string(),
                    v["name"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        agents.sort();
        agents
    };
    let pair = |id: &str, name: &str| (id.to_string(), name.to_string());

    // notion: the narrowed ceo, the wildcard-inheriting eng, and the overlay.
    // Issue #931: every row carries the display label the rest of the console
    // uses — a manifest agent's role, an overlay teammate's name — so the minted
    // overlay id is never what a reader sees.
    // The four baseline teammates every company inherits ask for `mcp:*`, so a
    // company granting it reaches them too. Listed rather than filtered out:
    // this asserts the whole reachable set, and hiding the half that is not
    // this manifest's own would leave the baseline free to drift unseen.
    assert_eq!(
        reach("notion"),
        vec![
            pair("019fa75dbc9b-000000000001", "Helper"),
            pair("ceo", "Chief"),
            pair("eng", "Engineer"),
            pair("operations", "Operations"),
            pair("page_builder", "Page Builder"),
            pair("researcher", "Researcher"),
            pair("writer", "Writer"),
        ]
    );
    // linear: only the wildcard holders — ceo scoped itself out of it.
    assert_eq!(
        reach("linear"),
        vec![
            pair("019fa75dbc9b-000000000001", "Helper"),
            pair("eng", "Engineer"),
            pair("operations", "Operations"),
            pair("page_builder", "Page Builder"),
            pair("researcher", "Researcher"),
            pair("writer", "Writer"),
        ],
        "ceo narrowed to mcp:notion, so it cannot reach linear"
    );
}

/// Issue #568: a server no agent's grants cover comes back with an **empty**
/// `reachableBy` — the signal the console flags loudly rather than showing a
/// healthy server that is silently unreachable. Here a narrow company
/// `allow = ["mcp:docs"]` reaches `docs` but never `notion`.
#[tokio::test]
async fn mcp_reachability_flags_a_server_no_agent_can_reach() {
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [\"mcp:docs\"]\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\ntools = [\"mcp:docs\"]\n[policy]\nmode = \"full\"\n\
         [[mcp_server]]\nname = \"docs\"\nendpoint = \"https://docs.example/mcp\"\n\
         [[mcp_server]]\nname = \"notion\"\nendpoint = \"https://notion.example/mcp\"\n",
    )
    .unwrap();
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, manifest).await;

    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    let row = |name: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("server `{name}` is listed"))
            .clone()
    };
    assert_eq!(
        row("docs")["reachableBy"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["id"].as_str().unwrap(), v["name"].as_str().unwrap()))
            .collect::<Vec<_>>(),
        vec![("ceo", "Chief")],
        "the company allow covers mcp:docs for the one agent"
    );
    assert!(
        row("notion")["reachableBy"].as_array().unwrap().is_empty(),
        "no agent's grants cover mcp:notion — the flagged zero case"
    );
}

/// Issue #568: a **disabled** server reaches nobody, however wide the grants.
/// `registry_for_agent` filters on `decl.enabled && grants_cover_server(..)`, so
/// an agent holding `mcp:docs` is handed no such tool while the server is off —
/// reporting it as reachable would be the console/harness disagreement this
/// feature exists to remove. Asserted on both readers: the mutating response
/// that turns the server off, and the later list.
#[tokio::test]
async fn mcp_reachability_is_empty_for_a_disabled_server() {
    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n[tools]\nallow = [\"*\", \"mcp:*\"]\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\ntools = [\"mcp:docs\"]\n[policy]\nmode = \"full\"\n\
         [[mcp_server]]\nname = \"docs\"\nendpoint = \"https://docs.example/mcp\"\n",
    )
    .unwrap();
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_manifest(&home, manifest).await;

    let reach = |body: &serde_json::Value| -> Vec<String> {
        body["reachableBy"]
            .as_array()
            .expect("reachableBy serializes as an array")
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect()
    };

    // Enabled: the one agent's grant covers it, and so does the baseline's —
    // this company grants `mcp:*`, which the inherited teammates ask for. The
    // disabled assertion below is the one this test is about.
    let (status, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reach(&list[0]),
        vec![
            "ceo".to_string(),
            "operations".to_string(),
            "page_builder".to_string(),
            "researcher".to_string(),
            "writer".to_string(),
        ]
    );

    // Disabling it empties reachability in the mutating response itself.
    let (status, updated) = send(
        &state,
        "PUT",
        "/api/v1/company/mcp/servers/docs",
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["server"]["enabled"], false);
    assert!(
        reach(&updated["server"]).is_empty(),
        "a disabled server is handed to no agent, so it is reachable by none"
    );

    // And the list agrees on the next read — the grant is unchanged, the server is off.
    let (_, list) = send(&state, "GET", "/api/v1/company/mcp/servers", None).await;
    assert_eq!(list[0]["enabled"], false);
    assert!(
        reach(&list[0]).is_empty(),
        "the list reader applies the same enabled filter as the harness"
    );
}
