//! Store-level tests for [`super::fan_out`]: the Q7 table, the §6 data
//! carry-over matrix (M1–M14, C1–C3, one test per row) and the rollback /
//! failure-isolation rules.
//!
//! Every test picks its own company id (`company("m1")`, …) rather than
//! sharing one: [`super::slot_guard`] is keyed by company id in a process-wide
//! table, and two tests racing on the same id would serialize against each
//! other's lock for no reason — cheap to avoid, and it keeps a slow test from
//! ever explaining a slowdown in an unrelated one.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use super::*;
use crate::company::inference::store as inference_store;
use crate::error::OpenCompanyError;
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

const OLD: &str = "th-not-a-real-key";
const NEW: &str = "th-not-a-real-key-2";
const CUSTOM: &str = "th-not-a-real-key-custom";
const MODEL: &str = "acme/test-model";

const ACCOUNT_KEY_KEY: &str = crate::company::company_key::KEY_KEY;
const COMPOSIO_KEY_KEY: &str = crate::company::composio::TINYHUMANS_KEY_KEY;
const COMPOSIO_LEGACY_KEY: &str = crate::company::composio::LEGACY_TOKEN_KEY;
const LEGACY_INFERENCE_KEY_KEY: &str = crate::company::inference::KEY_KEY;

fn company(tag: &str) -> CompanyId {
    CompanyId::new(format!("fan-out-{tag}"))
}

fn llm_key_key() -> String {
    inference_store::provider_key_key(inference::MANAGED_SLUG)
}

#[derive(Default)]
struct MemSecrets {
    map: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl SecretStore for MemSecrets {
    async fn get(&self, _c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(key)
            .map(|v| SecretValue(v.clone())))
    }
    async fn set(&self, _c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        self.map.lock().unwrap().insert(key.to_string(), value.0);
        Ok(())
    }
}

/// A store whose writes to one key always fail — for the rollback and
/// failure-isolation rules.
struct FailsWriting {
    inner: MemSecrets,
    failing_key: String,
}

#[async_trait]
impl SecretStore for FailsWriting {
    async fn get(&self, c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        self.inner.get(c, key).await
    }
    async fn set(&self, c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        if key == self.failing_key {
            return Err(OpenCompanyError::Store("disk is on fire".into()));
        }
        self.inner.set(c, key, value).await
    }
}

/// A store whose every write sleeps briefly — for
/// [`concurrent_saves_leave_every_copy_equal_to_the_account_key`], which needs
/// two `fan_out` calls to actually interleave rather than one finishing
/// before the other starts.
struct SlowSecrets {
    inner: MemSecrets,
}

#[async_trait]
impl SecretStore for SlowSecrets {
    async fn get(&self, c: &CompanyId, key: &str) -> Result<Option<SecretValue>> {
        self.inner.get(c, key).await
    }
    async fn set(&self, c: &CompanyId, key: &str, value: SecretValue) -> Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        self.inner.set(c, key, value).await
    }
}

/// A canned prober answer, with a call counter so a test can assert the
/// probe never ran at all (e.g. on a `CustomKey` skip).
struct FakeProber {
    answer: std::result::Result<Vec<String>, probe::ProbeClass>,
    calls: AtomicUsize,
}

impl FakeProber {
    fn ok(ids: &[&str]) -> Self {
        Self {
            answer: Ok(ids.iter().map(|s| s.to_string()).collect()),
            calls: AtomicUsize::new(0),
        }
    }

    fn failing(class: probe::ProbeClass) -> Self {
        Self {
            answer: Err(class),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl InferenceProber for FakeProber {
    async fn probe(
        &self,
        _base_url: &str,
        _key: &str,
    ) -> std::result::Result<Vec<String>, probe::ProbeFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.answer {
            Ok(ids) => Ok(ids.clone()),
            Err(class) => Err(probe::ProbeFailure {
                class: *class,
                raw: "fake".to_string(),
            }),
        }
    }
}

async fn raw_set(secrets: &dyn SecretStore, company: &CompanyId, key: &str, value: &str) {
    secrets
        .set(company, key, SecretValue(value.to_string()))
        .await
        .unwrap();
}

async fn raw_get(secrets: &dyn SecretStore, company: &CompanyId, key: &str) -> String {
    secrets
        .get(company, key)
        .await
        .unwrap()
        .map(|SecretValue(v)| v)
        .unwrap_or_default()
}

async fn seed_row(secrets: &dyn SecretStore, company: &CompanyId, model: &str) {
    inference_store::put_provider(
        company,
        secrets,
        inference_store::ProviderDraft {
            slug: inference::MANAGED_SLUG.to_string(),
            label: "TinyHumans".to_string(),
            kind: inference::MANAGED_SLUG.to_string(),
            base_url: catalogue::cloud_provider(inference::MANAGED_SLUG)
                .unwrap()
                .endpoint
                .to_string(),
            models: tier_overrides(model),
            enabled: true,
        },
    )
    .await
    .unwrap();
}

fn outcome(report: &FanOutReport, slot: Slot) -> SlotOutcome {
    report
        .slots
        .iter()
        .find(|r| r.slot == slot)
        .unwrap_or_else(|| panic!("no {slot:?} slot in {report:?}"))
        .outcome
}

fn full(provider: &str, model: &str) -> inference_store::DefaultChoice {
    inference_store::DefaultChoice::Full(inference_store::ModelChoice {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}

// ---------------------------------------------------------------------------
// decide_copy — the Q7 table
// ---------------------------------------------------------------------------

#[test]
fn decide_copy_follows_the_q7_table() {
    // Not clearing.
    assert_eq!(
        decide_copy("B", "A", "B"),
        CopyDecision::Keep(SkipReason::AlreadyCurrent)
    );
    assert_eq!(decide_copy("", "A", "B"), CopyDecision::Write);
    assert_eq!(decide_copy("A", "A", "B"), CopyDecision::Write);
    assert_eq!(
        decide_copy("C", "A", "B"),
        CopyDecision::Keep(SkipReason::CustomKey)
    );
    // No prior account key at all: nothing can be "the old value".
    assert_eq!(decide_copy("", "", "B"), CopyDecision::Write);
    assert_eq!(
        decide_copy("C", "", "B"),
        CopyDecision::Keep(SkipReason::CustomKey)
    );

    // Clearing.
    assert_eq!(
        decide_copy("", "A", ""),
        CopyDecision::Skip(SkipReason::AlreadyEmpty)
    );
    assert_eq!(decide_copy("A", "A", ""), CopyDecision::Clear);
    assert_eq!(
        decide_copy("C", "A", ""),
        CopyDecision::Keep(SkipReason::CustomKey)
    );
}

// ---------------------------------------------------------------------------
// §6 matrix — M1..M14, C1..C3
// ---------------------------------------------------------------------------

#[tokio::test]
async fn matrix_m1() {
    let cid = company("m1");
    let secrets = MemSecrets::default();
    let prober = FakeProber::ok(&[MODEL, "acme/other-model"]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        inference_store::DefaultChoice::Unset
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::NeedsModel)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::NeedsModel)
    );
    assert_eq!(outcome(&report, Slot::Health), SlotOutcome::HealthOk);
    assert!(report.needs_model);
    assert!(report.sets_default);
    assert_eq!(
        report.models,
        vec!["acme/other-model".to_string(), MODEL.to_string()]
    );
}

#[tokio::test]
async fn matrix_m2() {
    let cid = company("m2");
    let secrets = MemSecrets::default();
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    let providers = inference_store::list_providers(&cid, &secrets)
        .await
        .unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].slug, "tinyhumans");
    assert_eq!(
        providers[0].model(),
        inference_store::ModelOnRow::One(MODEL.to_string())
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Provider), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Default), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Health), SlotOutcome::HealthOk);
    assert!(!report.needs_model);
}

#[tokio::test]
async fn matrix_m3() {
    let cid = company("m3");
    let secrets = MemSecrets::default();
    inference_store::put_provider(
        &cid,
        &secrets,
        inference_store::ProviderDraft {
            slug: "openrouter".to_string(),
            label: "OpenRouter".to_string(),
            kind: "openrouter".to_string(),
            base_url: "https://openrouter.example/v1".to_string(),
            models: BTreeMap::new(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "openrouter".to_string(),
            model: "x".to_string(),
        },
    )
    .await
    .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    let providers = inference_store::list_providers(&cid, &secrets)
        .await
        .unwrap();
    assert_eq!(
        providers
            .iter()
            .map(|p| p.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["openrouter", "tinyhumans"]
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("openrouter", "x")
    );
    assert_eq!(outcome(&report, Slot::Provider), SlotOutcome::Filled);
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Kept(SkipReason::DefaultAlreadySet)
    );
}

#[tokio::test]
async fn matrix_m4() {
    let cid = company("m4");
    let secrets = MemSecrets::default();
    inference_store::set_default_slug(&cid, &secrets, "openrouter")
        .await
        .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        inference_store::DefaultChoice::ProviderOnly("openrouter".to_string())
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::NeedsModel)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Kept(SkipReason::DefaultAlreadySet)
    );
    assert!(report.needs_model);
    assert!(
        !report.sets_default,
        "a bare-slug default already counts as set (Q1)"
    );
}

#[tokio::test]
async fn matrix_m5() {
    let cid = company("m5");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, &llm_key_key(), OLD).await;
    seed_row(&secrets, &cid, MODEL).await;
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "tinyhumans".to_string(),
            model: MODEL.to_string(),
        },
    )
    .await
    .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Rotated);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Rotated);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Kept(SkipReason::RowExists)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Kept(SkipReason::DefaultAlreadySet)
    );
}

#[tokio::test]
async fn matrix_m6() {
    let cid = company("m6");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, CUSTOM).await;
    raw_set(&secrets, &cid, &llm_key_key(), CUSTOM).await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, CUSTOM);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, CUSTOM);
    assert!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        inference_store::DefaultChoice::Unset
    );

    assert_eq!(
        outcome(&report, Slot::Composio),
        SlotOutcome::Kept(SkipReason::CustomKey)
    );
    assert_eq!(
        outcome(&report, Slot::Inference),
        SlotOutcome::Kept(SkipReason::CustomKey)
    );
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::CustomKey)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::CustomKey)
    );
    assert_eq!(
        outcome(&report, Slot::Health),
        SlotOutcome::Skipped(SkipReason::CustomKey)
    );
    assert_eq!(
        prober.calls.load(Ordering::SeqCst),
        0,
        "a custom key is never probed"
    );
}

#[tokio::test]
async fn matrix_m7() {
    let cid = company("m7");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, OLD).await;
    seed_row(&secrets, &cid, MODEL).await;
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "tinyhumans".to_string(),
            model: MODEL.to_string(),
        },
    )
    .await
    .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Kept(SkipReason::RowExists)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Kept(SkipReason::DefaultAlreadySet)
    );
}

#[tokio::test]
async fn matrix_m8() {
    let cid = company("m8");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    inference::save_runtime_config(
        &cid,
        &secrets,
        &inference::RuntimeInference {
            provider: "managed".to_string(),
            base_url: None,
            models: Default::default(),
        },
    )
    .await
    .unwrap();
    raw_set(&secrets, &cid, LEGACY_INFERENCE_KEY_KEY, OLD).await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        raw_get(&secrets, &cid, LEGACY_INFERENCE_KEY_KEY).await,
        "",
        "the set_managed_key convergence rule clears the legacy slot on the copy too"
    );
    assert!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .iter()
            .all(|p| p.origin != inference_store::ProviderOrigin::Indexed),
        "the legacy Managed row and an indexed tinyhumans row never both exist"
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        inference_store::DefaultChoice::Unset
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Rotated);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::LegacyManagedConfig)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::LegacyManagedConfig)
    );
    assert_eq!(
        outcome(&report, Slot::Health),
        SlotOutcome::Skipped(SkipReason::LegacyManagedConfig)
    );
    assert!(!report.needs_model);
    assert_eq!(
        prober.calls.load(Ordering::SeqCst),
        0,
        "entry zero being managed skips the probe entirely"
    );
}

#[tokio::test]
async fn matrix_m9() {
    let cid = company("m9");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    inference::save_runtime_config(
        &cid,
        &secrets,
        &inference::RuntimeInference {
            provider: "openrouter".to_string(),
            base_url: None,
            models: Default::default(),
        },
    )
    .await
    .unwrap();
    raw_set(
        &secrets,
        &cid,
        LEGACY_INFERENCE_KEY_KEY,
        "sk-not-a-real-key",
    )
    .await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        raw_get(&secrets, &cid, LEGACY_INFERENCE_KEY_KEY).await,
        "sk-not-a-real-key",
        "entry zero's own vendor credential must never be touched"
    );
    let providers = inference_store::list_providers(&cid, &secrets)
        .await
        .unwrap();
    assert!(
        providers
            .iter()
            .any(|p| p.slug == "tinyhumans" && p.origin == inference_store::ProviderOrigin::Indexed)
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );

    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Provider), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Default), SlotOutcome::Filled);
}

#[tokio::test]
async fn matrix_m10() {
    let cid = company("m10");
    let secrets = MemSecrets::default();
    let prober = FakeProber::failing(probe::ProbeClass::Auth);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(
        raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await,
        NEW,
        "the account key is kept"
    );
    assert_eq!(
        raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await,
        NEW,
        "the Composio copy is kept"
    );
    assert_eq!(
        raw_get(&secrets, &cid, &llm_key_key()).await,
        "",
        "the LLM copy is rolled back"
    );
    assert!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        inference_store::DefaultChoice::Unset
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::RolledBack);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::InferenceRejected)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::InferenceRejected)
    );
    assert_eq!(
        outcome(&report, Slot::Health),
        SlotOutcome::HealthFailed(probe::ProbeClass::Auth)
    );
    assert!(!report.needs_model);
}

#[tokio::test]
async fn matrix_m11() {
    let cid = company("m11");
    let secrets = MemSecrets::default();
    let prober = FakeProber::ok(&[MODEL]);
    fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );

    assert_eq!(
        outcome(&report, Slot::Composio),
        SlotOutcome::Kept(SkipReason::AlreadyCurrent)
    );
    assert_eq!(
        outcome(&report, Slot::Inference),
        SlotOutcome::Kept(SkipReason::AlreadyCurrent)
    );
    assert_eq!(outcome(&report, Slot::Provider), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Default), SlotOutcome::Filled);
    assert_eq!(outcome(&report, Slot::Health), SlotOutcome::HealthOk);
    assert!(!report.needs_model);
}

#[tokio::test]
async fn matrix_m12() {
    let cid = company("m12");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, &llm_key_key(), OLD).await;
    seed_row(&secrets, &cid, MODEL).await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL),
        "the default fills from the row's own model, none having been sent"
    );

    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Kept(SkipReason::RowExists)
    );
    assert_eq!(outcome(&report, Slot::Default), SlotOutcome::Filled);
}

#[tokio::test]
async fn matrix_m13() {
    let cid = company("m13");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, "composio/mode", "byok").await;
    raw_set(&secrets, &cid, "composio/byok/key", "ak-not-a-real-key").await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Filled);
    assert_eq!(
        raw_get(&secrets, &cid, "composio/mode").await,
        "byok",
        "the fan-out never reads or writes composio/mode"
    );
    assert_eq!(
        raw_get(&secrets, &cid, "composio/byok/key").await,
        "ak-not-a-real-key",
        "the fan-out never reads or writes composio/byok/key"
    );
}

#[tokio::test]
async fn matrix_m14() {
    let cid = company("m14");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    // Only the legacy address holds the old bearer — the fallback-read case.
    raw_set(&secrets, &cid, COMPOSIO_LEGACY_KEY, OLD).await;
    raw_set(&secrets, &cid, &llm_key_key(), OLD).await;
    seed_row(&secrets, &cid, MODEL).await;
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "tinyhumans".to_string(),
            model: MODEL.to_string(),
        },
    )
    .await
    .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, NEW);
    assert_eq!(
        raw_get(&secrets, &cid, COMPOSIO_LEGACY_KEY).await,
        "",
        "the fan-out's own copy retires the legacy mirror rather than carrying it forward"
    );
    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Rotated);
}

#[tokio::test]
async fn matrix_c1() {
    let cid = company("c1");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, NEW).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, NEW).await;
    raw_set(&secrets, &cid, &llm_key_key(), NEW).await;
    seed_row(&secrets, &cid, MODEL).await;
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "tinyhumans".to_string(),
            model: MODEL.to_string(),
        },
    )
    .await
    .unwrap();
    inference_store::record_health(&cid, &secrets, "tinyhumans", "ok", "2026-01-01T00:00:00Z")
        .await
        .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: "",
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, "");
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1,
        "the row stays"
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL),
        "the default stays"
    );
    assert!(
        inference_store::load_health(&cid, &secrets)
            .await
            .unwrap()
            .get("tinyhumans")
            .is_none(),
        "health is forgotten"
    );

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Cleared);
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Cleared);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::KeyCleared)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::KeyCleared)
    );
    assert_eq!(outcome(&report, Slot::Health), SlotOutcome::Cleared);
}

#[tokio::test]
async fn matrix_c2() {
    let cid = company("c2");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, NEW).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, CUSTOM).await;
    raw_set(&secrets, &cid, &llm_key_key(), CUSTOM).await;
    seed_row(&secrets, &cid, MODEL).await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: "",
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, CUSTOM);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, CUSTOM);

    assert_eq!(
        outcome(&report, Slot::Composio),
        SlotOutcome::Kept(SkipReason::CustomKey)
    );
    assert_eq!(
        outcome(&report, Slot::Inference),
        SlotOutcome::Kept(SkipReason::CustomKey)
    );
}

#[tokio::test]
async fn matrix_c3() {
    let cid = company("c3");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, NEW).await;

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: "",
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, "");
    assert_eq!(
        outcome(&report, Slot::Composio),
        SlotOutcome::Skipped(SkipReason::AlreadyEmpty)
    );
    assert_eq!(
        outcome(&report, Slot::Inference),
        SlotOutcome::Skipped(SkipReason::AlreadyEmpty)
    );
}

// ---------------------------------------------------------------------------
// Rollback and failure isolation
// ---------------------------------------------------------------------------

/// Adapted from the plan's "M8 with a prober that answers `auth`": M8 itself
/// gates health on `legacy_managed` (entry zero declared managed), which
/// makes the probe unreachable there no matter what it would answer — see
/// `matrix_m8`'s own `calls == 0` assertion. This is the scenario that
/// actually reaches the probe while still matching M8's *other* defining
/// trait (`legacy_slot_is_managed` reading true through the "no entry zero at
/// all" branch, with a real value sitting in the legacy `inference/key`
/// slot): no entry zero, so nothing is `legacy_managed`, but the flat
/// `inference/key` slot is still what backs the LLM copy — and an `auth`
/// rejection has to restore both of the slots this request touched.
#[tokio::test]
async fn an_auth_probe_restores_the_llm_slots_exactly() {
    let cid = company("auth-restore");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, LEGACY_INFERENCE_KEY_KEY, OLD).await;

    let prober = FakeProber::failing(probe::ProbeClass::Auth);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(
        raw_get(&secrets, &cid, &llm_key_key()).await,
        "",
        "restored to exactly what it held before this request"
    );
    assert_eq!(
        raw_get(&secrets, &cid, LEGACY_INFERENCE_KEY_KEY).await,
        OLD,
        "the legacy slot this request cleared is put back"
    );
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::RolledBack);
    assert_eq!(
        raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await,
        NEW,
        "the account key itself is kept"
    );
}

#[tokio::test]
async fn a_non_auth_probe_failure_keeps_everything() {
    let cid = company("non-auth");
    let secrets = MemSecrets::default();
    let prober = FakeProber::failing(probe::ProbeClass::Endpoint);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );
    assert_eq!(
        outcome(&report, Slot::Health),
        SlotOutcome::HealthFailed(probe::ProbeClass::Endpoint)
    );
    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Filled);
}

#[tokio::test]
async fn an_invalid_model_writes_nothing() {
    let cid = company("invalid-model");
    let secrets = MemSecrets::default();
    let prober = FakeProber::ok(&[MODEL]);

    let err = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some("chat-v1"),
        },
        &prober,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("workload name"), "{err}");
    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, "");
    assert_eq!(prober.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_model_with_a_clear_is_refused() {
    let cid = company("model-clear");
    let secrets = MemSecrets::default();
    let prober = FakeProber::ok(&[MODEL]);

    let err = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: "",
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("cannot be chosen"), "{err}");
    assert_eq!(raw_get(&secrets, &cid, ACCOUNT_KEY_KEY).await, "");
}

#[tokio::test]
async fn failing_account_key_write_writes_nothing_else() {
    let cid = company("acct-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: ACCOUNT_KEY_KEY.to_string(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let err = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, OpenCompanyError::Store(_)));
    assert_eq!(raw_get(&secrets, &cid, COMPOSIO_KEY_KEY).await, "");
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, "");
}

#[tokio::test]
async fn failing_composio_write_still_sets_up_llm() {
    let cid = company("composio-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: COMPOSIO_KEY_KEY.to_string(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(outcome(&report, Slot::Composio), SlotOutcome::Failed);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        inference_store::load_default(&cid, &secrets).await.unwrap(),
        full("tinyhumans", MODEL)
    );
}

#[tokio::test]
async fn failing_inference_write_skips_row_default_and_probe() {
    let cid = company("inference-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: llm_key_key(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(outcome(&report, Slot::Inference), SlotOutcome::Failed);
    assert_eq!(
        outcome(&report, Slot::Provider),
        SlotOutcome::Skipped(SkipReason::InferenceNotWritten)
    );
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::InferenceNotWritten)
    );
    assert_eq!(
        outcome(&report, Slot::Health),
        SlotOutcome::Skipped(SkipReason::InferenceNotWritten)
    );
    assert_eq!(prober.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failing_row_write_keeps_the_key_copy() {
    let cid = company("row-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: inference_store::PROVIDER_INDEX_KEY.to_string(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(outcome(&report, Slot::Provider), SlotOutcome::Failed);
    assert_eq!(raw_get(&secrets, &cid, &llm_key_key()).await, NEW);
    assert_eq!(
        outcome(&report, Slot::Default),
        SlotOutcome::Skipped(SkipReason::InferenceNotWritten)
    );
}

#[tokio::test]
async fn failing_default_write_keeps_the_row() {
    let cid = company("default-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: inference_store::DEFAULT_PROVIDER_KEY.to_string(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(outcome(&report, Slot::Default), SlotOutcome::Failed);
    assert_eq!(
        inference_store::list_providers(&cid, &secrets)
            .await
            .unwrap()
            .len(),
        1,
        "the row stays despite the default failing to write"
    );
}

#[tokio::test]
async fn failing_health_record_does_not_change_outcomes() {
    let cid = company("health-fail");
    let secrets = FailsWriting {
        inner: MemSecrets::default(),
        failing_key: inference_store::HEALTH_KEY.to_string(),
    };
    let prober = FakeProber::ok(&[MODEL]);

    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: Some(MODEL),
        },
        &prober,
    )
    .await
    .unwrap();

    assert_eq!(outcome(&report, Slot::Health), SlotOutcome::HealthOk);
}

/// Red-proof in the PR: without `slot_guard` held for the whole call, this
/// fails — one save's read-then-write can interleave with the other's and
/// leave the Composio and LLM copies pointing at different values than
/// `tinyhumans/key` itself.
#[tokio::test]
async fn concurrent_saves_leave_every_copy_equal_to_the_account_key() {
    const OTHER: &str = "th-not-a-real-key-3";
    for i in 0..20 {
        let cid = company(&format!("concurrent-{i}"));
        let secrets = std::sync::Arc::new(SlowSecrets {
            inner: MemSecrets::default(),
        });
        raw_set(secrets.as_ref(), &cid, ACCOUNT_KEY_KEY, OLD).await;
        raw_set(secrets.as_ref(), &cid, COMPOSIO_KEY_KEY, OLD).await;
        raw_set(secrets.as_ref(), &cid, &llm_key_key(), OLD).await;

        let prober_a = FakeProber::ok(&[MODEL]);
        let prober_b = FakeProber::ok(&[MODEL]);
        let (s1, s2) = (secrets.clone(), secrets.clone());
        let (c1, c2) = (cid.clone(), cid.clone());
        let first = fan_out(
            &c1,
            s1.as_ref(),
            FanOutRequest {
                key: NEW,
                model: None,
            },
            &prober_a,
        );
        let second = fan_out(
            &c2,
            s2.as_ref(),
            FanOutRequest {
                key: OTHER,
                model: None,
            },
            &prober_b,
        );
        let (r1, r2) = tokio::join!(first, second);
        r1.unwrap();
        r2.unwrap();

        let account = raw_get(secrets.as_ref(), &cid, ACCOUNT_KEY_KEY).await;
        let composio = raw_get(secrets.as_ref(), &cid, COMPOSIO_KEY_KEY).await;
        let llm = raw_get(secrets.as_ref(), &cid, &llm_key_key()).await;
        assert_eq!(
            composio, account,
            "iteration {i}: composio must equal whichever save landed last"
        );
        assert_eq!(
            llm, account,
            "iteration {i}: the LLM copy must equal whichever save landed last"
        );
    }
}

#[tokio::test]
async fn no_report_or_note_contains_a_key() {
    let cid = company("no-leak");
    let secrets = MemSecrets::default();
    raw_set(&secrets, &cid, ACCOUNT_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, COMPOSIO_KEY_KEY, OLD).await;
    raw_set(&secrets, &cid, &llm_key_key(), OLD).await;
    seed_row(&secrets, &cid, MODEL).await;
    inference_store::set_default_choice(
        &cid,
        &secrets,
        &inference_store::ModelChoice {
            provider: "tinyhumans".to_string(),
            model: MODEL.to_string(),
        },
    )
    .await
    .unwrap();

    let prober = FakeProber::ok(&[MODEL]);
    let report = fan_out(
        &cid,
        &secrets,
        FanOutRequest {
            key: NEW,
            model: None,
        },
        &prober,
    )
    .await
    .unwrap();
    let note = fan_out_note(false, &report, None);

    let rendered = format!("{report:?}");
    assert!(!rendered.contains(OLD), "{rendered}");
    assert!(!rendered.contains(NEW), "{rendered}");
    assert!(!note.contains(OLD), "{note}");
    assert!(!note.contains(NEW), "{note}");
}
