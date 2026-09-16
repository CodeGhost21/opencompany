use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::company::CompanyManifest;
use crate::ports::CompanyStore;
use crate::ports::store::company_write_lock;
use crate::ports::types::{CompanyId, CompanyRecord};
use crate::runtime::RuntimeBuilder;
use crate::server::router;
use crate::store::FsCompanyStore;
use crate::{AppConfig, AppState};

/// A company whose grants actually bite: `ceo` asks for one tool the company
/// does not allow, `writer` asks for nothing at all, and `hermit` sits on no
/// desk. Each of those is a different arm of the resolution under test.
const ROSTER: &str = r#"
[company]
name = "Acme"
[policy]
mode = "full"
[tools]
allow = ["workspace", "workspace.*", "composio"]

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction and delegates."
tier = "orchestrator"
tools = ["workspace.read", "email.send"]

[[agent]]
id = "writer"
role = "Writer"

[[agent]]
id = "hermit"
role = "Hermit"

[[group_chat]]
id = "content"
name = "Content desk"
members = ["writer", "ceo"]
"#;

/// [`ROSTER`], plus a declared `[[harness]]` set (issue #1245's
/// harness-picker follow-up): `laptop` is a `local` ACP harness and the
/// **default**, so a fresh overlay teammate — which names no harness of
/// its own — lands there and a model override on it is meaningful. Tests
/// that need to exercise the harness picker itself declare a second,
/// non-default `built_in` entry (`main`) to switch *away* from.
const ACP_ROSTER: &str = r#"
[company]
name = "Acme"
[policy]
mode = "full"
[tools]
allow = ["workspace", "workspace.*", "composio"]

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction and delegates."
tier = "orchestrator"
tools = ["workspace.read", "email.send"]

[[harness]]
id = "main"
kind = "built_in"

[[harness]]
id = "laptop"
kind = "acp"
default = true

[harness.acp]
transport = "local"
agent = "claude"
"#;

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-agent-detail-")
        .tempdir()
        .expect("tempdir")
}

/// Issue #661 / L5, updated for #1804's three-state grant: `requested_grants`
/// reads a manifest agent's `tools` line, falls back to an overlay teammate's
/// own grant, and returns the three states verbatim — `None` (absent line,
/// the standard company-wide grant), `Some(vec![])` (an explicit deny-all),
/// and `Some(globs)` (a narrowed grant). An unknown id reads as `None`.
#[test]
fn requested_grants_reads_overlay_then_manifest_then_empty() {
    use crate::ports::types::OverlayAgent;

    let manifest: CompanyManifest = toml::from_str(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"ceo\"\nrole = \"Chief\"\ntools = [\"workspace.read\"]\n",
    )
    .unwrap();
    let mut record = CompanyRecord {
        overlay_desk_hive: Vec::new(),
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: CompanyId::new("acme"),
        manifest,
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
    };
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "scoped".to_string(),
        name: "Scoped".to_string(),
        role: "Researcher".to_string(),
        description: None,
        tools: Some(vec!["docs.*".to_string()]),
        model: None,
        harness: None,
    });
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "standard".to_string(),
        name: "Standard".to_string(),
        role: "Generalist".to_string(),
        description: None,
        // `None` = no line of its own → the standard company-wide grant.
        tools: None,
        model: None,
        harness: None,
    });
    record.overlay_agents.push(OverlayAgent {
        provider: None,
        id: "denied".to_string(),
        name: "Denied".to_string(),
        role: "Contractor".to_string(),
        description: None,
        // `Some(vec![])` = an explicit deny-all since #1804, distinct from None.
        tools: Some(Vec::new()),
        model: None,
        harness: None,
    });

    // A manifest agent's own line.
    assert_eq!(
        super::requested_grants(&record, "ceo"),
        Some(vec!["workspace.read".to_string()])
    );
    // An overlay teammate's own grant (the L5 read side).
    assert_eq!(
        super::requested_grants(&record, "scoped"),
        Some(vec!["docs.*".to_string()])
    );
    // An overlay teammate with no line of its own → None (the standard grant).
    assert_eq!(super::requested_grants(&record, "standard"), None);
    // An explicit deny-all reads back as `Some(vec![])`, NOT None (#1804).
    assert_eq!(super::requested_grants(&record, "denied"), Some(Vec::new()));
    // An unknown id → None, as before.
    assert_eq!(super::requested_grants(&record, "nobody"), None);
}

async fn state_with_manifest(home: &std::path::Path, manifest_toml: &str) -> AppState {
    let manifest: CompanyManifest = toml::from_str(manifest_toml).unwrap();
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
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

async fn send(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", crate::server::test_support::fixed_cookie("acme"));
    let request = match &body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(value).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
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

async fn draft_for(state: &AppState, agent: &str, body: Value) -> (StatusCode, Value) {
    send(
        state,
        "POST",
        &format!("/api/v1/company/team/{agent}/draft"),
        Some(body),
    )
    .await
}

async fn get_agent(state: &AppState, agent: &str) -> (StatusCode, Value) {
    send(state, "GET", &format!("/api/v1/company/team/{agent}"), None).await
}

async fn patch_agent(state: &AppState, agent: &str, body: Value) -> (StatusCode, Value) {
    send(
        state,
        "PATCH",
        &format!("/api/v1/company/team/{agent}"),
        Some(body),
    )
    .await
}

/// Drives the route as a specific principal. The harness signs every other
/// request in as an admin, which is exactly why this exists: an
/// authority check verified only as an admin passes identically against no
/// check at all.
async fn send_as(
    state: &AppState,
    method: &str,
    uri: &str,
    body: Option<Value>,
    cookie: String,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let request = match &body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(value).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
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

/// Adds a teammate through the console's own route and returns its id.
async fn add_overlay(state: &AppState, name: &str, role: &str) -> String {
    let (status, created) = send(
        state,
        "POST",
        "/api/v1/company/team",
        Some(json!({"name": name, "role": role, "description": "Original."})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    created["id"].as_str().unwrap().to_string()
}

/// Seeds one `inference/providers` row directly on `agent`'s secret
/// store, for the pin-validation tests (keys rework, issue #2306, slice
/// 3a) — the same pattern `server::ops::inference`'s own tests use.
async fn seed_provider(state: &AppState, slug: &str, enabled: bool) {
    use crate::company::inference::store;

    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");
    store::put_provider(
        &id,
        runtime.secrets().as_ref(),
        store::ProviderDraft {
            slug: slug.to_string(),
            label: slug.to_string(),
            kind: "custom".to_string(),
            base_url: "http://127.0.0.1:9/v1".to_string(),
            models: std::collections::BTreeMap::new(),
            enabled,
        },
    )
    .await
    .unwrap();
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected an array, got {value}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

// --- The read half (issue #264) -----------------------------------------

/// The whole of what the issue calls unreachable, on the wire: tier,
/// description, resolved tools and desk membership for a manifest teammate.
#[tokio::test]
async fn a_manifest_agent_opens_with_its_tier_tools_and_desks() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), ROSTER).await;

    let (status, ceo) = get_agent(&state, "ceo").await;
    assert_eq!(status, StatusCode::OK, "{ceo}");

    assert_eq!(ceo["id"], "ceo");
    assert_eq!(ceo["role"], "Chief Executive");
    assert_eq!(ceo["description"], "Sets direction and delegates.");
    assert_eq!(ceo["source"], "manifest");
    assert_eq!(ceo["tier"], "orchestrator");
    assert_eq!(ceo["isOrchestrator"], true, "{ceo}");
    assert!(
        ceo["name"].is_null(),
        "a manifest teammate is named by its role: {ceo}"
    );

    // Desk membership, with the lead flag resolved from the effective order
    // rather than from the declared list — `writer` is declared first.
    let desks = ceo["desks"].as_array().unwrap();
    assert_eq!(desks.len(), 1, "{ceo}");
    assert_eq!(desks[0]["id"], "content");
    assert_eq!(desks[0]["name"], "Content desk");
    assert_eq!(desks[0]["lead"], false, "the writer leads this desk: {ceo}");

    // A teammate on no desk says so with an empty list rather than by
    // omitting the key, so the console can render "no desks" for sure.
    let (_, hermit) = get_agent(&state, "hermit").await;
    assert_eq!(hermit["desks"].as_array().unwrap().len(), 0, "{hermit}");
    assert_eq!(hermit["isOrchestrator"], false, "{hermit}");
}

/// Issue #1872 (codex): an `auto` channel confers no lead, so the roster
/// surfaces must not badge one.
///
/// `desks_for` used to read `members[0] == agent_id` straight off the
/// effective order, which is a rank only on a lead desk — on a channel it
/// is whoever happens to be listed first, and TeamView, the agent detail
/// page and the profile sheet all rendered them "(lead)". Reading through
/// `desk_lead` (`None` for an auto channel by definition) is what keeps
/// this honest; revert that and the first assertion below reads `true`.
///
/// The lead desk beside it is the half that must not move: a mode nobody
/// stated still badges its first member exactly as before.
#[tokio::test]
async fn an_auto_channel_badges_no_lead_but_a_desk_still_does() {
    let home = tempfile::tempdir().unwrap();
    let state = state_with_manifest(home.path(), ROSTER).await;
    // A channel and a lead desk, both holding `ceo` first.
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");
    let store = runtime.store();
    let mut record = store.load(&id).await.unwrap().unwrap();
    record.overlay_desks.push(crate::ports::types::OverlayDesk {
        id: "launch".to_string(),
        name: "Launch week".to_string(),
        description: None,
        members: vec!["ceo".to_string(), "writer".to_string()],
        responder: crate::ports::types::ResponderMode::Auto,
        hive: Default::default(),
    });
    record.overlay_desks.push(crate::ports::types::OverlayDesk {
        id: "growth".to_string(),
        name: "Growth".to_string(),
        description: None,
        members: vec!["ceo".to_string(), "writer".to_string()],
        responder: crate::ports::types::ResponderMode::Lead,
        hive: Default::default(),
    });
    store.save(&record).await.unwrap();

    let (_, ceo) = get_agent(&state, "ceo").await;
    let desks = ceo["desks"].as_array().unwrap();
    let by = |id: &str| {
        desks
            .iter()
            .find(|d| d["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing from {ceo}"))
            .clone()
    };
    assert_eq!(
        by("launch")["lead"],
        false,
        "an auto channel confers no rank on its first member: {ceo}"
    );
    assert_eq!(
        by("growth")["lead"],
        true,
        "a lead desk is unchanged: {ceo}"
    );
}

/// The verification gap the issue names: what an agent *asks* for and what
/// it *holds* are different lists, and only the second one matters.
///
/// `ceo` requests `email.send`, which `[tools].allow` does not cover, so it
/// is dropped. `writer` requests nothing, which means the company's standard
/// grant rather than no tools at all — the opposite reading, and the one a
/// naive surface would get wrong.
#[tokio::test]
async fn effective_tools_are_the_intersection_not_the_request() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), ROSTER).await;

    let (_, ceo) = get_agent(&state, "ceo").await;
    assert_eq!(
        strings(&ceo["tools"]["requested"]),
        vec!["workspace.read", "email.send"],
        "{ceo}"
    );
    assert_eq!(
        strings(&ceo["tools"]["companyAllow"]),
        vec!["workspace", "workspace.*", "composio"],
        "{ceo}"
    );
    assert_eq!(
        strings(&ceo["tools"]["effective"]),
        vec!["workspace.read"],
        "a request the company never allowed is not a grant: {ceo}"
    );

    let (_, writer) = get_agent(&state, "writer").await;
    assert!(writer["tools"]["requested"].is_null(), "{writer}");
    assert_eq!(
        strings(&writer["tools"]["effective"]),
        vec!["workspace", "workspace.*", "composio"],
        "an agent that lists no tools holds the company's whole allow-list, \
         which is the reading a surface must not invert: {writer}"
    );
}

/// With no desk declaring a ceiling — the shape of every company written
/// before desks could scope tools — the desk row is empty and the effective
/// grant is unchanged. This is the case that must not regress for anybody.
#[tokio::test]
async fn a_company_with_no_desk_ceilings_reports_an_empty_desk_row() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), ROSTER).await;

    for agent in ["ceo", "writer", "hermit"] {
        let (_, body) = get_agent(&state, agent).await;
        assert!(
            strings(&body["tools"]["deskAllow"]).is_empty(),
            "{agent}: {body}"
        );
        assert_eq!(
            body["tools"]["deskCeilingActive"], false,
            "no desk states a ceiling, so the desk level is not in play: {agent}: {body}"
        );
    }
}

/// A desk ceiling narrows every member of that desk, and only that desk's
/// members — the department scoping the feature exists for.
#[tokio::test]
async fn a_desk_ceiling_narrows_its_members_and_nobody_else() {
    let scoped = ROSTER.replace(
        "members = [\"writer\", \"ceo\"]",
        "members = [\"writer\", \"ceo\"]\ntools = [\"workspace.read\"]",
    );
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), &scoped).await;

    // `writer` asks for nothing, so before the desk it held the whole
    // company allow-list. The desk cuts it to one grant.
    let (_, writer) = get_agent(&state, "writer").await;
    assert_eq!(
        strings(&writer["tools"]["deskAllow"]),
        vec!["workspace.read"],
        "{writer}"
    );
    assert_eq!(
        writer["tools"]["deskCeilingActive"], true,
        "a desk ceiling is in play for a member of the desk: {writer}"
    );
    assert_eq!(
        strings(&writer["tools"]["effective"]),
        vec!["workspace.read"],
        "the desk ceiling must bite on a member that requested nothing: {writer}"
    );

    // `hermit` sits on no desk, so it is untouched and still holds the
    // company grant. A ceiling that leaked to non-members would be a scoping
    // bug invisible from the desk's own screen.
    let (_, hermit) = get_agent(&state, "hermit").await;
    assert!(
        strings(&hermit["tools"]["deskAllow"]).is_empty(),
        "{hermit}"
    );
    assert_eq!(
        hermit["tools"]["deskCeilingActive"], false,
        "no desk states a ceiling for hermit: {hermit}"
    );
    assert_eq!(
        strings(&hermit["tools"]["effective"]),
        vec!["workspace", "workspace.*", "composio"],
        "{hermit}"
    );
}

/// The three rows the console renders must shrink monotonically, or the card
/// would show a "ceiling" that is not one.
#[tokio::test]
async fn a_desk_ceiling_can_never_widen_past_the_company_grant() {
    // The desk names a grant the company never allowed.
    let scoped = ROSTER.replace(
        "members = [\"writer\", \"ceo\"]",
        "members = [\"writer\", \"ceo\"]\ntools = [\"shell\", \"workspace.read\"]",
    );
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), &scoped).await;

    let (_, writer) = get_agent(&state, "writer").await;
    assert!(
        !strings(&writer["tools"]["deskAllow"]).contains(&"shell".to_string()),
        "a desk cannot grant what the company withheld: {writer}"
    );
    assert!(
        !strings(&writer["tools"]["effective"]).contains(&"shell".to_string()),
        "{writer}"
    );
}

/// A desk ceiling can resolve to an **empty** narrowed list while still
/// being active: `media` is an explicit opt-in that a bare `*` does not
/// confer, so a desk naming only `media` under a company that allows `*`
/// narrows everything away. The DTO must report the ceiling active with an
/// empty `deskAllow` — a console keying on `deskAllow`'s emptiness would
/// substitute `companyAllow` and promise grants the host drops.
#[tokio::test]
async fn an_active_desk_ceiling_that_resolves_empty_is_reported_active() {
    let manifest = r#"
[company]
name = "Acme"
[policy]
mode = "full"
[tools]
allow = ["*"]

[[agent]]
id = "writer"
role = "Writer"

[[group_chat]]
id = "creative"
name = "Creative desk"
members = ["writer"]
tools = ["media"]
"#;
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), manifest).await;

    let (_, writer) = get_agent(&state, "writer").await;
    assert!(
        strings(&writer["tools"]["deskAllow"]).is_empty(),
        "media under a bare * is an explicit opt-in that narrows to nothing: {writer}"
    );
    assert_eq!(
        writer["tools"]["deskCeilingActive"], true,
        "the desk states a ceiling even though the narrowed list is empty: {writer}"
    );
    assert!(
        strings(&writer["tools"]["effective"]).is_empty(),
        "with an empty ceiling the standard grant holds nothing: {writer}"
    );
}

/// A roster that tags nobody still has an orchestrator: the first declared
/// agent. A console that read `tier` alone would call every teammate on such
/// a company a worker, and be wrong about all of them.
#[tokio::test]
async fn an_untagged_roster_still_names_an_orchestrator() {
    let home_dir = home();
    let state = state_with_manifest(
        home_dir.path(),
        "[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n\
         [[agent]]\nid = \"writer\"\nrole = \"Writer\"\n\
         [[agent]]\nid = \"editor\"\nrole = \"Editor\"\n",
    )
    .await;

    let (_, writer) = get_agent(&state, "writer").await;
    assert!(writer["tier"].is_null(), "{writer}");
    assert_eq!(writer["isOrchestrator"], true, "{writer}");

    let (_, editor) = get_agent(&state, "editor").await;
    assert_eq!(editor["isOrchestrator"], false, "{editor}");
}

/// An operator-added membership counts: the detail view resolves desks
/// through `effective_desk_members`, not through the manifest's list.
#[tokio::test]
async fn an_operator_added_desk_membership_shows_on_the_agent() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), ROSTER).await;

    assert_eq!(
        get_agent(&state, "hermit").await.1["desks"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let (status, _) = send(
        &state,
        "POST",
        "/api/v1/company/desks/content/members",
        Some(json!({"agent_id": "hermit"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, hermit) = get_agent(&state, "hermit").await;
    let desks = hermit["desks"].as_array().unwrap();
    assert_eq!(desks.len(), 1, "{hermit}");
    assert_eq!(desks[0]["id"], "content", "{hermit}");
}

/// An overlay teammate reads back with the company's standard grant and no
/// tier, which is exactly what the harness builds it with.
#[tokio::test]
async fn an_overlay_teammate_reports_the_standard_grant() {
    let home_dir = home();
    let state = state_with_manifest(home_dir.path(), ROSTER).await;
    let jamie = add_overlay(&state, "Jamie", "Growth").await;

    let (status, agent) = get_agent(&state, &jamie).await;
    assert_eq!(status, StatusCode::OK, "{agent}");
    assert_eq!(agent["source"], "overlay");
    assert_eq!(agent["name"], "Jamie");
    assert!(agent["tier"].is_null(), "{agent}");
    assert_eq!(agent["isOrchestrator"], false, "{agent}");
    assert!(agent["tools"]["requested"].is_null(), "{agent}");
    assert_eq!(
        strings(&agent["tools"]["effective"]),
        vec!["workspace", "workspace.*", "composio"],
        "{agent}"
    );
}

