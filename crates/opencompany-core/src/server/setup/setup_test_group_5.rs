//! Setup tests for the onboarding redesign's two credential paths: the managed
//! branch's account key (slice 4a) and the self-managed branch's provider
//! (slice 4b-i).
//!
//! Its own group because both are about what the **apply** does after the seed,
//! which is a different question from every other group here — those ask what
//! the wizard collects and what the manifest ends up saying, and these ask
//! which host function ran and what it left in the company's own stores.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::company::runtime::CompanyRuntime;
use crate::ports::types::{CompanyId, SecretValue};
use crate::server::router;
use crate::{AppConfig, AppState};

use super::setup_test_support_1::*;

/// Reads one of a company's secrets, or `None` when it holds nothing.
async fn secret(runtime: &CompanyRuntime, key: &str) -> Option<String> {
    runtime
        .secrets()
        .get(runtime.id(), key)
        .await
        .unwrap()
        .map(|SecretValue(value)| value)
}

/// A key shaped like a real one and worth nothing.
const ACCOUNT_KEY: &str = "th-not-a-real-key";

/// The wizard's managed branch collects the **company's** TinyHumans account
/// key, and the apply runs it through the same fan-out `PUT …/credential`
/// does.
///
/// This used to be two features wearing one name. Connecting TinyHumans on the
/// Account page wrote the account key and fanned it out — the Composio copy,
/// the LLM copy, the `tinyhumans` row, the default. Connecting TinyHumans in
/// onboarding wrote the instance-wide `tinyhumans_api_key` setting, which
/// fills none of those and is read once at boot. The console cannot make the
/// real call itself (`PUT …/credential` is admin-scoped to a company, and
/// during first run there is neither a company nor anyone signed in), so the
/// key rides the apply and the fan-out runs here, right after the company
/// exists.
///
/// The rebuild-in-place half is not asserted here and cannot be: this build
/// has no harness compiled in, so `harness_reachable` is false and there is
/// genuinely nothing to rebuild a company onto. It is reached through the same
/// `rebuild_if_pending` the Account page's save calls, which
/// `put_credential_that_configures_inference_rebuilds_the_runtime_in_place`
/// covers on a fixture
/// that does have a pool.
#[tokio::test]
async fn the_wizards_account_key_fans_out_onto_the_company_it_seeds() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());
    // Keeps the fan-out's health probe off `api.tinyhumans.ai`. Keyed on the
    // id the chosen name mints, which is the one this apply is about to seed.
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
    assert!(
        !body.to_string().contains(ACCOUNT_KEY),
        "the apply response must never echo the key: {body}"
    );

    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("the seeded company is registered");

    assert_eq!(
        secret(&runtime, crate::company::company_key::KEY_KEY).await,
        Some(ACCOUNT_KEY.to_string()),
        "the key belongs to the company, in the slot the Account page writes"
    );
    // The load-bearing one: only the fan-out writes this. A wizard that stored
    // the key and stopped there leaves it empty, which is the state where
    // "connected to TinyHumans" buys the operator no integrations at all.
    assert_eq!(
        secret(&runtime, crate::company::composio::TINYHUMANS_KEY_KEY).await,
        Some(ACCOUNT_KEY.to_string()),
        "the Composio copy must have been filled from it"
    );
    assert_eq!(
        secret(
            &runtime,
            &crate::company::inference::store::provider_key_key(
                crate::company::inference::MANAGED_SLUG
            )
        )
        .await,
        Some(ACCOUNT_KEY.to_string()),
        "and the LLM copy with it"
    );

    // The note is the host's own account of all of that, for the completion
    // screen to show verbatim rather than flatten into "you're set up".
    let note = body["credential_note"]
        .as_str()
        .unwrap_or_else(|| panic!("the fan-out's own words must come back: {body}"));
    assert!(note.contains("acme/test-model"), "{note}");
}

/// A key sent with nothing to attach it to is dropped, not guessed at.
///
/// A host that already has companies seeds none, and there is no one of its
/// existing companies this wizard can claim the operator meant — writing the
/// key onto whichever happened to be first would hand one company another's
/// wallet.
#[tokio::test]
async fn an_account_key_with_no_company_to_own_it_is_not_written_anywhere() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());
    let existing = with_company(&state, home_dir.path()).await;

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({
            "fields": {},
            "template": "law_firm",
            "tinyhumans_key": ACCOUNT_KEY,
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["seeded_company"].is_null(), "{body}");
    assert!(
        body["credential_note"].is_null(),
        "nothing happened, so nothing is claimed: {body}"
    );

    let runtime = state.registry().get(&existing).expect("still registered");
    assert_eq!(
        secret(&runtime, crate::company::company_key::KEY_KEY).await,
        None,
        "a company this wizard did not create must not be given a wallet"
    );
}

/// A key shaped like a real one and worth nothing.
const PROVIDER_KEY: &str = "sk-not-a-real-key";

/// A local endpoint nothing is listening on, so the add's probe fails the
/// non-destructive way rather than dialling anyone.
const DRAFT_ENDPOINT: &str = "http://127.0.0.1:1/v1";

/// The wizard's self-managed branch connects a provider, and the apply runs it
/// through the same `add_provider` the LLM page's own add runs.
///
/// The row and the key are the visible half. The **default** is the half that
/// says which function wrote them: decision X1 makes the first provider a
/// company ever connects its default, and it lives inside `add_provider` — a
/// hand-written flush of `put_provider` plus a secret set would land the row
/// and the key exactly as below and leave the company with no default at all,
/// which is a company whose agents have nothing to route to.
#[tokio::test]
async fn the_wizards_connected_provider_is_added_through_the_real_add() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({
            "fields": {},
            "template": "law_firm",
            "name": "Acme",
            "provider_draft": {
                "kind": "custom",
                "label": "Acme Models",
                "baseUrl": DRAFT_ENDPOINT,
                "key": PROVIDER_KEY,
                "model": "acme/test-model",
                "addAnyway": true,
            },
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["seeded_company"], "acme", "{body}");
    assert!(
        !body.to_string().contains(PROVIDER_KEY),
        "the apply response must never echo the key: {body}"
    );

    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("the seeded company is registered");
    let secrets = runtime.secrets();

    let providers =
        crate::company::inference::store::list_providers(runtime.id(), secrets.as_ref())
            .await
            .unwrap();
    let row = providers
        .iter()
        .find(|p| p.slug == "acme-models")
        .unwrap_or_else(|| panic!("no row was added: {providers:?}"));
    assert!(
        matches!(
            row.model(),
            crate::company::inference::store::ModelOnRow::One(ref model)
                if model == "acme/test-model"
        ),
        "the row carries the model the operator chose: {:?}",
        row.model()
    );
    assert_eq!(
        secret(
            &runtime,
            &crate::company::inference::store::provider_key_key(&row.slug)
        )
        .await,
        Some(PROVIDER_KEY.to_string()),
        "the credential belongs to the row, in the slot the LLM page writes"
    );

    // The load-bearing one. Only `add_provider` decides this (decision X1), so
    // a flush that wrote the row itself leaves it `Unset`.
    let default = crate::company::inference::store::load_default(runtime.id(), secrets.as_ref())
        .await
        .unwrap();
    assert_eq!(
        default
            .full()
            .map(|choice| (choice.provider.as_str(), choice.model.as_str())),
        Some(("acme-models", "acme/test-model")),
        "the first provider a company ever connects becomes its default: {default:?}"
    );

    let note = body["provider_note"]
        .as_str()
        .unwrap_or_else(|| panic!("the add's own words must come back: {body}"));
    assert!(note.contains("Acme Models"), "{note}");
}

/// A provider with nothing to attach it to is dropped, not guessed at — the
/// same rule the account key follows, for the same reason: on a host that
/// already had companies there is none of them this wizard can claim the
/// operator meant.
#[tokio::test]
async fn a_provider_draft_with_no_company_to_own_it_is_not_written_anywhere() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());
    let existing = with_company(&state, home_dir.path()).await;

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({
            "fields": {},
            "template": "law_firm",
            "provider_draft": {
                "kind": "custom",
                "label": "Acme Models",
                "baseUrl": DRAFT_ENDPOINT,
                "key": PROVIDER_KEY,
                "model": "acme/test-model",
                "addAnyway": true,
            },
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["seeded_company"].is_null(), "{body}");
    assert!(
        body["provider_note"].is_null(),
        "nothing happened, so nothing is claimed: {body}"
    );

    let runtime = state.registry().get(&existing).expect("still registered");
    let providers =
        crate::company::inference::store::list_providers(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap();
    assert!(
        providers.is_empty(),
        "a company this wizard did not create must not be given a provider: {providers:?}"
    );
}

/// No draft, no write. An apply that carries none must leave the seeded
/// company's provider list exactly as the seed left it, and claim nothing.
#[tokio::test]
async fn an_apply_with_no_provider_draft_adds_nothing_and_says_nothing() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({ "fields": {}, "template": "law_firm", "name": "Acme" }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["provider_note"].is_null(), "{body}");

    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("the seeded company is registered");
    let providers =
        crate::company::inference::store::list_providers(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap();
    assert!(providers.is_empty(), "{providers:?}");
}

/// TinyHumans picked on the **self-managed** branch goes through the same add
/// as anything else, including its slot guard.
///
/// `provider/tinyhumans/key` is shared with the account-key fan-out, so the add
/// reads whatever is there before it writes and puts it back on any rollback.
/// This pins that a wizard-side add lands in that slot rather than beside it.
#[tokio::test]
async fn tinyhumans_connected_on_the_self_managed_branch_lands_in_the_shared_slot() {
    let home_dir = home();
    // The TinyHumans row's endpoint is the configured proxy, so this points it
    // at a closed local port: the add's probe then fails as transport, which is
    // not a class that rolls a cloud provider back, and no test ever dials the
    // real hub.
    let state = AppState::new(AppConfig {
        bind: "127.0.0.1:8080".to_string(),
        api_url: "http://127.0.0.1:1".to_string(),
        ..AppConfig::default()
    })
    .with_home(home_dir.path().to_path_buf());
    // The account key's own fan-out runs first and writes the same slot, which
    // is the state the add has to read and replace rather than write beside.
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
            "provider_draft": {
                "kind": crate::company::inference::MANAGED_SLUG,
                "key": PROVIDER_KEY,
                "model": "acme/test-model",
                "addAnyway": true,
            },
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("the seeded company is registered");

    assert_eq!(
        secret(
            &runtime,
            &crate::company::inference::store::provider_key_key(
                crate::company::inference::MANAGED_SLUG
            )
        )
        .await,
        Some(PROVIDER_KEY.to_string()),
        "the row's key belongs in the slot the fan-out shares, not beside it"
    );
    // One slot, one occupant. The add read the fan-out's copy as its
    // `previous_key` and replaced it; a wizard-side write that missed the slot
    // would leave the account key here and the row's credential nowhere.
    assert_ne!(
        secret(
            &runtime,
            &crate::company::inference::store::provider_key_key(
                crate::company::inference::MANAGED_SLUG
            )
        )
        .await,
        Some(ACCOUNT_KEY.to_string()),
    );
    // And it is a row, not the bare key the deprecated managed-key route
    // leaves behind — the difference between a provider the LLM page can show
    // and a credential nothing owns.
    let providers =
        crate::company::inference::store::list_providers(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap();
    assert!(
        providers
            .iter()
            .any(|p| p.slug == crate::company::inference::MANAGED_SLUG),
        "{providers:?}"
    );
}

/// The draft probe is behind the same gate every other setup route is.
///
/// It widens the first-run surface by one outward dial, so the gate is the
/// whole of what keeps it honest: a routable host must refuse it anonymously,
/// exactly as it refuses the read and the apply.
#[tokio::test]
async fn the_draft_probe_is_refused_on_a_routable_host() {
    let home_dir = home();
    let response = router(routable_state(home_dir.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup/inference/probe")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "baseUrl": DRAFT_ENDPOINT }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a routable host must not let an anonymous caller dial an address it names"
    );
}

/// And it keeps its own refusal, which is not about authority at all: an
/// endpoint carrying userinfo is refused before any request is made, because
/// this host would otherwise put a basic-auth credential on the wire to an
/// address the caller chose.
#[tokio::test]
async fn the_draft_probe_refuses_an_endpoint_carrying_a_credential() {
    let home_dir = home();
    let response = router(fresh_state(home_dir.path()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup/inference/probe")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "baseUrl": "http://alice:pw@127.0.0.1:1/v1" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A refused add is **reported**, not raised.
///
/// The company is already seeded by the time the add runs, so failing the apply
/// over a draft would leave the operator with a built company behind an error
/// screen and no way back into the wizard. The refusal rides `provider_note`
/// instead, and the rest of the apply stands.
///
/// Driven through the one refusal reachable without a network: a draft with no
/// model, which `store::check_model_id` refuses before any write.
#[tokio::test]
async fn a_refused_provider_is_reported_rather_than_failing_the_apply() {
    let home_dir = home();
    let state = fresh_state(home_dir.path());

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({
            "fields": {},
            "template": "law_firm",
            "name": "Acme",
            "provider_draft": {
                "kind": "custom",
                "label": "Acme Models",
                "baseUrl": DRAFT_ENDPOINT,
                "key": PROVIDER_KEY,
            },
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "the apply still succeeds: {body}");
    assert_eq!(body["seeded_company"], "acme", "{body}");
    let note = body["provider_note"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal must be said: {body}"));
    assert!(
        !note.contains("invalid request"),
        "the envelope's own vocabulary is not for a person: {note}"
    );

    // And nothing half-applied: the add refuses before any write.
    let runtime = state
        .registry()
        .get(&CompanyId::new("acme"))
        .expect("the seeded company is registered");
    let providers =
        crate::company::inference::store::list_providers(runtime.id(), runtime.secrets().as_ref())
            .await
            .unwrap();
    assert!(providers.is_empty(), "{providers:?}");
    assert_eq!(
        secret(
            &runtime,
            &crate::company::inference::store::provider_key_key("acme-models")
        )
        .await,
        None,
        "a refused add must leave no orphaned credential"
    );
}
