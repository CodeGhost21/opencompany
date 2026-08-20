//! Tests for the company's Smithery directory credential (issue #1287).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use super::*;
use crate::app::config::MapEnv;

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

/// A store whose reads always fail — the transient-hiccup case.
struct BrokenSecrets;

#[async_trait]
impl SecretStore for BrokenSecrets {
    async fn get(&self, _c: &CompanyId, _key: &str) -> Result<Option<SecretValue>> {
        Err(crate::error::OpenCompanyError::Store("boom".into()))
    }
    async fn set(&self, _c: &CompanyId, _key: &str, _value: SecretValue) -> Result<()> {
        Err(crate::error::OpenCompanyError::Store("boom".into()))
    }
}

fn empty_env() -> MapEnv {
    MapEnv::default()
}

fn host_env(key: &str) -> MapEnv {
    MapEnv::new([(API_KEY_ENV, key)])
}

#[tokio::test]
async fn the_company_key_outranks_the_host_environment() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "company-key").await.unwrap();

    let resolved = resolve(&company, &secrets, &host_env("host-key"))
        .await
        .unwrap();

    assert_eq!(resolved.source(), DirectoryKeySource::Company);
    assert_eq!(resolved.value(), Some("company-key"));
}

#[tokio::test]
async fn the_host_environment_answers_when_the_company_set_nothing() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();

    let resolved = resolve(&company, &secrets, &host_env("host-key"))
        .await
        .unwrap();

    assert_eq!(resolved.source(), DirectoryKeySource::Environment);
    assert_eq!(resolved.value(), Some("host-key"));
}

#[tokio::test]
async fn nothing_anywhere_resolves_to_none_with_no_value() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();

    let resolved = resolve(&company, &secrets, &empty_env()).await.unwrap();

    assert_eq!(resolved.source(), DirectoryKeySource::None);
    assert_eq!(resolved.value(), None);
    assert!(!resolved.source().configured());
}

/// The host tier is a *different answer*, not a shade of the same one: one
/// Smithery account shared by every company on the instance. A console that
/// cannot tell the two apart repeats the #886 failure, so the tier must survive
/// as its own value rather than collapsing into a boolean.
#[tokio::test]
async fn the_two_working_tiers_stay_distinguishable() {
    let company = CompanyId::new("acme");
    let mine = MemSecrets::default();
    store_key(&company, &mine, "company-key").await.unwrap();
    let theirs = MemSecrets::default();

    let own = resolve(&company, &mine, &empty_env()).await.unwrap();
    let shared = resolve(&company, &theirs, &host_env("host-key"))
        .await
        .unwrap();

    assert!(own.source().configured());
    assert!(shared.source().configured());
    assert_ne!(own.source(), shared.source());
    assert_eq!(own.source().as_str(), "company");
    assert_eq!(shared.source().as_str(), "environment");
}

/// A cleared key must fall through to the host tier rather than pinning the
/// company to "no directory" — clearing withdraws *this company's* key, it does
/// not opt the company out of a host that has one.
#[tokio::test]
async fn clearing_falls_through_to_the_host_environment() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "company-key").await.unwrap();
    store_key(&company, &secrets, "").await.unwrap();

    assert!(!key_configured(&company, &secrets).await.unwrap());
    let resolved = resolve(&company, &secrets, &host_env("host-key"))
        .await
        .unwrap();
    assert_eq!(resolved.source(), DirectoryKeySource::Environment);
}

/// Whitespace is not a credential. A field submitted with spaces in it clears,
/// and never resolves as a key that would authenticate nothing and report
/// `company`.
#[tokio::test]
async fn a_blank_stored_value_is_not_a_credential() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "   ").await.unwrap();

    assert!(!key_configured(&company, &secrets).await.unwrap());
    assert_eq!(
        resolve(&company, &secrets, &empty_env())
            .await
            .unwrap()
            .source(),
        DirectoryKeySource::None
    );
}

/// A stored key is trimmed on the way in, so a pasted value with a trailing
/// newline still authenticates.
#[tokio::test]
async fn a_pasted_key_is_trimmed() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "  key-with-space\n")
        .await
        .unwrap();

    assert_eq!(
        resolve(&company, &secrets, &empty_env())
            .await
            .unwrap()
            .value(),
        Some("key-with-space")
    );
}

/// An unreadable store is not "no key". Degrading to the host tier here would
/// send a *different Smithery account's* credential on behalf of a company that
/// has its own.
#[tokio::test]
async fn an_unreadable_store_propagates_rather_than_downgrading() {
    let company = CompanyId::new("acme");

    assert!(
        resolve(&company, &BrokenSecrets, &host_env("host-key"))
            .await
            .is_err()
    );
    assert!(key_configured(&company, &BrokenSecrets).await.is_err());
}

/// The env var name is upstream's own (`registries::smithery::smithery_api_key`
/// reads it directly). If this drifts, our reported tier and the key upstream
/// actually finds stop agreeing.
#[test]
fn the_env_var_matches_the_name_upstream_reads() {
    assert_eq!(API_KEY_ENV, "SMITHERY_API_KEY");
}
