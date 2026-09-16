use super::*;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

use crate::AppConfig;
use crate::company::CompanyManifest;
use crate::economy::signer::LocalSigner;
use crate::economy::x402::X402Challenge;
use crate::economy::{MockTinyplaceClient, TinyplaceEconomy};
use crate::ports::types::{
    CompanyId, CompressedTrace, CycleRequest, CycleResult, EventSeq, TokenUsage,
};
use crate::ports::{AgentEconomy, Brain, CompanyStore, CycleHost};
use crate::runtime::RuntimeBuilder;
use crate::store::FsCompanyStore;

const DISCOVERABLE_TOML: &str = r#"
    [company]
    name = "Acme SEO"
    output = "SEO audits"
    handle = "acme"

    [place]
    discoverable = true
    skills = [
        { id = "seo.audit", price_usd = "25.00", description = "Full audit" },
        { id = "seo.free", price_usd = "0.00" },
    ]
"#;

/// Builds an `AppState` with one discoverable company wired to a mock
/// economy, rooted at `home`, and returns the client-side signer to sign
/// inbound requests with.
async fn seeded_state(home: &std::path::Path) -> (AppState, Arc<LocalSigner>) {
    let manifest: CompanyManifest = toml::from_str(DISCOVERABLE_TOML).unwrap();
    let id = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> = Arc::new(FsCompanyStore::new(home.to_path_buf()));
    let signer = Arc::new(LocalSigner::generate());
    let mock = Arc::new(MockTinyplaceClient::new());
    let economy: Arc<dyn AgentEconomy> = Arc::new(
        TinyplaceEconomy::new(mock, signer.clone(), store.clone(), id.clone(), None)
            .going_public(true),
    );
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id)
        .with_economy(economy)
        .build()
        .await
        .unwrap();

    let state = AppState::new(AppConfig::default()).with_home(home.to_path_buf());
    state
        .registry()
        .insert(runtime.id().clone(), Arc::new(runtime));

    // The counterparty (client) signs with its own identity.
    let client_signer = Arc::new(LocalSigner::generate());
    (state, client_signer)
}

/// Same as [`seeded_state`], but the company cycle is driven by `brain`
/// instead of the default hosted one — for tests that need to control how
/// long (or how) a cycle runs.
async fn seeded_state_with_brain(
    home: &std::path::Path,
    brain: Arc<dyn Brain>,
) -> (AppState, Arc<LocalSigner>) {
    let manifest: CompanyManifest = toml::from_str(DISCOVERABLE_TOML).unwrap();
    let id = CompanyId::new("acme");
    let store: Arc<dyn CompanyStore> = Arc::new(FsCompanyStore::new(home.to_path_buf()));
    let signer = Arc::new(LocalSigner::generate());
    let mock = Arc::new(MockTinyplaceClient::new());
    let economy: Arc<dyn AgentEconomy> = Arc::new(
        TinyplaceEconomy::new(mock, signer.clone(), store.clone(), id.clone(), None)
            .going_public(true),
    );
    let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
        .with_id(id)
        .with_economy(economy)
        .with_brain(brain)
        .build()
        .await
        .unwrap();

    let state = AppState::new(AppConfig::default()).with_home(home.to_path_buf());
    state
        .registry()
        .insert(runtime.id().clone(), Arc::new(runtime));

    let client_signer = Arc::new(LocalSigner::generate());
    (state, client_signer)
}

/// A brain that never returns, so a test can prove the cycle it drives is
/// bounded by something other than the brain's own good behavior.
struct HangingBrain;

#[async_trait]
impl Brain for HangingBrain {
    async fn run_cycle(
        &self,
        _req: CycleRequest,
        _host: &dyn CycleHost,
    ) -> crate::Result<CycleResult> {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        unreachable!("the cycle timeout must fire long before this wakes")
    }
}

/// A brain that answers a cycle with nothing, cheaply — for tests that
/// only care about the transport, not what cognition produces.
struct SilentBrain;

#[async_trait]
impl Brain for SilentBrain {
    async fn run_cycle(
        &self,
        req: CycleRequest,
        _host: &dyn CycleHost,
    ) -> crate::Result<CycleResult> {
        Ok(CycleResult {
            channel_responses: Vec::new(),
            new_traces: vec![CompressedTrace::now(req.cycle_id, "silent test brain")],
            ledger_deltas: Vec::new(),
            token_usage: TokenUsage::default(),
        })
    }
}

/// Builds an `AppState` with two distinct discoverable companies, each
/// answering only its own handle — for tests of the prosumer (single-
/// company) fallback's boundary.
async fn two_company_state(home: &std::path::Path) -> AppState {
    let state = AppState::new(AppConfig::default()).with_home(home.to_path_buf());
    for handle in ["acme", "globex"] {
        let toml_src = format!(
            r#"
            [company]
            name = "{handle}"
            output = "audits"
            handle = "{handle}"

            [place]
            discoverable = true
            skills = [
                {{ id = "seo.free", price_usd = "0.00" }},
            ]
            "#
        );
        let manifest: CompanyManifest = toml::from_str(&toml_src).unwrap();
        let id = CompanyId::new(handle);
        let runtime = RuntimeBuilder::new(home.to_path_buf(), manifest)
            .with_id(id.clone())
            .with_brain(Arc::new(SilentBrain))
            .build()
            .await
            .unwrap();
        state.registry().insert(id, Arc::new(runtime));
    }
    state
}

/// Signs a POST body for `/a2a/{handle}` and returns the SIWX header value.
fn siwx_header(signer: &LocalSigner, handle: &str, body: &[u8], ts: i64) -> String {
    let hash = sha256_hex(body);
    let header = siwx::build_header(
        signer,
        &siwx::SiwxPayload {
            method: "POST",
            path: &format!("/a2a/{handle}"),
            timestamp: ts,
            body_hash: &hash,
        },
    );
    siwx::header_value(&header)
}

/// Builds a SIWX-signed `seo.audit` request carrying `auth` as its payment.
/// `site` varies the body so each request has its own SIWX signature.
fn paid_request(client: &LocalSigner, auth: &X402Authorization, site: &str) -> Request<Body> {
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": { "site": site }, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(client, "acme", &body, now_secs());
    Request::builder()
        .method("POST")
        .uri("/a2a/acme")
        .header(AUTHORIZATION, header)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

fn task_body(skill: &str) -> Vec<u8> {
    serde_json::to_vec(&JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": skill, "input": { "site": "x.com" } }),
    ))
    .unwrap()
}

#[tokio::test]
async fn siwx_invalid_inbound_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _client) = seeded_state(dir.path()).await;
    let app = router().with_state(state);

    let body = task_body("seo.free");
    // No Authorization header at all.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn priced_skill_without_payment_returns_402() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    // Our address is the on-disk signer for the company id.
    let our_id = signer_for(dir.path(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    let app = router().with_state(state);

    let body = task_body("seo.audit");
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let challenge: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(challenge["amount"], "25.00");
    assert_eq!(challenge["recipient"], our_id);
    assert_eq!(challenge["asset"], "USDC");
    assert_eq!(challenge["network"], "solana");
}

#[tokio::test]
async fn valid_signed_free_task_routes_to_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let runtime = state.registry().sole().unwrap();
    let app = router().with_state(state);

    let body = task_body("seo.free");
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(envelope["jsonrpc"], "2.0");
    assert!(envelope["result"]["cycleId"].is_string());

    // The A2aTaskReceived event was persisted by the cycle.
    let stored = runtime
        .events
        .read_from(runtime.id(), EventSeq::new(0), 10)
        .await
        .unwrap();
    assert!(stored.iter().any(|e| matches!(
        &e.event,
        CompanyEvent::A2aTaskReceived { from, .. } if from == &client.agent_id()
    )));
}

#[tokio::test]
async fn paid_skill_with_valid_x402_routes_and_journals() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let runtime = state.registry().sole().unwrap();
    let our_id = signer_for(dir.path(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    let app = router().with_state(state);

    // Build a valid x402 authorization paying the 25.00 seo.audit price.
    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": {}, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    // The inbound receipt was journaled as x402.in.
    let record = runtime.store.load(runtime.id()).await.unwrap().unwrap();
    let inflow = record
        .ledger
        .iter()
        .find(|e| e.kind == "x402.in")
        .expect("x402.in row");
    assert_eq!(inflow.amount_usd, 25.0);
}

/// The same signed authorization, presented on two different tasks. Each
/// request carries its own SIWX signature, so the transport replay cache
/// admits both; only the payment layer can refuse the second.
#[tokio::test]
async fn replayed_x402_authorization_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let our_id = signer_for(dir.path(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    let app = router().with_state(state);

    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());

    let first = paid_request(&client, &auth, "first.example");
    let response = app.clone().oneshot(first).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "first purchase is served"
    );

    let second = paid_request(&client, &auth, "second.example");
    let response = app.oneshot(second).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "the same authorization must not buy a second task"
    );
}

/// Spending one nonce must not blind the company to the next payment.
#[tokio::test]
async fn a_freshly_minted_authorization_is_admitted() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let our_id = signer_for(dir.path(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    let app = router().with_state(state);

    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };

    for site in ["first.example", "second.example"] {
        let auth = x402::authorize(&client, &challenge, now_secs());
        let request = paid_request(&client, &auth, site);
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{site} pays its own way");
    }
}

/// A skill id the card never advertises must not slip past the pricing gate
/// on a company that prices its work.
#[tokio::test]
async fn unknown_skill_id_is_refused_on_a_pricing_card() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state);

    let body = task_body("seo.ghost");
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "an unpriced, unadvertised skill must not run for free"
    );
}

#[tokio::test]
async fn paid_skill_with_wrong_recipient_is_rechallenged() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state.clone());

    // A well-formed, correctly-signed authorization that pays SOMEONE ELSE
    // (a self-dealing payer) must not buy priced work from this company.
    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: client.agent_id(), // not our company's agent id
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": {}, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // Re-challenged with a 402, not served for free.
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);

    // And the rejection must not have spent the nonce: on a multi-company
    // host, submitting a valid authorization against the wrong company's
    // handle would otherwise burn it here and reject the payer's retry
    // against the right company as a replay, even though it was never
    // accepted anywhere.
    assert!(
        state
            .x402_nonce()
            .check_and_insert(&auth.nonce, now_secs(), auth.timestamp)
            .expect("nonce cache must still answer")
    );
}

#[tokio::test]
async fn an_underpaid_authorization_does_not_spend_its_nonce() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state.clone());

    let our_id = signer_for(state.home(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    let challenge = X402Challenge {
        amount: "1.00".into(), // below seo.audit's price
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": {}, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert!(
        state
            .x402_nonce()
            .check_and_insert(&auth.nonce, now_secs(), auth.timestamp)
            .expect("nonce cache must still answer"),
        "an underpaid authorization must not burn its nonce — the payer \
         cannot fix the amount without re-signing, but nothing here \
         should have consumed it either"
    );
}

#[tokio::test]
async fn a_correctly_priced_authorization_in_the_wrong_asset_is_rechallenged() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state.clone());

    let our_id = signer_for(state.home(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    // The payer signs a fully-priced authorization, but in an asset the
    // card never priced this skill in.
    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: our_id,
        asset: "NOTUSDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": {}, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "a signed payment in the wrong asset must not buy work priced in a different one"
    );
    assert!(
        state
            .x402_nonce()
            .check_and_insert(&auth.nonce, now_secs(), auth.timestamp)
            .expect("nonce cache must still answer"),
        "the rejected authorization must not have spent its nonce"
    );
}

#[tokio::test]
async fn a_non_finite_amount_is_rechallenged_not_treated_as_paid() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state.clone());

    let our_id = signer_for(state.home(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    // `"NaN".parse::<f64>()` succeeds and every comparison against NaN is
    // false, so a naive `paid < price` underpayment check treats this as
    // sufficient. It must not be.
    let challenge = X402Challenge {
        amount: "NaN".into(),
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.audit", "input": {}, "payment": auth }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::PAYMENT_REQUIRED,
        "a non-finite claimed amount must never be treated as sufficient payment"
    );
    assert!(
        state
            .x402_nonce()
            .check_and_insert(&auth.nonce, now_secs(), auth.timestamp)
            .expect("nonce cache must still answer"),
        "the rejected authorization must not have spent its nonce"
    );
}

#[tokio::test]
async fn replayed_signature_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let app = router().with_state(state);

    let body = task_body("seo.free");
    let header = siwx_header(&client, "acme", &body, now_secs());

    let build = || {
        Request::builder()
            .method("POST")
            .uri("/a2a/acme")
            .header(AUTHORIZATION, header.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };

    let first = app.clone().oneshot(build()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    // The identical signature is rejected on replay.
    let second = app.oneshot(build()).await.unwrap();
    assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
}

fn card_pricing(skills: &[(&str, &str)]) -> AgentCard {
    AgentCard {
        payment_requirements: skills
            .iter()
            .map(|(id, price)| CardPayment {
                skill_id: (*id).to_string(),
                price: (*price).to_string(),
                asset: "USDC".into(),
                network: "solana".into(),
            })
            .collect(),
        ..AgentCard::default()
    }
}

#[test]
fn a_priced_skill_is_charged_for() {
    let card = card_pricing(&[("seo.audit", "25.00")]);
    assert!(matches!(
        classify_skill(&card, "seo.audit"),
        SkillCharge::Priced(_)
    ));
}

#[test]
fn a_zero_price_is_deliberately_free() {
    let card = card_pricing(&[("seo.audit", "25.00"), ("seo.free", "0.00")]);
    assert!(matches!(
        classify_skill(&card, "seo.free"),
        SkillCharge::Free
    ));
}

#[test]
fn an_unparsable_price_is_still_free() {
    let card = card_pricing(&[("seo.audit", "25.00"), ("seo.odd", "gratis")]);
    assert!(matches!(
        classify_skill(&card, "seo.odd"),
        SkillCharge::Free
    ));
}

#[test]
fn an_unadvertised_skill_is_unknown_not_free() {
    let card = card_pricing(&[("seo.audit", "25.00"), ("seo.free", "0.00")]);
    assert!(matches!(
        classify_skill(&card, "seo.ghost"),
        SkillCharge::Unknown
    ));
}

#[test]
fn a_card_that_prices_nothing_charges_for_nothing() {
    // A company that never opted into pricing keeps serving every id,
    // including one it does not list — refusing here would take A2A away
    // from it.
    let card = card_pricing(&[("seo.free", "0.00")]);
    assert!(matches!(
        classify_skill(&card, "seo.ghost"),
        SkillCharge::Free
    ));
    assert!(matches!(
        classify_skill(&AgentCard::default(), "seo.ghost"),
        SkillCharge::Free
    ));
}

#[test]
fn a_duplicate_id_with_a_priced_entry_is_still_charged() {
    // Manifest validation now rejects this shape outright, but the lookup
    // itself must stay safe by construction: given both a free and a
    // priced entry under the same id, in either order, the priced one
    // must win. Letting the free entry win would waive a price the
    // company does charge for that skill.
    let free_first = card_pricing(&[("seo.audit", "0.00"), ("seo.audit", "25.00")]);
    assert!(matches!(
        classify_skill(&free_first, "seo.audit"),
        SkillCharge::Priced(_)
    ));

    let priced_first = card_pricing(&[("seo.audit", "25.00"), ("seo.audit", "0.00")]);
    assert!(matches!(
        classify_skill(&priced_first, "seo.audit"),
        SkillCharge::Priced(_)
    ));
}

/// The spent-nonce set is the only thing that makes an authorization
/// single-use, so a set that cannot answer must stop the sale.
#[tokio::test]
async fn an_unusable_spent_nonce_set_refuses_a_paid_task() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let runtime = state.registry().sole().unwrap();
    let our_id = signer_for(dir.path(), &CompanyId::new("acme"))
        .await
        .unwrap()
        .agent_id();
    state.x402_nonce().poison_for_tests();
    let app = router().with_state(state);

    let challenge = X402Challenge {
        amount: "25.00".into(),
        recipient: our_id,
        asset: "USDC".into(),
        network: "solana".into(),
    };
    let auth = x402::authorize(&client, &challenge, now_secs());
    let response = app
        .oneshot(paid_request(&client, &auth, "x.com"))
        .await
        .unwrap();

    assert_ne!(
        response.status(),
        StatusCode::OK,
        "an unreadable spent-nonce set must refuse the payment"
    );
    let stored = runtime
        .events
        .read_from(runtime.id(), EventSeq::new(0), 10)
        .await
        .unwrap();
    assert!(
        !stored
            .iter()
            .any(|e| matches!(&e.event, CompanyEvent::A2aTaskReceived { .. })),
        "no task may reach cognition when the payment was refused"
    );
}

#[tokio::test]
async fn promptguard_sanitizes_control_chars_before_event() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let runtime = state.registry().sole().unwrap();
    let app = router().with_state(state);

    // A bell (0x07) and ESC (0x1b) must be stripped; newline survives.
    let rpc = JsonRpcRequest::new(
        "tasks/send",
        json!({ "skill": "seo.free", "note": "hi\u{0007}there\u{001b}\nok" }),
    );
    let body = serde_json::to_vec(&rpc).unwrap();
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = runtime
        .events
        .read_from(runtime.id(), EventSeq::new(0), 10)
        .await
        .unwrap();
    let task = stored
        .iter()
        .find_map(|e| match &e.event {
            CompanyEvent::A2aTaskReceived { task, .. } => Some(task.clone()),
            _ => None,
        })
        .expect("a2a event");
    let note = task["note"].as_str().unwrap();
    assert_eq!(note, "hithere\nok");
}

#[tokio::test]
async fn well_known_and_skill_md_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _client) = seeded_state(dir.path()).await;
    let app = router().with_state(state);

    // The platform well-known returns the card with the a2a endpoint.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/companies/acme/.well-known/agent-card.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let card: AgentCard = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(card.endpoint, "http://127.0.0.1:8080/a2a/acme");
    assert!(card.skills.contains(&"seo.audit".to_string()));

    // skill.md lists each priced skill line.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/a2a/acme/skill.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/markdown; charset=utf-8")
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let md = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(md.contains("`seo.audit` — 25.00 USDC (solana)"));
}

/// PLAT-067 / PLAT-066-067: `tasks/send` is fully synchronous, so a cycle
/// that never returns must not be able to hold the connection (and the
/// task behind it) open forever.
#[tokio::test(start_paused = true)]
async fn a_task_that_never_finishes_is_bounded_by_a_cycle_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state_with_brain(dir.path(), Arc::new(HangingBrain)).await;
    let app = router().with_state(state);

    let body = task_body("seo.free");
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::GATEWAY_TIMEOUT,
        "a company cycle that never returns must not hold the connection open forever"
    );
}

/// PLAT-067: one `tasks/send` POST must append exactly one
/// `A2aTaskReceived` event — not a batch, not a loop that could run the
/// counterparty's task more than once.
#[tokio::test]
async fn exactly_one_cycle_runs_per_inbound_task() {
    let dir = tempfile::tempdir().unwrap();
    let (state, client) = seeded_state(dir.path()).await;
    let runtime = state.registry().sole().unwrap();
    let app = router().with_state(state);

    let body = task_body("seo.free");
    let header = siwx_header(&client, "acme", &body, now_secs());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/a2a/acme")
                .header(AUTHORIZATION, header)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = runtime
        .events
        .read_from(runtime.id(), EventSeq::new(0), 10)
        .await
        .unwrap();
    let received = stored
        .iter()
        .filter(|e| matches!(&e.event, CompanyEvent::A2aTaskReceived { .. }))
        .count();
    assert_eq!(
        received, 1,
        "one POST to tasks/send must append exactly one A2aTaskReceived event: {stored:?}"
    );
}

/// PLAT-066: the prosumer fallback is scoped to a genuinely sole company.
/// With two companies registered, an unmatched handle must 404 rather than
/// silently answering as either of them.
#[tokio::test]
async fn the_prosumer_fallback_does_not_fire_when_more_than_one_company_is_registered() {
    let dir = tempfile::tempdir().unwrap();
    let state = two_company_state(dir.path()).await;
    let app = router().with_state(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/a2a/nonexistent-handle/skill.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "with two companies registered, an unmatched handle must not resolve to either"
    );
}

/// PLAT-066-067: this IS the SIWX design — a self-issued identity, not an
/// allow-listed one. Two independently generated keypairs, neither ever
/// provisioned or seen before, must each transact on their very first
/// request.
#[tokio::test]
async fn two_independent_strangers_each_transact_without_prior_registration() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _seed_client) = seeded_state(dir.path()).await;
    let app = router().with_state(state);

    for _ in 0..2 {
        let stranger = LocalSigner::generate();
        let body = task_body("seo.free");
        let header = siwx_header(&stranger, "acme", &body, now_secs());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/a2a/acme")
                    .header(AUTHORIZATION, header)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a freshly generated, never-before-seen keypair must transact on its first request"
        );
    }
}
