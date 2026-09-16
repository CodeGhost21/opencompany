use crate::ports::CompanyStore;
use crate::ports::types::CompanyId;
use axum::http::StatusCode;

use super::setup_test_support_1::*;

/// The roster arrives over the wire after an operator edited it, so neither the
/// bounds nor the de-duplication can be assumed to have survived. Validation
/// runs again on the way in rather than trusting the client.
#[tokio::test]
async fn an_edited_roster_is_revalidated_on_the_way_in() {
    let home = home();
    let state = fresh_state(home.path());

    let mut company = designed_company(None);
    // Two rows that slug alike, and a blank one — all three are things a client
    // could send and `validate` would refuse.
    company["agents"] = serde_json::json!([
        { "name": "Ops", "role": "Ops Lead", "description": "a" },
        { "name": "Ops", "role": "ops  lead", "description": "b" },
        { "name": "", "role": "   ", "description": "c" },
        { "name": "Accounts", "role": "Accountant", "description": "d" }
    ]);

    let (status, body) = post_setup(state.clone(), serde_json::json!({ "company": company })).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let seeded = body["seeded_company"].as_str().expect("seeded");
    let manifest = seeded_manifest(home.path(), seeded).await;
    assert!(
        manifest.validate().is_empty(),
        "a registered company must be valid: {:?}",
        manifest.validate()
    );
    let ids: Vec<&str> = manifest.agents.iter().map(|a| a.id.as_str()).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "duplicate ids survived: {ids:?}");
}

/// Phase 2 builds this company's workflows from the same answers, so it must
/// never have to ask again. The company-scoped route already stores them; the
/// wizard is the *default* path, and a company created through it arriving
/// without them would be the one that gets re-interrogated.
#[tokio::test]
async fn the_answers_are_stored_on_the_company_the_wizard_built() {
    let home = home();
    let state = fresh_state(home.path());

    let (status, body) = post_setup(
        state.clone(),
        serde_json::json!({ "company": designed_company(Some("ada@example.com")) }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let seeded = body["seeded_company"].as_str().expect("seeded");
    let store = crate::store::FsCompanyStore::new(home.path().to_path_buf());
    let record = store
        .load(&CompanyId::new(seeded))
        .await
        .expect("load")
        .expect("record");
    let answers = record.setup.expect("the answers were stored");
    assert_eq!(answers.industry, "E-commerce — I sell homeware online");
    assert_eq!(answers.automate, "Meta ads, order dispatch");
}
