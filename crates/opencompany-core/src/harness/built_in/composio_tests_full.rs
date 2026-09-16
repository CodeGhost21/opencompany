use super::*;

// Used by the resolver tests below; the module body itself resolves its
// credential through `company::composio::resolve_credential`.
use crate::company::company_key;
use crate::ports::types::SecretValue;

// The three helper tests below follow their subjects behind the `composio`
// feature: `toolkit_allowed` / `slug_toolkit` do not exist in an
// `openhuman`-without-`composio` build.
#[cfg(feature = "composio")]
#[test]
fn toolkit_allowed_empty_defers_to_backend() {
    // Empty allowlist = open mode: every toolkit admitted.
    assert!(toolkit_allowed(&[], "gmail"));
    assert!(toolkit_allowed(&[], "anything"));
}

#[cfg(feature = "composio")]
#[test]
fn toolkit_allowed_non_empty_narrows_case_insensitively() {
    let allow = vec!["gmail".to_string(), "github".to_string()];
    assert!(toolkit_allowed(&allow, "gmail"));
    assert!(toolkit_allowed(&allow, "GMAIL"));
    assert!(toolkit_allowed(&allow, "GitHub"));
    assert!(!toolkit_allowed(&allow, "slack"));
}

#[cfg(feature = "composio")]
#[test]
fn slug_toolkit_extracts_lowercased_prefix() {
    assert_eq!(slug_toolkit("GMAIL_SEND_EMAIL"), "gmail");
    assert_eq!(slug_toolkit("SLACK_POST_MESSAGE"), "slack");
    assert_eq!(slug_toolkit("GITHUB_CREATE_ISSUE"), "github");
    assert_eq!(slug_toolkit(""), "");
}

#[test]
fn debug_redacts_the_token() {
    let config = TenantComposio::new(
        "https://api.tinyhumans.ai",
        Credential::from_value("super-secret-tenant-token"),
        vec!["gmail".to_string()],
    );
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("super-secret-tenant-token"),
        "token leaked: {shown}"
    );
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(shown.contains("api.tinyhumans.ai"), "{shown}");
    assert!(
        shown.contains("gmail"),
        "toolkits should be visible: {shown}"
    );
}

#[test]
fn debug_marks_unset_token() {
    let config = TenantComposio::new("https://api.tinyhumans.ai", Credential::None, Vec::new());
    let shown = format!("{config:?}");
    assert!(shown.contains("<unset>"), "{shown}");
}

/// The resolver's precedence and its fail-closed floor: the company's own
/// stored token always wins; with none stored the instance's platform token
/// source is used; with neither there is no config at all (no tools) — never a
/// borrowed identity. A raw `TINYHUMANS_API_KEY` in the environment is not a
/// source: the platform identity is passed in explicitly by the caller.
#[tokio::test]
async fn resolve_prefers_the_stored_token_then_the_token_source_then_fails_closed() {
    use crate::ports::SecretStore;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-res-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    let source = || Arc::new(TinyhumansTokenSource::static_key("platform-identity"));

    // Nothing stored and no platform identity → fail closed. That an ambient
    // `TINYHUMANS_API_KEY` cannot be consulted is guaranteed by the signature
    // — `resolve` takes the source explicitly and has no `EnvSource` — so it
    // needs no proof by process-env mutation. Setting one here used to leak
    // into every other test in this binary (`std::env` is process-wide and
    // nothing restored it), which made an ops-route assertion on
    // `credentialSource == "none"` flake depending on test order.
    assert!(
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, None)
            .await
            .is_none(),
        "no credential at all must fail closed"
    );

    // Nothing stored, but this instance has an identity → it is used.
    let attested =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("the platform identity resolves");
    assert_eq!(
        token_of(&attested).await.as_deref(),
        Some("platform-identity")
    );

    // An explicitly-empty stored token is not a token: still the source.
    secrets
        .set(&company, TINYHUMANS_KEY_KEY, SecretValue("   ".to_string()))
        .await
        .unwrap();
    let attested =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("the platform identity resolves");
    assert_eq!(
        token_of(&attested).await.as_deref(),
        Some("platform-identity")
    );
    // …and with no source either, an empty stored token fails closed.
    assert!(
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, None)
            .await
            .is_none()
    );

    // The company's OWN token wins over the platform identity.
    secrets
        .set(
            &company,
            TINYHUMANS_KEY_KEY,
            SecretValue("tenant-token-xyz".to_string()),
        )
        .await
        .unwrap();
    let resolved = TenantComposio::resolve(
        &company,
        &secrets,
        vec!["gmail".into()],
        None,
        Some(source()),
    )
    .await
    .expect("a stored token resolves");
    assert_eq!(
        token_of(&resolved).await.as_deref(),
        Some("tenant-token-xyz"),
        "a company that brought its own token keeps it"
    );
    assert_eq!(resolved.backend_url, "https://api.tinyhumans.ai");
    assert_eq!(resolved.toolkits, vec!["gmail".to_string()]);

    // The tenant API base is threaded into the backend URL so a staging
    // tenant's Composio follows staging.
    let staged = TenantComposio::resolve(
        &company,
        &secrets,
        Vec::new(),
        Some("https://staging-api.tinyhumans.ai".into()),
        None,
    )
    .await
    .expect("a stored token resolves");
    assert_eq!(staged.backend_url, "https://staging-api.tinyhumans.ai");
}

/// Issue #586: the company's own TinyHumans key sits between its pasted
/// Composio token and the instance's identity, and it is enough on its own —
/// a company with a key set connects providers with no Composio token and no
/// per-tenant provider app.
#[tokio::test]
async fn the_company_key_credentials_composio_between_a_byo_token_and_the_instance() {
    use crate::company::credentials::CredentialSource;
    use crate::ports::SecretStore;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-companykey-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    let source = || Arc::new(TinyhumansTokenSource::static_key("platform-identity"));

    company_key::store_key(&company, &secrets, "th_company_key")
        .await
        .unwrap();

    // With no instance identity at all, the company key alone credentials
    // Composio — the case this issue exists to fix, since a pod with no
    // projected token previously had to fall back to a pasted token.
    let resolved = TenantComposio::resolve(&company, &secrets, Vec::new(), None, None)
        .await
        .expect("the company key resolves without any instance identity");
    assert_eq!(token_of(&resolved).await.as_deref(), Some("th_company_key"));
    assert_eq!(resolved.credential().source(), CredentialSource::Company);

    // And it outranks the instance's identity: the company acts as itself,
    // not as the pod it happens to run in.
    let resolved =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("resolves");
    assert_eq!(token_of(&resolved).await.as_deref(), Some("th_company_key"));

    // A pasted Composio token still outranks it — the BYO hatch survives.
    secrets
        .set(
            &company,
            TINYHUMANS_KEY_KEY,
            SecretValue("byo-composio".to_string()),
        )
        .await
        .unwrap();
    let resolved =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("resolves");
    assert_eq!(token_of(&resolved).await.as_deref(), Some("byo-composio"));
    assert_eq!(resolved.credential().source(), CredentialSource::Static);

    // Clearing the BYO token falls back to the company key, not to the
    // instance — clearing one tier must not silently re-borrow another.
    secrets
        .set(&company, TINYHUMANS_KEY_KEY, SecretValue(String::new()))
        .await
        .unwrap();
    // Also blank the legacy address explicitly (P2-3): a whitespace-only
    // `composio/token` must not itself be read as "the legacy address
    // holds a value" and shadow the company key — it must fall through
    // exactly as an unwritten legacy address does.
    secrets
        .set(
            &company,
            crate::company::composio::LEGACY_TOKEN_KEY,
            SecretValue("   ".to_string()),
        )
        .await
        .unwrap();
    let resolved =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("resolves");
    assert_eq!(token_of(&resolved).await.as_deref(), Some("th_company_key"));
    assert_eq!(resolved.credential().source(), CredentialSource::Company);

    // Clearing the company key too falls all the way back to the instance.
    company_key::store_key(&company, &secrets, "")
        .await
        .unwrap();
    let resolved =
        TenantComposio::resolve(&company, &secrets, Vec::new(), None, Some(source()))
            .await
            .expect("resolves");
    assert_eq!(
        token_of(&resolved).await.as_deref(),
        Some("platform-identity")
    );
}

/// Storage addresses and the legacy fallback (#2306), exercised against a
/// real backend so the nested address (`composio/tinyhumans/key`) is proven
/// on disk, not just in `MemSecrets`.
#[tokio::test]
async fn a_legacy_only_token_still_wires_managed_composio() {
    use crate::company::credentials::CredentialSource;
    use crate::ports::SecretStore;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-legacy-token-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    secrets
        .set(
            &company,
            crate::company::composio::LEGACY_TOKEN_KEY,
            SecretValue("th-not-a-real-key".into()),
        )
        .await
        .unwrap();

    let resolved = TenantComposio::resolve(&company, &secrets, Vec::new(), None, None)
        .await
        .expect("a legacy-only token still resolves");
    assert_eq!(
        token_of(&resolved).await.as_deref(),
        Some("th-not-a-real-key")
    );
    assert_eq!(resolved.credential().source(), CredentialSource::Static);
}

#[tokio::test]
async fn a_legacy_only_byok_key_still_resolves_the_byok_route() {
    use crate::company::composio::{BYOK_MODE, MODE_KEY};
    use crate::ports::SecretStore;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-legacy-byok-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    secrets
        .set(&company, MODE_KEY, SecretValue(BYOK_MODE.into()))
        .await
        .unwrap();
    secrets
        .set(
            &company,
            crate::company::composio::LEGACY_API_KEY_KEY,
            SecretValue("ak-not-a-real-key".into()),
        )
        .await
        .unwrap();

    let resolved = TenantComposio::resolve(&company, &secrets, Vec::new(), None, None)
        .await
        .expect("a legacy-only BYOK key still resolves");
    assert_eq!(resolved.mode(), ComposioMode::Byok);
    assert_eq!(
        token_of(&resolved).await.as_deref(),
        Some("ak-not-a-real-key")
    );

    crate::company::composio::store_api_key(&company, &secrets, "ak-not-a-real-key-2")
        .await
        .unwrap();
    assert_eq!(
        secrets
            .get(&company, BYOK_KEY_KEY)
            .await
            .unwrap()
            .map(|SecretValue(v)| v)
            .as_deref(),
        Some("ak-not-a-real-key-2")
    );
    assert_eq!(
        secrets
            .get(&company, crate::company::composio::LEGACY_API_KEY_KEY)
            .await
            .unwrap()
            .map(|SecretValue(v)| v)
            .as_deref(),
        Some("ak-not-a-real-key-2")
    );
}

/// The roster path's half of the store-error contract: it must **fail
/// closed** — no tools this cycle — rather than fall through to the
/// instance identity or bubble and brick the build.
///
/// Fewer tools for a cycle is recoverable and visible. Silently presenting
/// a different identity is neither: it would attribute whatever the agents
/// did in that window to the wrong account.
#[tokio::test]
async fn an_unreadable_store_withholds_tools_rather_than_borrowing_an_identity() {
    use crate::ports::types::CompanyId;

    struct BrokenSecrets;

    #[async_trait::async_trait]
    impl SecretStore for BrokenSecrets {
        async fn get(
            &self,
            _c: &CompanyId,
            _key: &str,
        ) -> crate::Result<Option<crate::ports::types::SecretValue>> {
            Err(crate::error::OpenCompanyError::Store("boom".into()))
        }
        async fn set(
            &self,
            _c: &CompanyId,
            _key: &str,
            _v: crate::ports::types::SecretValue,
        ) -> crate::Result<()> {
            Err(crate::error::OpenCompanyError::Store("boom".into()))
        }
    }

    let company = CompanyId::new("acme");
    let resolved = TenantComposio::resolve(
        &company,
        &BrokenSecrets,
        Vec::new(),
        None,
        // An instance identity IS available — and must still not be used,
        // because we cannot tell whether this company has a key of its own.
        Some(Arc::new(TinyhumansTokenSource::static_key(
            "platform-identity",
        ))),
    )
    .await;
    assert!(
        resolved.is_none(),
        "an unreadable store must withhold the tools, not present the instance's identity"
    );
}

/// The rotation guarantee (issue #586 acceptance): rotating the company key
/// moves the roster fingerprint, so agents cannot be left on the previous
/// credential after a console rotation.
#[tokio::test]
async fn rotating_the_company_key_moves_the_composio_fingerprint() {
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-rotate-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");

    let resolve =
        async || TenantComposio::resolve(&company, &secrets, Vec::new(), None, None).await;

    company_key::store_key(&company, &secrets, "key-a")
        .await
        .unwrap();
    let before = TenantComposio::fingerprint(&resolve().await);

    company_key::store_key(&company, &secrets, "key-b")
        .await
        .unwrap();
    let after = TenantComposio::fingerprint(&resolve().await);
    assert_ne!(
        before, after,
        "a rotated company key must rebuild the roster, or agents keep the old credential"
    );

    // And clearing it is a change too — the roster must drop the tools.
    company_key::store_key(&company, &secrets, "")
        .await
        .unwrap();
    assert_eq!(
        TenantComposio::fingerprint(&resolve().await),
        TenantComposio::fingerprint(&None),
        "a cleared credential resolves to nothing, so no tools are wired"
    );
}

/// The bearer a config would present right now.
async fn token_of(config: &TenantComposio) -> Option<String> {
    config.current_token().await.expect("resolves")
}

fn config_with(credential: Credential) -> Option<TenantComposio> {
    Some(TenantComposio::new(
        "https://api.tinyhumans.ai",
        credential,
        vec!["gmail".to_string()],
    ))
}

/// The rotation contract: a projected platform token whose bytes change every
/// few minutes must NOT move the roster fingerprint, or every agent's tool
/// roster is rebuilt on the cluster's rotation schedule. A tier change or a
/// changed *stored* token must still move it.
#[tokio::test]
async fn fingerprint_is_stable_across_a_projected_rotation() {
    let dir = tempfile::Builder::new()
        .prefix("oc-composio-fp-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "token-before").unwrap();

    let projected = config_with(Credential::from_source(Arc::new(
        TinyhumansTokenSource::projected_file(&path),
    )));
    let before = TenantComposio::fingerprint(&projected);
    assert_eq!(
        token_of(projected.as_ref().unwrap()).await.as_deref(),
        Some("token-before")
    );

    // The kubelet rewrites the file in place.
    std::fs::write(&path, "token-after").unwrap();
    assert_eq!(
        token_of(projected.as_ref().unwrap()).await.as_deref(),
        Some("token-after"),
        "the call must pick up the rotated token"
    );
    assert_eq!(
        TenantComposio::fingerprint(&projected),
        before,
        "a rotation must NOT rebuild the roster"
    );

    // A different projected path is a different identity.
    let other = config_with(Credential::from_source(Arc::new(
        TinyhumansTokenSource::projected_file(dir.path().join("other")),
    )));
    assert_ne!(TenantComposio::fingerprint(&other), before);
}

#[test]
fn fingerprint_moves_on_tier_and_stored_value_changes() {
    let a = config_with(Credential::from_value("token-a"));
    let b = config_with(Credential::from_value("token-b"));
    assert_ne!(
        TenantComposio::fingerprint(&a),
        TenantComposio::fingerprint(&b),
        "a rotated stored token must move the fingerprint"
    );
    assert_ne!(
        TenantComposio::fingerprint(&a),
        TenantComposio::fingerprint(&None),
        "None (fail-closed) must differ from a configured tenant"
    );
    assert_eq!(
        TenantComposio::fingerprint(&a),
        TenantComposio::fingerprint(&a.clone()),
        "the same config fingerprints stably"
    );

    // Swapping a stored token for the platform identity is a tier change.
    let attested = config_with(Credential::from_source(Arc::new(
        TinyhumansTokenSource::projected_file("/var/run/secrets/tinyhumans.ai/token"),
    )));
    assert_ne!(
        TenantComposio::fingerprint(&attested),
        TenantComposio::fingerprint(&a),
        "a tier change must move the fingerprint"
    );
}

/// Switching routes is an identity change even when the credential's bytes
/// do not move: the same string means a different Composio account
/// depending on which host it is presented to, so the roster has to rebuild
/// or the agents keep calling the account the company just left.
#[test]
fn fingerprint_moves_when_the_route_changes() {
    let managed = config_with(Credential::from_value("same-bytes"));
    let byok = Some(TenantComposio::from_access(
        "https://api.tinyhumans.ai",
        crate::company::composio::ComposioAccess {
            mode: ComposioMode::Byok,
            credential: Credential::from_value("same-bytes"),
        },
        vec!["gmail".to_string()],
    ));
    assert_ne!(
        TenantComposio::fingerprint(&managed),
        TenantComposio::fingerprint(&byok),
        "managed and BYOK must not fingerprint alike"
    );
}

/// Under BYOK the config carries a *second* live credential — the
/// managed-chain bearer `list_toolkits` fetches OpenHuman's curated catalog
/// with. Rotating a company's TinyHumans key while BYOK is active changes
/// neither `mode` nor the Composio `credential`, so this bearer is the only
/// thing that moves; if the fingerprint did not cover it, the roster would
/// keep the stale one until some unrelated change happened to rebuild it,
/// and the curated fetch would keep failing on a bearer the operator
/// already rotated away from.
#[test]
fn fingerprint_moves_when_the_catalog_credential_rotates() {
    let byok_with = |catalog: Credential| {
        Some(
            TenantComposio::from_access(
                "https://api.tinyhumans.ai",
                crate::company::composio::ComposioAccess {
                    mode: ComposioMode::Byok,
                    credential: Credential::from_value("ak_live"),
                },
                vec!["gmail".to_string()],
            )
            .with_catalog_credential(catalog),
        )
    };
    let before = byok_with(Credential::from_value("th-company-a"));
    let after = byok_with(Credential::from_value("th-company-b"));
    assert_ne!(
        TenantComposio::fingerprint(&before),
        TenantComposio::fingerprint(&after),
        "rotating the catalog bearer alone must still rebuild the roster"
    );

    // A managed config's `catalog` is always `Credential::None` (see
    // `TenantComposio::new`), so this must be a genuine no-op there rather
    // than a source of spurious rebuilds on every managed roster build.
    let managed_a = config_with(Credential::from_value("same-bytes"));
    let managed_b = config_with(Credential::from_value("same-bytes"));
    assert_eq!(
        TenantComposio::fingerprint(&managed_a),
        TenantComposio::fingerprint(&managed_b),
        "an always-None catalog credential must not itself vary the managed fingerprint"
    );
}

/// The endpoint a config reports is the host it dials — under BYOK that is
/// Composio itself, whatever managed backend URL was resolved alongside it.
#[test]
fn a_byok_config_reports_composios_own_host() {
    let managed = TenantComposio::new(
        "https://api.tinyhumans.ai",
        Credential::from_value("k"),
        vec![],
    );
    assert_eq!(managed.endpoint(), "https://api.tinyhumans.ai");
    assert_eq!(managed.mode(), ComposioMode::Managed);

    let byok = TenantComposio::from_access(
        "https://api.tinyhumans.ai",
        crate::company::composio::ComposioAccess {
            mode: ComposioMode::Byok,
            credential: Credential::from_value("k"),
        },
        vec![],
    );
    assert_eq!(byok.endpoint(), DIRECT_BASE_URL);
    assert_eq!(byok.mode(), ComposioMode::Byok);
}

/// The hazard `from_access` exists to make unrepresentable: a BYOK
/// credential is a **Composio** key, and pairing it with the managed route
/// would send it to `api.tinyhumans.ai` as a bearer. Building from the
/// resolved pair carries the route with the credential, so the two cannot
/// be separated by a caller that forgets.
#[test]
fn a_byok_credential_cannot_be_built_onto_the_managed_route() {
    let config = TenantComposio::from_access(
        "https://api.tinyhumans.ai",
        crate::company::composio::ComposioAccess {
            mode: ComposioMode::Byok,
            credential: Credential::from_value("ak_live"),
        },
        vec![],
    );
    assert_eq!(config.mode(), ComposioMode::Byok);
    assert_eq!(
        config.endpoint(),
        DIRECT_BASE_URL,
        "the key must be presented to Composio, never to the managed backend"
    );
}

/// A BYOK company still resolves the managed chain — not to act through, but
/// to ask OpenHuman which providers to offer. The two credentials are kept
/// apart: the Composio key is what calls present, the managed bearer is only
/// ever the curated list's.
#[tokio::test]
async fn byok_keeps_the_managed_credential_for_the_curated_catalog_only() {
    use crate::company::company_key;
    use crate::company::composio::store_api_key;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-catalog-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");

    company_key::store_key(&company, &secrets, "th_company")
        .await
        .unwrap();
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");

    assert_eq!(config.mode(), ComposioMode::Byok);
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live"),
        "calls present the company's own Composio key"
    );
    assert_eq!(
        config.catalog_token().await.unwrap().as_deref(),
        Some("th_company"),
        "the curated list is fetched with the managed credential, not the Composio key"
    );
}

/// With no managed tier at all — a standalone host carrying no TinyHumans
/// identity — there is no curated list to fetch, and the config says so
/// rather than presenting the Composio key to the OpenHuman backend.
#[tokio::test]
async fn byok_without_a_managed_tier_has_no_curated_catalog_credential() {
    use crate::company::composio::store_api_key;
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-standalone-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live")
    );
    assert!(
        config.catalog_token().await.unwrap().is_none(),
        "no managed tier means no curated list — never the Composio key standing in for one"
    );
}

/// The roster path honours the stored route: a company that brought its own
/// Composio account resolves to a BYOK config carrying that key, and one
/// that selected BYOK without storing a key resolves to **no tools** rather
/// than to the platform identity standing in for it.
#[tokio::test]
async fn resolve_follows_the_stored_route() {
    use crate::company::composio::{BYOK_MODE, MODE_KEY, store_api_key};
    use crate::store::FsSecretStore;

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-byok-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    store_api_key(&company, &secrets, "ak_live").await.unwrap();

    let config = TenantComposio::resolve(&company, &secrets, vec![], None, None)
        .await
        .expect("a BYOK company has a config");
    assert_eq!(config.mode(), ComposioMode::Byok);
    assert_eq!(
        config.current_token().await.unwrap().as_deref(),
        Some("ak_live")
    );

    // BYOK selected with nothing stored: fail closed.
    let bare = CompanyId::new("bare");
    secrets
        .set(&bare, MODE_KEY, SecretValue(BYOK_MODE.into()))
        .await
        .unwrap();
    assert!(
        TenantComposio::resolve(&bare, &secrets, vec![], None, None)
            .await
            .is_none(),
        "an operator who asked for their own account must never silently get the platform's"
    );
}
}

/// The console-facing ops helpers ([`authorize_connect_url`],
/// [`list_connection_states`]) over a mock Composio backend: proves the connect
/// URL is surfaced, the allowlist is enforced before any network call, and
/// connection rows aggregate to per-toolkit `connected` state filtered to the
/// tenant grant.
#[cfg(all(test, feature = "composio"))]
mod ops_helper_tests {
use super::*;

use std::net::SocketAddr;

use crate::company::composio::CatalogEntry;

use axum::Router;
use axum::routing::{get, post};
use serde_json::{Value, json};

/// Mock `POST /agent-integrations/composio/authorize` — returns a hosted
/// connect URL inside the backend's `{success,data}` envelope.
async fn authorize_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "connectUrl": "https://connect.composio.dev/abc", "connectionId": "conn-xyz" }
    }))
}

/// Mock `GET /agent-integrations/composio/connections` — gmail has one
/// active + one pending row (→ connected), slack only pending (→ not
/// connected), notion active (filtered out unless allowlisted).
///
/// The identity fields exercise each arm of the account-label precedence
/// (issue #404): `c1` publishes an email, `c2` only a blank one plus a
/// workspace, `c3` only a username, `c4` nothing at all.
async fn connections_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "connections": [
            {
                "id": "c1", "toolkit": "gmail", "status": "ACTIVE",
                "createdAt": "2026-08-01T10:00:00Z",
                "accountEmail": " ops@acme.test ",
                "username": "ignored-when-an-email-is-present"
            },
            {
                "id": "c2", "toolkit": "gmail", "status": "INITIATED",
                "accountEmail": "   ",
                "workspace": "Acme Workspace"
            },
            { "id": "c3", "toolkit": "slack", "status": "INITIATED", "username": "acme-bot" },
            { "id": "c4", "toolkit": "notion", "status": "ACTIVE" }
        ] }
    }))
}

/// Mock `GET /agent-integrations/composio/toolkits` — the dynamic catalog
/// shape (backend #1012): a `toolkits` allowlist plus a `catalog[]` whose
/// entries carry an `enabled` gate. `zendesk` is present but not connectable
/// and must not be advertised; the casing and whitespace on `HubSpot` must
/// normalise.
///
/// Entries carry the display metadata (`logo`, `description`, `categories`)
/// the backend actually publishes — issue #600 is that all of it was dropped
/// on the way through, so a mock that omitted it could not have caught the
/// bug.
async fn toolkits_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": {
            "toolkits": ["gmail", "slack"],
            "catalog": [
                {
                    "slug": " HubSpot ",
                    "name": "HubSpot",
                    "enabled": true,
                    "logo": " https://logos.composio.dev/api/hubspot ",
                    "description": "  CRM and marketing automation.  ",
                    "categories": ["crm", " marketing ", ""]
                },
                {
                    "slug": "gmail",
                    "name": "Gmail",
                    "enabled": true,
                    "description": "Send and read email.",
                    "categories": ["email"]
                },
                { "slug": "zendesk", "name": "Zendesk", "enabled": false },
                { "slug": "gmail", "name": "Gmail (dup)", "enabled": true }
            ]
        }
    }))
}

/// Mock toolkits route for a backend predating the dynamic catalog: the
/// plain slug allowlist and no `catalog[]` at all.
async fn legacy_toolkits_handler() -> axum::Json<Value> {
    axum::Json(json!({
        "success": true,
        "data": { "toolkits": ["Notion", "gmail", ""] }
    }))
}

async fn spawn_backend() -> String {
    spawn_backend_with(get(toolkits_handler)).await
}

async fn spawn_backend_with(toolkits: axum::routing::MethodRouter) -> String {
    let app = Router::new()
        .route(
            "/agent-integrations/composio/authorize",
            post(authorize_handler),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(connections_handler),
        )
        .route("/agent-integrations/composio/toolkits", toolkits);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn config(url: &str, toolkits: Vec<String>) -> TenantComposio {
    TenantComposio::new(
        url.to_string(),
        Credential::from_value("tenant-token"),
        toolkits,
    )
}

#[tokio::test]
async fn authorize_returns_hosted_connect_url() {
    let url = spawn_backend().await;
    let out = authorize_connect_url(&config(&url, vec!["gmail".into()]), "gmail")
        .await
        .expect("authorize returns a connect URL");
    assert_eq!(out, "https://connect.composio.dev/abc");
}

#[tokio::test]
async fn authorize_rejects_toolkit_outside_allowlist_before_any_network_call() {
    // Backend URL is unreachable — the allowlist rejection must fire first.
    let out =
        authorize_connect_url(&config("http://127.0.0.1:1", vec!["gmail".into()]), "slack")
            .await;
    let err = out.expect_err("a toolkit outside the allowlist must be refused");
    assert!(err.to_string().contains("allowlist"), "{err}");
}

#[tokio::test]
async fn list_connection_states_aggregates_active_and_filters_to_allowlist() {
    let url = spawn_backend().await;
    // gmail + slack allowed; notion is active upstream but not in the grant.
    let states = list_connection_states(&config(&url, vec!["gmail".into(), "slack".into()]))
        .await
        .expect("list connections");
    assert_eq!(
        states,
        vec![("gmail".to_string(), true), ("slack".to_string(), false)],
        "gmail active (one ACTIVE row), slack pending only, notion filtered out"
    );
}

/// Issue #404: the detail view needs the account behind a connection, not
/// just that one exists. Pins the whole projection — per-connection rows
/// (two for gmail, where the fold gives one), the raw status, the account
/// label precedence, and the `(toolkit, id)` order — against the same
/// allowlist filter the fold applies.
#[tokio::test]
async fn list_connections_detailed_projects_each_account_with_its_identity() {
    let url = spawn_backend().await;
    let rows = list_connections_detailed(&config(&url, vec!["gmail".into(), "slack".into()]))
        .await
        .expect("list connections");

    // Compared as whole rows rather than as a tuple projection, so a field
    // added to `ComposioConnectionRow` later cannot slip past this
    // assertion unexamined.
    let expect = |id: &str,
                  toolkit: &str,
                  status: &str,
                  connected: bool,
                  created_at: Option<&str>,
                  account: Option<&str>| ComposioConnectionRow {
        id: id.to_string(),
        toolkit: toolkit.to_string(),
        status: status.to_string(),
        connected,
        created_at: created_at.map(str::to_string),
        account: account.map(str::to_string),
    };
    assert_eq!(
        rows,
        vec![
            // Email wins over the username the same row carries, and is
            // trimmed.
            expect(
                "c1",
                "gmail",
                "ACTIVE",
                true,
                Some("2026-08-01T10:00:00Z"),
                Some("ops@acme.test"),
            ),
            // A blank email is not an email: falls through to the workspace.
            // Kept as its own row rather than folded into c1 — this is the
            // "two Gmail accounts" case a disconnect has to tell apart.
            expect(
                "c2",
                "gmail",
                "INITIATED",
                false,
                None,
                Some("Acme Workspace"),
            ),
            // Username is the last resort.
            expect("c3", "slack", "INITIATED", false, None, Some("acme-bot")),
        ],
        "one row per connection, sorted by (toolkit, id); notion filtered out \
         by the allowlist exactly as the fold filters it"
    );
}

/// The fold the tile grid and the reconciliation probe read must keep
/// meaning what it meant before #404 widened the call underneath it —
/// `connected` is still "any account active", not "the first one".
#[tokio::test]
async fn the_per_toolkit_fold_still_summarises_the_detailed_rows() {
    let url = spawn_backend().await;
    let cfg = config(&url, vec!["gmail".into(), "slack".into()]);
    let rows = list_connections_detailed(&cfg).await.expect("rows");
    let states = list_connection_states(&cfg).await.expect("states");

    let folded: std::collections::BTreeMap<String, bool> =
        rows.into_iter().fold(Default::default(), |mut acc, r| {
            let e = acc.entry(r.toolkit).or_insert(false);
            *e = *e || r.connected;
            acc
        });
    assert_eq!(
        states,
        folded.into_iter().collect::<Vec<_>>(),
        "the states route is exactly the OR-fold of the detailed rows"
    );
}

/// Issue #404 + #403: an id this company's own reads will not show must not
/// be deletable by naming it. The mock serves no DELETE route at all, so a
/// request that got as far as dialling would fail loudly rather than pass —
/// the refusal has to come from the guard, before the call.
#[tokio::test]
async fn disconnect_refuses_an_id_outside_this_companys_visible_connections() {
    let url = spawn_backend().await;
    // `c4` (notion) is a real, active connection upstream — but this
    // company's manifest does not grant notion, so no read here surfaces
    // it. That is the case the guard exists for: the bearer *could* delete
    // it, and the allowlist must be a boundary rather than a display filter.
    let err = delete_connection(&config(&url, vec!["gmail".into()]), "c4")
        .await
        .expect_err("a connection outside the grant is not deletable");
    // The *variant* is the assertion, not the message: it is what decides
    // the status code the console sees, and asserting only on the string
    // is what let a refusal ship as a `502`.
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "a refused id must be NotFound, not an upstream failure: {err:?}"
    );

    // And an id that exists nowhere at all fails the same way.
    let err = delete_connection(&config(&url, vec!["gmail".into()]), "nope")
        .await
        .expect_err("an unknown id is not deletable");
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
}

/// An empty / whitespace id is refused before the list is even fetched —
/// and as a client mistake, not as an unreachable backend.
#[tokio::test]
async fn disconnect_refuses_a_blank_id_before_any_network_call() {
    // Unreachable backend — the argument check must fire first. If it did
    // not, this would surface as `Upstream`, which is what the assertion
    // below rules out.
    let err = delete_connection(&config("http://127.0.0.1:1", vec!["gmail".into()]), "  ")
        .await
        .expect_err("a blank id is refused");
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
}

/// Issue #820: an account that is not usable cannot be the one agents act
/// as. `c2` is a real gmail connection of this company's, and `INITIATED` —
/// pinning it would route every gmail send to an account that cannot send,
/// which is worse than the unpinned behaviour it replaces. So the refusal is
/// a product decision, not a validation nicety, and it is asserted with the
/// store: a refusal that still wrote would be a broken toolkit with a
/// reassuring error message.
///
/// The two blunter refusals share the test because they share the guard, and
/// the assertion that matters for all three is the same one — nothing
/// reached [`crate::company::composio::set_default`].
#[tokio::test]
async fn pinning_an_account_that_cannot_send_is_refused_and_stores_nothing() {
    use crate::company::composio::load_defaults;
    use crate::ports::types::CompanyId;
    use crate::store::FsSecretStore;

    let url = spawn_backend().await;
    let dir = tempfile::Builder::new()
        .prefix("oc-composio-pin-")
        .tempdir()
        .expect("tempdir");
    let secrets = FsSecretStore::new(dir.path());
    let company = CompanyId::new("acme");
    let cfg = config(&url, vec!["gmail".into(), "slack".into()]);

    let err = set_default_connection(&cfg, &company, &secrets, "c2")
        .await
        .expect_err("an account that is not connected cannot be pinned");
    // `NotFound` and not `Upstream`: the backend answered fine, and the
    // console must render this as the operator's mistake with the fix in it
    // ("re-authorize it"), not as a provider outage.
    assert!(
        matches!(err, DisconnectError::NotFound(_)),
        "unexpected error: {err:?}"
    );
    assert!(
        err.to_string().contains("INITIATED") && err.to_string().contains("not connected"),
        "the message names the status the operator has to fix: {err}"
    );

    // An id belonging to nobody, and an id belonging to this company under a
    // toolkit its manifest does not grant — the same boundary
    // `delete_connection` draws, so a pin cannot reach what no read shows.
    for id in ["nope", "c4", "   "] {
        match set_default_connection(&cfg, &company, &secrets, id).await {
            Err(DisconnectError::NotFound(_)) => {}
            other => panic!("`{id}` must be refused as NotFound, got {other:?}"),
        }
    }

    assert!(
        load_defaults(&company, &secrets)
            .await
            .expect("defaults read")
            .is_empty(),
        "a refused pin must not be stored — the whole point is that the next \
         agent turn is unchanged"
    );

    // The control: `c1` is the same toolkit, ACTIVE, and goes through. Without
    // it a guard that refused everything would pass every assertion above.
    let toolkit = set_default_connection(&cfg, &company, &secrets, "c1")
        .await
        .expect("an active account is pinnable");
    assert_eq!(toolkit, "gmail", "the pinned toolkit is reported back");
    assert_eq!(
        load_defaults(&company, &secrets)
            .await
            .expect("defaults read")
            .get("gmail")
            .map(String::as_str),
        Some("c1")
    );
}

/// The console's open-mode source (issue #397): the backend's real catalog,
/// normalised. Connectable entries only, trimmed + lowercased, de-duplicated,
/// sorted.
#[tokio::test]
async fn list_catalog_toolkits_returns_the_backends_connectable_catalog() {
    let url = spawn_backend().await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");
    assert_eq!(
        catalog.iter().map(|e| e.slug.as_str()).collect::<Vec<_>>(),
        vec!["gmail", "hubspot"],
        "connectable entries only, normalised, de-duplicated and sorted"
    );
}

/// Issue #600: the display metadata the backend publishes reaches the
/// caller instead of being reduced to a slug.
///
/// This is the regression test for the defect itself. Every field asserted
/// here was present in the response and discarded by a single
/// `.map(|entry| entry.slug)`, which is why the console had nothing to
/// group by, nothing to brand with, and nothing to search but the slug.
#[tokio::test]
async fn list_catalog_toolkits_carries_the_display_metadata() {
    let url = spawn_backend().await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");

    let hubspot = catalog
        .iter()
        .find(|e| e.slug == "hubspot")
        .expect("hubspot is connectable");
    assert_eq!(hubspot.name, "HubSpot");
    assert_eq!(hubspot.description, "CRM and marketing automation.");
    assert_eq!(
        hubspot.logo.as_deref(),
        Some("https://logos.composio.dev/api/hubspot"),
        "the logo URL is what lets a tile be branded rather than a text row"
    );
    assert_eq!(
        hubspot.categories,
        vec!["crm".to_string(), "marketing".to_string()],
        "categories are trimmed and emptied-out entries dropped, but otherwise \
         forwarded verbatim — the console buckets them, not this layer"
    );

    let gmail = catalog
        .iter()
        .find(|e| e.slug == "gmail")
        .expect("gmail is connectable");
    assert_eq!(gmail.description, "Send and read email.");
    assert_eq!(
        gmail.logo, None,
        "an unpublished logo is None, not an empty string the console would \
         render as a broken image"
    );
    assert_eq!(
        gmail.name, "Gmail",
        "the FIRST entry for a slug wins, matching the de-duplication the slug \
         set used to do — not the later `Gmail (dup)`"
    );
}

/// A backend predating the dynamic catalog sends no `catalog[]`. Its plain
/// slug allowlist is used rather than reporting an empty catalog — which the
/// console would (correctly) render as a degraded fallback.
#[tokio::test]
async fn list_catalog_toolkits_falls_back_to_the_plain_allowlist() {
    let url = spawn_backend_with(get(legacy_toolkits_handler)).await;
    let catalog = list_catalog_toolkits(&config(&url, Vec::new()))
        .await
        .expect("catalog fetch");
    assert_eq!(
        catalog,
        vec![
            CatalogEntry::from_slug("gmail"),
            CatalogEntry::from_slug("notion"),
        ],
        "slug-only entries: the backend published nothing else, and the console \
         renders these with its own typography rather than dropping them"
    );
}

/// An unreachable backend is an error, never a quietly-empty catalog — the
/// caller has to be able to tell "nothing is permitted" from "I could not
/// ask".
#[tokio::test]
async fn list_catalog_toolkits_surfaces_a_fetch_failure() {
    let out = list_catalog_toolkits(&config("http://127.0.0.1:1", Vec::new())).await;
    out.expect_err("an unreachable backend must not read as an empty catalog");
}

#[tokio::test]
async fn list_connection_states_empty_allowlist_admits_every_toolkit() {
    let url = spawn_backend().await;
    let states = list_connection_states(&config(&url, Vec::new()))
        .await
        .expect("list connections");
    assert_eq!(
        states,
        vec![
            ("gmail".to_string(), true),
            ("notion".to_string(), true),
            ("slack".to_string(), false),
        ]
    );
}
}

/// The mandatory tenant-isolation test (issue #110): two per-tenant configs (A
/// and B) over a mock backend that records the `Authorization` header of each
/// request and answers with tenant-specific data. Proves the ONLY isolation
/// lever — which token the client is constructed with — actually holds: A's
/// request carries token A (never B), and A's result carries only A's account.
#[cfg(all(test, feature = "composio"))]
mod isolation_tests {
use super::*;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use oh::tools::traits::Tool;
use openhuman_core as oh;
use serde_json::{Value, json};

/// Shared recorder for every `Authorization` header the mock backend saw.
type AuthLog = Arc<Mutex<Vec<String>>>;

/// The mock `/agent-integrations/composio/connections` handler: records the
/// bearer it received and returns a `{success,data}` envelope whose single
/// connection's `account_email` is derived from that bearer — so a caller can
/// prove it only ever sees *its own* tenant's data.
async fn connections(State(log): State<AuthLog>, headers: HeaderMap) -> axum::Json<Value> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    log.lock().unwrap().push(auth.clone());
    // Derive tenant identity purely from the presented bearer.
    let email = if auth.contains("token-a") {
        "a@example.com"
    } else if auth.contains("token-b") {
        "b@example.com"
    } else {
        "unknown@example.com"
    };
    axum::Json(json!({
        "success": true,
        "data": {
            "connections": [
                { "id": "conn-1", "toolkit": "gmail", "status": "ACTIVE", "accountEmail": email }
            ]
        }
    }))
}

/// Spawn the mock backend on an ephemeral port; returns its base URL + the
/// auth-header recorder.
async fn spawn_backend() -> (String, AuthLog) {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/connections", get(connections))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), log)
}

fn config(url: &str, token: &str) -> TenantComposio {
    TenantComposio::new(url, Credential::from_value(token), Vec::new())
}

fn list_connections_tool(config: &TenantComposio) -> Box<dyn Tool> {
    let metering = ComposioMetering {
        company: CompanyId::new("acme"),
        agent: "ceo".to_string(),
        meter: None,
    };
    composio_tools(config, metering)
        .into_iter()
        .find(|t| t.name() == "composio_list_connections")
        .expect("composio_list_connections tool present")
}

#[tokio::test]
async fn each_tenant_only_ever_carries_its_own_token_and_sees_its_own_accounts() {
    let (url, log) = spawn_backend().await;

    let tool_a = list_connections_tool(&config(&url, "token-a"));
    let tool_b = list_connections_tool(&config(&url, "token-b"));

    let out_a = tool_a.execute(json!({})).await.unwrap();
    let text_a = out_a.output();
    let out_b = tool_b.execute(json!({})).await.unwrap();
    let text_b = out_b.output();

    // A saw only A's account; never B's account nor B's token.
    assert!(
        text_a.contains("a@example.com"),
        "A missing its account: {text_a}"
    );
    assert!(
        !text_a.contains("b@example.com"),
        "A leaked B's account: {text_a}"
    );
    assert!(!text_a.contains("token-b"), "A leaked B's token: {text_a}");
    // Symmetrically for B.
    assert!(
        text_b.contains("b@example.com"),
        "B missing its account: {text_b}"
    );
    assert!(
        !text_b.contains("a@example.com"),
        "B leaked A's account: {text_b}"
    );

    // A's own token is scrubbed out of its own successful output.
    assert!(
        !text_a.contains("token-a"),
        "A leaked its own token: {text_a}"
    );

    // The backend received exactly the two distinct bearers — each request
    // carried its own tenant's token, never the other's.
    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "expected one request per tenant: {seen:?}");
    assert!(
        seen.iter().any(|a| a == "Bearer token-a"),
        "missing A bearer: {seen:?}"
    );
    assert!(
        seen.iter().any(|a| a == "Bearer token-b"),
        "missing B bearer: {seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|a| a.contains("token-a") && a.contains("token-b")),
        "a single request must never carry both tokens: {seen:?}"
    );
}

/// The rotation contract at the tool boundary: a projected platform token the
/// cluster rewrites in place must reach the backend on the **next** call, with
/// no roster rebuild — and the freshly-resolved value must be the one the
/// scrub vector protects, so a backend that reflects it still cannot leak it.
#[tokio::test]
async fn a_rotated_projected_token_is_presented_and_scrubbed_per_call() {
    use crate::company::credentials::TinyhumansTokenSource;

    // Reflect the bearer back inside an envelope failure, and record it.
    async fn reflect(State(log): State<AuthLog>, headers: HeaderMap) -> axum::Json<Value> {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        log.lock().unwrap().push(auth.clone());
        axum::Json(json!({ "success": false, "error": format!("upstream said: {auth}") }))
    }
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/connections", get(reflect))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let dir = tempfile::Builder::new()
        .prefix("oc-composio-rot-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-secret-before").unwrap();

    // ONE config, built once — exactly what a roster holds across turns.
    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_source(Arc::new(TinyhumansTokenSource::projected_file(&path))),
        Vec::new(),
    );
    let tool = list_connections_tool(&config);

    let first = tool.execute(json!({})).await.unwrap();
    assert!(
        !first.output().contains("projected-secret-before"),
        "the resolved token leaked into agent-visible output: {}",
        first.output()
    );

    // The kubelet rewrites the file in place; the SAME tool must present the
    // new token and scrub that one.
    std::fs::write(&path, "projected-secret-after").unwrap();
    let second = tool.execute(json!({})).await.unwrap();
    assert!(
        !second.output().contains("projected-secret-after"),
        "the rotated token leaked into agent-visible output: {}",
        second.output()
    );

    let seen = log.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![
            "Bearer projected-secret-before".to_string(),
            "Bearer projected-secret-after".to_string()
        ],
        "each call must carry the token the file held at that moment: {seen:?}"
    );
}

/// A mock backend that echoes the caller's bearer inside an error body; the
/// tool's scrub must strip it before the agent ever sees it.
#[tokio::test]
async fn error_body_reflecting_the_token_is_scrubbed() {
    async fn reflect(headers: HeaderMap) -> axum::Json<Value> {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // A 2xx envelope failure whose message reflects the raw bearer.
        axum::Json(json!({ "success": false, "error": format!("upstream said: {auth}") }))
    }
    let app = Router::new().route("/agent-integrations/composio/connections", get(reflect));
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!("http://{addr}");

    let tool = list_connections_tool(&config(&url, "reflected-secret-token"));
    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        !text.contains("reflected-secret-token"),
        "the reflected token leaked into agent-visible output: {text}"
    );
}

// --- FAIL-axis: what each Composio tool does when the backend fails ------

use axum::http::StatusCode;
use axum::routing::post;

/// A handler that always 5xxs, recording each hit so a caller can count the
/// requests a single tool call actually made.
async fn always_500(State(log): State<AuthLog>) -> (StatusCode, axum::Json<Value>) {
    log.lock().unwrap().push("hit".to_string());
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(json!({ "success": false, "error": "upstream exploded" })),
    )
}

/// Spawn a backend whose every Composio route 5xxs.
async fn spawn_failing_backend() -> (String, AuthLog) {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/toolkits", get(always_500))
        .route("/agent-integrations/composio/tools", get(always_500))
        .route("/agent-integrations/composio/connections", get(always_500))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), log)
}

fn tool_named(config: &TenantComposio, name: &str) -> Box<dyn Tool> {
    let metering = ComposioMetering {
        company: CompanyId::new("acme"),
        agent: "ceo".to_string(),
        meter: None,
    };
    composio_tools(config, metering)
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("`{name}` tool present"))
}

/// A hard backend failure on the toolkit catalogue must surface as an error,
/// never as an empty-but-successful listing. The distinction is the whole
/// point: an agent told "no toolkits" concludes the company has connected
/// nothing and stops asking, while an agent told the catalogue could not be
/// read can say so and retry later.
#[tokio::test]
async fn list_toolkits_reports_a_backend_failure_rather_than_an_empty_catalogue() {
    let (url, _log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_toolkits");

    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 from the catalogue must be an error, not a listing: {text}"
    );
    assert!(
        !text.contains("token-a"),
        "the tenant token leaked into the failure text: {text}"
    );
}

/// The same contract on the action catalogue. `composio_list_tools` is what
/// an agent reads before it picks a slug, so an empty success here sends it
/// on to guess a slug that was never listed.
#[tokio::test]
async fn list_tools_reports_a_backend_failure_rather_than_an_empty_listing() {
    let (url, _log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_tools");

    let out = tool
        .execute(json!({ "search": "send email" }))
        .await
        .unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 from the action catalogue must be an error, not a listing: {text}"
    );
    assert!(
        !text.contains("token-a"),
        "the tenant token leaked into the failure text: {text}"
    );
}

/// Cross-tenant isolation must hold on the failure path too. A backend that
/// 5xxs gives the tool nothing to render, and the one thing it must not do
/// is fall back to any other source of connections — the output carries no
/// account at all, and no other tenant's bearer was ever presented.
#[tokio::test]
async fn a_backend_failure_on_connections_yields_no_accounts_and_no_other_tenants_token() {
    let (url, log) = spawn_failing_backend().await;
    let tool = tool_named(&config(&url, "token-a"), "composio_list_connections");

    let out = tool.execute(json!({})).await.unwrap();
    let text = out.output();
    assert!(
        out.is_error,
        "a 500 on connections must be an error: {text}"
    );
    assert!(
        !text.contains("@example.com"),
        "a failed listing must render no account whatsoever: {text}"
    );
    assert!(
        !text.contains("token-a") && !text.contains("token-b"),
        "no bearer may appear in the failure text: {text}"
    );
    let seen = log.lock().unwrap().len();
    assert!(seen >= 1, "the call must actually have reached the backend");
}

/// A company that has configured no Composio credential at all must have
/// every tool refuse before the network, rather than calling the backend
/// unauthenticated and rendering whatever it returns.
#[tokio::test]
async fn an_absent_credential_refuses_every_tool_before_the_network() {
    let (url, log) = spawn_failing_backend().await;
    let config = TenantComposio::new(url, Credential::None, Vec::new());

    for name in [
        "composio_list_toolkits",
        "composio_list_connections",
        "composio_list_tools",
    ] {
        let out = tool_named(&config, name).execute(json!({})).await.unwrap();
        assert!(
            out.is_error,
            "`{name}` must refuse without a credential: {}",
            out.output()
        );
    }
    assert!(
        log.lock().unwrap().is_empty(),
        "no request may leave for a company that configured no credential"
    );
}

#[tokio::test]
async fn a_repeated_authorize_for_one_toolkit_is_deduped() {
    let log: AuthLog = Arc::new(Mutex::new(Vec::new()));
    async fn authorize(State(log): State<AuthLog>) -> axum::Json<Value> {
        log.lock().unwrap().push("authorize".to_string());
        axum::Json(json!({
            "success": true,
            "data": { "connectUrl": "https://connect.composio.dev/abc", "connectionId": "conn-1" }
        }))
    }
    let app = Router::new()
        .route("/agent-integrations/composio/authorize", post(authorize))
        .route(
            "/agent-integrations/composio/connections",
            get(async || {
                axum::Json(json!({
                    "success": true,
                    "data": { "connections": [
                        { "id": "conn-1", "toolkit": "gmail", "status": "INITIATED" }
                    ] }
                }))
            }),
        )
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_value("token-a"),
        vec!["gmail".to_string()],
    );
    let tool = tool_named(&config, "composio_authorize");

    let first = tool.execute(json!({ "toolkit": "gmail" })).await.unwrap();
    assert!(!first.is_error, "{}", first.output());
    let second = tool.execute(json!({ "toolkit": "GMAIL" })).await.unwrap();
    assert!(!second.is_error, "{}", second.output());

    assert_eq!(
        log.lock().unwrap().len(),
        1,
        "a repeat authorize for the same toolkit must not open a second handoff"
    );
}

/// Repeated managed executes carry the same backend idempotency key.
#[tokio::test]
async fn a_repeated_execute_carries_an_idempotency_key_the_backend_can_dedupe_on() {
    type BodyLog = Arc<Mutex<Vec<(Value, Option<String>)>>>;
    async fn execute(
        State(log): State<BodyLog>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let key = headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        log.lock().unwrap().push((body, key));
        axum::Json(json!({
            "success": true,
            "data": { "successful": true, "data": { "id": "msg-1" } }
        }))
    }
    let log: BodyLog = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/agent-integrations/composio/execute", post(execute))
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config = TenantComposio::new(
        format!("http://{addr}"),
        Credential::from_value("token-a"),
        vec!["gmail".to_string()],
    );
    let tool = tool_named(&config, "composio_execute");

    let args = json!({
        "tool": "GMAIL_SEND_EMAIL",
        "arguments": { "to": "ops@acme.test", "subject": "hi", "body": "hello" }
    });
    let _ = tool.execute(args.clone()).await.unwrap();
    let _ = tool.execute(args).await.unwrap();

    let seen = log.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "both calls must have reached the backend");
    let keys: Vec<Option<String>> = seen.iter().map(|(_, k)| k.clone()).collect();
    assert!(
        keys.iter().all(Option::is_some),
        "a side-effecting execute must carry an idempotency key: {keys:?}"
    );
    assert_eq!(
        keys[0], keys[1],
        "two identical executes must present the SAME key so the backend can dedupe"
    );
}
