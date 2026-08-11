//! The company's own TinyHumans credential — the one key its admin sets on the
//! tenant, and the identity every **brokered** surface presents (issue #586).
//!
//! ## What this is, and what it is not
//!
//! A company belongs to one owner. The admin sets one TinyHumans key here, and
//! everything the platform backend brokers on the company's behalf rides it:
//! Composio today, whatever else the backend fronts tomorrow. Membership in the
//! company is what grants access to it.
//!
//! It is deliberately **not** [`inference::KEY_KEY`](crate::company::inference)
//! . That slot holds whatever credential the company's *declared provider*
//! wants — an OpenRouter `sk-or-…`, a raw BYOK key, an `openai_compatible`
//! token. It is provider-scoped, not an identity. Handing it to the TinyHumans
//! backend would present one vendor's credential to another. The two coincide
//! only when the declared provider is `managed`, and even then they are the same
//! *value* for different reasons. One slot per meaning.
//!
//! ## Precedence
//!
//! [`resolve`] is the single seam. Most specific first:
//!
//! 1. **The company's own key**, stored here — set, rotated and cleared by an
//!    admin through the console, write-only, never read back.
//! 2. **This instance's platform identity** —
//!    [`TinyhumansTokenSource`](crate::company::credentials::TinyhumansTokenSource),
//!    the projected, audience-bound token a hosted pod carries. Unchanged
//!    behaviour for a tenant whose admin has set nothing.
//! 3. **Nothing** ⇒ [`Credential::None`], and the caller fails closed.
//!
//! Every brokered surface resolving through *this one function* is what makes
//! the rotation guarantee true by construction: there is no second resolution to
//! drift, so a key set in the console cannot reach one surface and miss another.
//! A surface may prepend its own escape-hatch tier ahead of this (Composio keeps
//! its BYO `composio/token` — see
//! [`harness::composio`](crate::harness::composio)), but nothing may resolve a
//! *company* identity any other way.
//!
//! ## Write-only
//!
//! The value is never returned by any read route. [`key_configured`] answers the
//! only question the console needs, and [`Credential`]'s `Debug` redacts what it
//! holds.
//!
//! ## Freshness
//!
//! Read live from the [`SecretStore`] on every resolution, so a console set /
//! rotate / clear takes effect on the next cycle with no restart. The resolved
//! credential contributes its **value** to the roster fingerprint
//! ([`Credential::hash_identity`]), so a rotation rebuilds the tool roster
//! rather than leaving agents on the previous identity.

use crate::Result;
use crate::company::credentials::{Credential, TinyhumansTokenSource};
use crate::ports::SecretStore;
use crate::ports::types::{CompanyId, SecretValue};

/// The canonical per-company TinyHumans credential key. Write-only via the
/// console; the value is the raw key string.
///
/// Namespaced `tinyhumans/` rather than under any one consumer, because it is
/// not any one consumer's key — see the module docs on why this is not
/// `inference/key`.
pub const KEY_KEY: &str = "tinyhumans/key";

/// Store (or rotate / clear) the company's TinyHumans credential. A non-empty
/// value rotates it; an empty string clears it. Write-only — never read back
/// over the API.
pub async fn store_key(company: &CompanyId, secrets: &dyn SecretStore, key: &str) -> Result<()> {
    secrets
        .set(company, KEY_KEY, SecretValue(key.trim().to_string()))
        .await
}

/// Whether a non-empty company key is stored — never the key itself. The only
/// thing the read planes ever surface about it.
pub async fn key_configured(company: &CompanyId, secrets: &dyn SecretStore) -> Result<bool> {
    Ok(matches!(
        load(company, secrets).await,
        Credential::Company(_)
    ))
}

/// The company's own key as a [`Credential`], or [`Credential::None`] when unset,
/// blank, or unreadable.
///
/// A secret-store read error degrades to `None` rather than bubbling: a
/// transient store hiccup must fall through to the instance identity for that
/// cycle, not brick a roster build. Mirrors the same choice in
/// [`TenantComposio::resolve`](crate::harness::composio::TenantComposio::resolve).
pub async fn load(company: &CompanyId, secrets: &dyn SecretStore) -> Credential {
    match secrets.get(company, KEY_KEY).await {
        Ok(Some(SecretValue(key))) => Credential::from_company_key(key),
        _ => Credential::None,
    }
}

/// The credential a brokered call presents for this company — **the** seam.
///
/// The company's own key wins; failing that this instance's platform identity;
/// failing that [`Credential::None`], which every caller treats as fail-closed.
/// See the module docs for why there is exactly one of these.
pub async fn resolve(
    company: &CompanyId,
    secrets: &dyn SecretStore,
    token_source: Option<std::sync::Arc<TinyhumansTokenSource>>,
) -> Credential {
    match (load(company, secrets).await, token_source) {
        (company_key @ Credential::Company(_), _) => company_key,
        (_, Some(source)) => Credential::from_source(source),
        (_, None) => Credential::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company::credentials::CredentialSource;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

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

    fn source() -> Arc<TinyhumansTokenSource> {
        Arc::new(TinyhumansTokenSource::static_key("instance-identity"))
    }

    #[tokio::test]
    async fn the_company_key_outranks_the_instance_identity() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();

        // Nothing set: the tenant borrows the instance's identity, exactly as
        // it did before this seam existed.
        let credential = resolve(&company, &secrets, Some(source())).await;
        assert_eq!(credential.source(), CredentialSource::Static);
        assert_eq!(
            credential.current().await.unwrap().as_deref(),
            Some("instance-identity")
        );

        // Admin sets the company's own key: every brokered call now presents it.
        store_key(&company, &secrets, "th_company_key")
            .await
            .unwrap();
        let credential = resolve(&company, &secrets, Some(source())).await;
        assert_eq!(credential.source(), CredentialSource::Company);
        assert_eq!(
            credential.current().await.unwrap().as_deref(),
            Some("th_company_key")
        );
    }

    #[tokio::test]
    async fn no_key_and_no_instance_identity_fails_closed() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        let credential = resolve(&company, &secrets, None).await;
        assert_eq!(credential.source(), CredentialSource::None);
        assert!(!credential.configured());
    }

    #[tokio::test]
    async fn clearing_the_key_falls_back_rather_than_stranding_the_company() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        store_key(&company, &secrets, "th_company_key")
            .await
            .unwrap();
        assert!(key_configured(&company, &secrets).await.unwrap());

        // An empty value is a clear. The store has no delete, so this is how
        // "unset" reads back.
        store_key(&company, &secrets, "").await.unwrap();
        assert!(!key_configured(&company, &secrets).await.unwrap());
        assert_eq!(
            resolve(&company, &secrets, Some(source())).await.source(),
            CredentialSource::Static
        );
    }

    #[tokio::test]
    async fn a_blank_key_is_not_configuration() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        store_key(&company, &secrets, "   ").await.unwrap();
        assert!(!key_configured(&company, &secrets).await.unwrap());
        assert_eq!(
            resolve(&company, &secrets, None).await.source(),
            CredentialSource::None
        );
    }

    /// A store read that fails must degrade to the instance identity for that
    /// cycle, never bubble into the roster build.
    #[tokio::test]
    async fn an_unreadable_store_degrades_to_the_instance_identity() {
        let company = CompanyId::new("acme");
        assert!(!key_configured(&company, &BrokenSecrets).await.unwrap());
        let credential = resolve(&company, &BrokenSecrets, Some(source())).await;
        assert!(credential.configured());
        assert_eq!(
            credential.current().await.unwrap().as_deref(),
            Some("instance-identity")
        );
    }

    /// The rotation contract (issue #586 acceptance): a rotated company key is a
    /// new identity, so anything fingerprinting the credential rebuilds.
    #[tokio::test]
    async fn rotating_the_key_moves_the_fingerprint() {
        use std::hash::Hasher;

        let identity_of = |credential: &Credential| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            credential.hash_identity(&mut hasher);
            hasher.finish()
        };

        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        store_key(&company, &secrets, "key-a").await.unwrap();
        let before = identity_of(&resolve(&company, &secrets, Some(source())).await);

        store_key(&company, &secrets, "key-b").await.unwrap();
        let after = identity_of(&resolve(&company, &secrets, Some(source())).await);
        assert_ne!(
            before, after,
            "a rotated company key must rebuild the roster"
        );

        // And a company key is never confused with an identically-valued
        // per-provider token pasted into some other slot.
        assert_ne!(
            identity_of(&Credential::from_company_key("key-b")),
            identity_of(&Credential::from_value("key-b"))
        );
    }

    #[tokio::test]
    async fn the_key_is_write_only_in_every_rendering() {
        let company = CompanyId::new("acme");
        let secrets = MemSecrets::default();
        store_key(&company, &secrets, "th_super_secret")
            .await
            .unwrap();
        let credential = resolve(&company, &secrets, None).await;
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("th_super_secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }
}
