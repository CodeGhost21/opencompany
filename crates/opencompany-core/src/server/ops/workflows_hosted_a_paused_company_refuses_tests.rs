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

/// **A paused company refuses Run**, the same way chat already refuses.
///
/// Every background path gates on `ensure_running` — the workflow
/// scheduler, the task scheduler, the mailbox poller, A2A, chat. This
/// route did not, so the one surface the operator drives themselves was
/// the one that ignored the pause: the console promises "Pause stops
/// this company taking new work", and pressing Run started a real billed
/// run anyway.
#[tokio::test]
async fn a_paused_company_refuses_to_start_a_run() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_hosted_company_lifecycle(&home, "paused").await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/workflows/demo/run")
                .header("content-type", "application/json")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Pinned to the exact status AND code, not merely "not 200"
    // (CodeRabbit on this PR): a runner-gap 404 or an unrelated 409
    // would satisfy `assert_ne!(.., OK)` while proving nothing about the
    // lifecycle gate this test exists for — and the gate sits above the
    // runner lookup precisely so the two cannot be confused.
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "a paused company must refuse the run as a lifecycle conflict"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["code"], "lifecycle_conflict",
        "the console triages on the structured code, never the prose: {body}"
    );
    let rendered = body.to_string();
    assert!(
        rendered.contains("paused"),
        "the refusal must name the lifecycle that caused it: {rendered}"
    );
}

#[tokio::test]
async fn manifest_enabled_workflow_lists_with_no_source_dir() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_hosted_company(&home).await;

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/company/workflows")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    // Regression for #70: the REST list used to scan the filesystem
    // only, so a hosted tenant with no source dir always got `[]`
    // here even though its manifest declared an enabled workflow.
    let items = own_rows(&body);
    assert_eq!(items.len(), 1, "body: {body}");
    assert_eq!(items[0]["id"], "demo");
    // No file to load a real name from, so the id is the fallback
    // name — same fallback the GraphQL `Company.workflows` resolver
    // uses for the same case.
    assert_eq!(items[0]["name"], "demo");
}

/// A hosted tenant's record with no manifest-enabled workflows — the
/// blank-slate a real tenant starts from before it creates anything.
fn empty_manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
}

/// Same as [`state_with_hosted_company`] but with nothing enabled, and
/// returning the store so a test can rebuild state from it.
async fn hosted_state(home: &std::path::Path) -> (AppState, FsCompanyStore, CompanyId) {
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

/// Builds an `AppState` whose runtime for `id` has **no source
/// directory** — the hosted shape — over the store rooted at `home`.
///
/// `seed_admin` seeds the fixed admin + session; a *rebuild* over the
/// same home must pass `false` (the durable user store already has that
/// admin, and its session survives with it).
async fn state_over(home: &std::path::Path, id: &CompanyId, seed_admin: bool) -> AppState {
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

/// The graph body the console posts.
fn create_body() -> serde_json::Value {
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

fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> Request<Body> {
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

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ------------------------------------------------------------------
// `POST …/workflows/validate` — the author-time verdict, no save (#1074)
// ------------------------------------------------------------------

async fn post_validate(state: &AppState, body: serde_json::Value) -> axum::response::Response {
    router(state.clone())
        .oneshot(request(
            "POST",
            "/api/v1/company/workflows/validate",
            Some(body),
        ))
        .await
        .unwrap()
}

async fn post_create_on(state: &AppState, body: serde_json::Value) -> axum::response::Response {
    router(state.clone())
        .oneshot(request("POST", "/api/v1/company/workflows", Some(body)))
        .await
        .unwrap()
}

/// `create_body` with an extra node nothing points at — the reachability
/// rule (`crate::company::workflow_file`), which is one of the two a
/// client cannot pre-empt without re-implementing it.
fn body_with_an_unreachable_node() -> serde_json::Value {
    let mut body = create_body();
    body["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(
            { "id": "orphan", "kind": "output", "name": "Orphan" }
        ));
    body
}

/// `create_body` with a `condition` node whose branch carries `label`,
/// and `onError` set on the condition when `on_error` is given.
fn body_with_condition(label: &str, on_error: Option<&str>) -> serde_json::Value {
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

/// **The point of the route.** A draft validate accepts is one create
/// accepts, and nothing is persisted in between — so the console can ask
/// before it submits and get the answer the submit would give.
#[tokio::test]
async fn a_draft_validate_accepts_is_one_create_accepts_and_validate_saves_nothing() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let validated = post_validate(&state, create_body()).await;
    assert_eq!(validated.status(), StatusCode::OK);
    assert_eq!(json_body(validated).await["valid"], serde_json::json!(true));

    // Nothing was written: the graph is not in the list yet.
    let listed = router(state.clone())
        .oneshot(request("GET", "/api/v1/company/workflows", None))
        .await
        .unwrap();
    let rows = json_body(listed).await;
    assert!(
        own_rows(&rows).is_empty(),
        "validate must persist nothing, but the list shows {rows}"
    );

    // And the very same body is accepted by create.
    let created = post_create_on(&state, create_body()).await;
    assert_eq!(created.status(), StatusCode::OK, "create must agree");
}

/// An unreachable node is refused by validate in **exactly** the words
/// and status create refuses it with — the same error value, so a console
/// renders one code path for both. This is the rule a client-side BFS
/// would have had to mirror.
#[tokio::test]
async fn validate_refuses_an_unreachable_node_exactly_as_create_does() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let validated = post_validate(&state, body_with_an_unreachable_node()).await;
    let created = post_create_on(&state, body_with_an_unreachable_node()).await;

    assert_eq!(validated.status(), StatusCode::BAD_REQUEST);
    assert_eq!(created.status(), validated.status());
    let from_validate = json_body(validated).await;
    let from_create = json_body(created).await;
    assert_eq!(
        from_validate, from_create,
        "the pre-flight must answer with the submit's own body"
    );
    assert!(
        from_validate["error"]
            .as_str()
            .unwrap_or_default()
            .contains("cannot be reached from any `trigger`"),
        "{from_validate}"
    );
}

/// The condition branch-label rule, the other one a client cannot
/// pre-empt without owning a copy of it. Same status, same body.
#[tokio::test]
async fn validate_refuses_an_illegal_condition_label_exactly_as_create_does() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    let body = body_with_condition("maybe", None);
    let validated = post_validate(&state, body.clone()).await;
    let created = post_create_on(&state, body).await;

    assert_eq!(validated.status(), StatusCode::BAD_REQUEST);
    assert_eq!(created.status(), validated.status());
    let from_validate = json_body(validated).await;
    assert_eq!(
        from_validate,
        json_body(created).await,
        "the pre-flight must answer with the submit's own body"
    );
    assert!(
        from_validate["error"]
            .as_str()
            .unwrap_or_default()
            .contains("must be labeled `yes` or `no`"),
        "{from_validate}"
    );
}

/// `yes` passes, and so does the `error` branch of a condition that is
/// also `on_error = "route"` — the narrow exception. Pinned here because
/// it is the part of the rule a hand-written client check gets wrong, and
/// the route is what makes a client not need one.
#[tokio::test]
async fn validate_accepts_yes_and_the_error_branch_of_a_route_condition() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    for (label, on_error) in [("yes", None), ("error", Some("route"))] {
        let response = post_validate(&state, body_with_condition(label, on_error)).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "label `{label}` with onError {on_error:?} must be accepted: {}",
            json_body(response).await
        );
    }

    // `error` WITHOUT `on_error = "route"` is the case the exception does
    // not cover, and it must still be refused.
    let response = post_validate(&state, body_with_condition("error", None)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A draft that breaks a **record** rule and a **graph** rule at once
/// must be refused by validate for the same one create refuses it for.
///
/// `courtesy_validate_draft` used to run `parse_workflow` before
/// `validate_draft_against_record`, the reverse of
/// `create_company_workflow`, so this graph was refused here for the
/// unreachable node and there for the condition label — the same verdict
/// in a different sentence. Harmless while the only caller was the
/// builder pass; not harmless once a pre-flight route quotes it to an
/// author, because they would fix the problem they were shown and be
/// refused again for the other one.
#[tokio::test]
async fn a_doubly_invalid_draft_is_refused_for_the_same_reason_on_both_routes() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    // Illegal condition label (a record-side rule) AND a node nothing
    // points at (a graph-side rule).
    let mut body = body_with_condition("maybe", None);
    body["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!(
            { "id": "orphan", "kind": "output", "name": "Orphan" }
        ));

    let validated = post_validate(&state, body.clone()).await;
    let created = post_create_on(&state, body).await;

    assert_eq!(validated.status(), StatusCode::BAD_REQUEST);
    assert_eq!(created.status(), validated.status());
    assert_eq!(
        json_body(validated).await,
        json_body(created).await,
        "a pre-flight that names a different problem than the submit is worse than none"
    );
}

/// A company whose runtime HAS a source directory holding one seed
/// workflow at `workflows/child.toml` — the self-hosted / local `serve`
/// shape. `hosted_state` deliberately has none, so the seed-file half of
/// the `sub_workflow` existence probe is unreachable from it.
async fn seeded_state(home: &std::path::Path) -> (AppState, tempfile::TempDir) {
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

/// A graph whose `sub_workflow` node runs `child` — which exists ONLY as
/// a seed file, so the probe can only see it with a source directory.
fn body_with_sub_workflow() -> serde_json::Value {
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

/// **The review finding on #1074.** `courtesy_validate_draft` passed
/// `None` for `source_dir` while create passes
/// `company.runtime.source_dir()`. That argument feeds exactly one rule —
/// `workflow_id_exists` → `seed_file_exists` — so a draft naming a
/// seed-file child was refused 400 by the pre-flight and accepted 200 by
/// the submit.
///
/// A different **verdict**, not a different sentence: strictly worse than
/// the divergence the reorder fixed, and the exact failure a pre-flight
/// exists to prevent. Hosted tenants (`source_dir == None`) never saw it;
/// self-hosted and local `serve` from a company repo did.
#[tokio::test]
async fn validate_accepts_a_seed_file_sub_workflow_because_create_does() {
    let home_dir = home();
    let (state, _source) = seeded_state(home_dir.path()).await;

    let validated = post_validate(&state, body_with_sub_workflow()).await;
    let status = validated.status();
    let body = json_body(validated).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the pre-flight refused a child create accepts: {body}"
    );

    let created = post_create_on(&state, body_with_sub_workflow()).await;
    assert_eq!(
        created.status(),
        StatusCode::OK,
        "create must accept it, or this test proves nothing"
    );
}

/// The other half: a `sub_workflow` naming nothing at all is still
/// refused, and by both routes. Threading the source directory must widen
/// what the probe can see, not switch it off.
#[tokio::test]
async fn validate_still_refuses_a_sub_workflow_that_names_nothing() {
    let home_dir = home();
    let (state, _source) = seeded_state(home_dir.path()).await;

    let mut body = body_with_sub_workflow();
    body["nodes"][1]["config"]["workflow_id"] = serde_json::json!("ghost");

    let validated = post_validate(&state, body.clone()).await;
    let created = post_create_on(&state, body).await;
    assert_eq!(validated.status(), StatusCode::BAD_REQUEST);
    assert_eq!(created.status(), validated.status());
    assert_eq!(
        json_body(validated).await,
        json_body(created).await,
        "the pre-flight must answer with the submit's own body"
    );
}

/// The over-cap refusal, which the two routes used to word differently
/// ("the proposed workflow is N bytes" here, "the rendered workflow is N
/// bytes" there). Same status and verdict, different sentence — the drift
/// this PR set out to remove, and its own body claims the bodies are
/// identical. One constructor now, and this is what holds it.
#[tokio::test]
async fn validate_and_create_refuse_an_over_cap_draft_in_the_same_words() {
    let home_dir = home();
    let (state, _store, _id) = hosted_state(home_dir.path()).await;

    // Well past the 64 KiB TOML cap once rendered, and structurally fine
    // otherwise, so the cap is the only thing either route can complain
    // about.
    let mut body = create_body();
    body["description"] = serde_json::json!("x".repeat(70_000));

    let validated = post_validate(&state, body.clone()).await;
    let created = post_create_on(&state, body).await;

    assert_eq!(validated.status(), StatusCode::BAD_REQUEST);
    assert_eq!(created.status(), validated.status());
    let from_validate = json_body(validated).await;
    assert_eq!(
        from_validate,
        json_body(created).await,
        "the pre-flight must answer with the submit's own body"
    );
    assert!(
        from_validate["error"]
            .as_str()
            .unwrap_or_default()
            .contains("over the"),
        "{from_validate}"
    );
}
