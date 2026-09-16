use super::*;
// The globals-unaware readers: these tests assert the company's own two
// sources, so they call the form that resolves no baseline.
use crate::company::{list_workflows_union, load_workflow_union};

/// The listed rows this company itself has, with the global baseline
/// filtered out. Every company lists the baseline graphs; these tests are
/// about what this one created, deleted, or declared.
///
/// This is an **id heuristic**, not provenance: `WorkflowSummary` carries
/// no `global` flag, so a row is classified as "the baseline's" purely by
/// id membership in `crate::globals::workflows()`. A company definition of
/// the *same* id supersedes the global one and would be wrongly excluded
/// here — none of the fixtures below give a company workflow a colliding
/// id, so the gap does not fire in this suite; see
/// `write_test::workflow_create_of_an_id_matching_a_global_wins_by_content`
/// for that case asserted directly, without this helper.
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

const DEMO: &str = r#"
    id = "demo"
    name = "Demo flow"
    description = "A tiny trigger → agent → output graph."
    [[node]]
    id = "start"
    kind = "trigger"
    name = "Start"
    summary = "Kicks it off."
    [[node]]
    id = "worker"
    kind = "agent"
    name = "Worker"
    summary = "Does the thing."
    agent = "assistant"
    [[node]]
    id = "done"
    kind = "output"
    name = "Report"
    [[edge]]
    from = "start"
    to = "worker"
    [[edge]]
    from = "worker"
    to = "done"
    label = "ok"
"#;

/// Writes `DEMO` to `<dir>/workflows/demo.toml` and returns `dir`.
fn seed_demo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(workflows.join("demo.toml"), DEMO).unwrap();
    dir
}

/// FAIL-axis: unlike the list route above (which skips a broken graph and
/// carries on), addressing the broken one directly is a single-resource
/// read on a body that cannot be used, and `OpenCompanyError::DataParse`
/// is centrally mapped to `400` (`server/error.rs`). This is the
/// single-workflow `GET` driven all the way through the router, not just
/// the loader function, so the mapping is proven at the seam the console
/// actually calls.
#[tokio::test]
async fn getting_a_malformed_workflow_by_id_answers_400_data_parse() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::company::CompanyManifest;
    use crate::ports::CompanyStore;
    use crate::ports::types::{CompanyId, CompanyRecord};
    use crate::runtime::RuntimeBuilder;
    use crate::server::router;
    use crate::store::FsCompanyStore;
    use crate::{AppConfig, AppState};

    let dir = seed_demo();
    std::fs::write(
        dir.path().join("workflows").join("broken.toml"),
        "id = \"broken\"\nname = \n[[node]] oops",
    )
    .unwrap();

    let manifest: CompanyManifest =
        toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap();
    let store = FsCompanyStore::new(dir.path().to_path_buf());
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
    let runtime = RuntimeBuilder::new(dir.path().to_path_buf(), manifest)
        .with_id(id.clone())
        .with_seed_dir(dir.path().to_path_buf())
        .build()
        .await
        .unwrap();
    assert!(
        runtime.source_dir().is_some(),
        "test setup must give the company a real source tree to read `broken.toml` from"
    );
    let state = AppState::new(AppConfig::default());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/company/workflows/broken")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["code"], "data_parse",
        "the stable error code must name a parse failure, not a generic 500: {body}"
    );
}

// HTTP-level: a hosted tenant has no source directory to scan, so these
// exercise the manifest-enabled union path end to end via the router.
