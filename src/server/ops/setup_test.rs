//! HTTP-level tests for `POST {scope}/setup/roster` (HT-124).

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

fn home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oc-setup-")
        .tempdir()
        .expect("tempdir")
}

fn manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n").unwrap()
}

async fn state_with(home: &std::path::Path) -> AppState {
    let store = FsCompanyStore::new(home.to_path_buf());
    let id = CompanyId::new("acme");
    store
        .save(&CompanyRecord {
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
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest())
        .with_id(id.clone())
        .build()
        .await
        .unwrap();
    let state = AppState::new(AppConfig::default()).with_home(home.to_path_buf());
    state.registry().insert(id, std::sync::Arc::new(runtime));
    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    state
}

fn roster_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/company/setup/roster")
        .header("cookie", crate::server::test_support::fixed_cookie("acme"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"industry":"bakery","team_hint":"","automate":"orders"}"#,
        ))
        .unwrap()
}

/// The route has no rate limit and no in-flight/re-entry guard (HT-124):
/// every one of N back-to-back calls reaches `build_proposal` (a real charged
/// model pass in production) and returns 200. A route that pays for inference
/// per call must refuse past some per-company burst rate, not accept an
/// unbounded rerun.
#[tokio::test]
#[ignore = "confirms fail-open: propose_roster has no rate limit — an \
            authenticated operator can re-trigger the paid model pass \
            unboundedly (HT-124), no cap enforced anywhere on this route"]
async fn repeated_roster_proposals_are_rate_limited() {
    let home_dir = home();
    let state = state_with(home_dir.path()).await;
    let app = router(state);

    const BURST: usize = 20;
    let mut statuses = Vec::with_capacity(BURST);
    for _ in 0..BURST {
        let response = app.clone().oneshot(roster_request()).await.unwrap();
        statuses.push(response.status());
    }

    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "expected at least one 429 across {BURST} rapid calls from the same \
         company, got: {statuses:?}"
    );
}

/// Pinned as today's actual behaviour so the finding above isn't mistaken for
/// a broken test: every call really does complete and return a real proposal.
#[tokio::test]
async fn today_every_rapid_call_succeeds_uncapped() {
    let home_dir = home();
    let state = state_with(home_dir.path()).await;
    let app = router(state);

    const BURST: usize = 5;
    for _ in 0..BURST {
        let response = app.clone().oneshot(roster_request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value["agents"].is_array());
    }
}
