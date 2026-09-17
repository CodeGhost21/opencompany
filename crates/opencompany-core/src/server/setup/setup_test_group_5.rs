#[cfg(feature = "openhuman")]
use std::sync::Arc;

#[cfg(feature = "openhuman")]
use async_trait::async_trait;
#[cfg(feature = "openhuman")]
use axum::body::Body;
#[cfg(feature = "openhuman")]
use axum::http::Request;
use axum::http::StatusCode;
#[cfg(feature = "openhuman")]
use tower::ServiceExt;

#[cfg(feature = "openhuman")]
use crate::AppState;
#[cfg(feature = "openhuman")]
use crate::company::runtime::CompanyRuntime;
use crate::ports::types::CompanyId;
#[cfg(feature = "openhuman")]
use crate::runtime::builder::RuntimeBuilder;
#[cfg(feature = "openhuman")]
use crate::runtime::rebuild::{RebuildRequest, RuntimeRebuilder};
#[cfg(feature = "openhuman")]
use crate::server::router;

use super::setup_test_support_1::*;

const ACCOUNT_KEY: &str = "th-not-a-real-key";

/// A brain on the harness cognition path, so a rebuilt company reads as
/// "thinking" rather than "echo".
#[cfg(feature = "openhuman")]
struct RebuiltBrain;

#[cfg(feature = "openhuman")]
#[async_trait]
impl crate::ports::brain::Brain for RebuiltBrain {
    async fn run_cycle(
        &self,
        _req: crate::ports::types::CycleRequest,
        _host: &dyn crate::ports::brain::CycleHost,
    ) -> crate::Result<crate::ports::types::CycleResult> {
        Ok(crate::ports::types::CycleResult {
            channel_responses: Vec::new(),
            new_traces: Vec::new(),
            ledger_deltas: Vec::new(),
            token_usage: crate::ports::types::TokenUsage::default(),
        })
    }

    fn cognition(&self) -> crate::ports::Cognition {
        crate::ports::Cognition {
            path: crate::ports::brain::HARNESS_PATH,
            provider: "stub",
            model: None,
            metering: crate::ports::UsageMetering::PerTurn,
        }
    }
}

/// Records which companies it was asked to rebuild, and hands back a
/// successor on the harness path.
#[cfg(feature = "openhuman")]
struct RecordingRebuilder {
    home: std::path::PathBuf,
    rebuilt: Arc<std::sync::Mutex<Vec<String>>>,
}

#[cfg(feature = "openhuman")]
#[async_trait]
impl RuntimeRebuilder for RecordingRebuilder {
    async fn rebuild(
        &self,
        _state: &AppState,
        request: RebuildRequest,
    ) -> crate::Result<CompanyRuntime> {
        self.rebuilt
            .lock()
            .unwrap()
            .push(request.id.as_ref().to_string());
        RuntimeBuilder::new(self.home.clone(), request.manifest)
            .with_id(request.id)
            .with_harness(Arc::new(crate::harness::HarnessPool::new()))
            .with_brain(Arc::new(RebuiltBrain))
            .with_handover(request.handover)
            .build()
            .await
    }
}

/// The wizard's account key rebuilds the company the same apply just seeded,
/// so the operator's first chat thinks rather than echoing behind a
/// "restart required" notice nobody asked for.
///
/// Only under `openhuman` does the seed attach a harness pool, so only there
/// is the rebuild the thing that moves the company off the echo brain.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn the_wizards_account_key_rebuilds_the_company_it_just_seeded() {
    let home_dir = home();
    let rebuilt: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let state = fresh_state(home_dir.path()).with_rebuilder(Arc::new(RecordingRebuilder {
        home: home_dir.path().to_path_buf(),
        rebuilt: rebuilt.clone(),
    }));
    crate::server::ops::company_key::prober_override::set(
        "acme",
        Ok(vec!["acme/test-model".to_string()]),
    );

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({
            "fields": {},
            "template": "law_firm",
            "name": "Acme",
            "tinyhumans_key": ACCOUNT_KEY,
            "tinyhumans_model": "acme/test-model",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["seeded_company"], "acme", "{body}");

    assert_eq!(
        rebuilt.lock().unwrap().as_slice(),
        ["acme".to_string()],
        "the apply must rebuild the company it seeded. If this is the only assertion \
         failing, check the environment: OPENCOMPANY_INFERENCE_KEY, TINYHUMANS_API_KEY \
         or an existing TINYHUMANS_TOKEN_FILE each boot that company already configured \
         on the harness, which genuinely owes no rebuild — the test is wrong about the \
         shell, not about the code. They are not cleared here because `set_var` is \
         process-global and would race every other test in this binary."
    );

    crate::server::test_support::seed_fixed_admin(&state, "acme").await;
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/company/inference")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let dto = body_json(response).await;
    assert_eq!(dto["cognition"], "harness", "{dto}");
    assert_eq!(dto["restartRequired"], false, "{dto}");
    assert_eq!(dto["defaultChoice"]["provider"], "tinyhumans", "{dto}");
}

/// A wizard company that brought its own provider writes `[inference].provider`
/// before the seed, so it boots already configured instead of onto echo — and
/// owes no rebuild.
#[tokio::test]
async fn a_wizard_company_that_brought_its_own_provider_boots_ready() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());
    let mut company = designed_company(None);
    company["inference"] = serde_json::json!({
        "provider": "openrouter",
        "model": "acme/test-model",
        "key": ACCOUNT_KEY,
    });

    let (status, body) = post_setup(state.clone(), serde_json::json!({ "company": company })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seeded = body["seeded_company"].as_str().expect("seeded");

    assert_eq!(
        seeded_manifest(home_dir.path(), seeded)
            .await
            .inference
            .provider
            .as_deref(),
        Some("openrouter"),
        "the manifest this company booted from must already name its provider"
    );

    let runtime = state
        .registry()
        .get(&CompanyId::new(seeded))
        .expect("the seeded company is registered");
    assert!(
        crate::company::inference::key_configured(runtime.id(), runtime.secrets().as_ref(), None)
            .await
            .unwrap(),
        "and hold the key the wizard collected for it"
    );
}
