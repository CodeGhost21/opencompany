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
