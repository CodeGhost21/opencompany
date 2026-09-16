use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

/// The resolved bearer for a decl, for the assertions below.
async fn bearer(decl: &InferenceDecl) -> Option<String> {
    decl.bearer().await.expect("credential resolves")
}

fn inference(provider: &str) -> Inference {
    Inference {
        provider: Some(provider.to_string()),
        base_url: None,
        api_key_secret: None,
        models: BTreeMap::new(),
    }
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

// ---- precedence matrix -------------------------------------------------

#[tokio::test]
async fn runtime_beats_manifest_beats_env() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/v1".into(),
        credential: Credential::from_value("env-key"),
    };
    let mut manifest = inference("openai_compatible");
    manifest.base_url = Some("https://manifest.example/v1".into());

    // Env only.
    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .expect("env default resolves");
    assert_eq!(decl.source, InferenceSource::Default);
    assert_eq!(decl.provider, DEFAULT_PROVIDER);
    assert!(decl.is_proxied(), "the default rides the subscription");
    assert_eq!(decl.telemetry_slug(), "subscription");
    assert_eq!(bearer(&decl).await.as_deref(), Some("env-key"));

    // Manifest beats env.
    let decl = resolve_effective(&company, &manifest, Some(&env), &secrets)
        .await
        .unwrap()
        .expect("manifest resolves");
    assert_eq!(decl.source, InferenceSource::Manifest);
    assert_eq!(decl.provider, "openai_compatible");
    assert_eq!(decl.base_url, "https://manifest.example/v1");

    // Runtime beats manifest.
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openrouter".into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    store_key(&company, &secrets, "or-secret").await.unwrap();
    let decl = resolve_effective(&company, &manifest, Some(&env), &secrets)
        .await
        .unwrap()
        .expect("runtime resolves");
    assert_eq!(decl.source, InferenceSource::Runtime);
    assert_eq!(decl.provider, "openrouter");
    assert_eq!(decl.base_url, OPENROUTER_BASE_URL);
    assert!(!decl.is_proxied(), "a tenant key goes direct");
    assert_eq!(decl.telemetry_slug(), "openrouter");
    assert_eq!(bearer(&decl).await.as_deref(), Some("or-secret"));
    assert!(decl.key_configured());
}

/// A keyless `openrouter` inherits the platform endpoint and credential
/// rather than dropping them — the branch `managed` used to own. Without it a
/// company that names its provider but holds no key of its own would 401
/// instead of riding the subscription.
#[tokio::test]
async fn keyless_openrouter_inherits_the_platform_endpoint_and_credential() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    let decl = resolve_effective(&company, &inference("openrouter"), Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.source, InferenceSource::Manifest);
    assert_eq!(decl.provider, "openrouter");
    assert_eq!(decl.base_url, "https://env.example/openai/v1");
    assert!(decl.is_proxied());
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
}

/// A keyless `openrouter` with a tenant-supplied `base_url` override goes
/// direct with **no** credential — the platform token must not ride an
/// arbitrary endpoint the operator pointed it at.
#[tokio::test]
async fn keyless_openrouter_never_sends_the_platform_credential_to_an_override() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    let mut manifest = inference("openrouter");
    manifest.base_url = Some("https://attacker.example/v1".into());
    let decl = resolve_effective(&company, &manifest, Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.base_url, "https://attacker.example/v1");
    assert!(
        !decl.is_proxied(),
        "an arbitrary endpoint is not the subscription"
    );
    assert!(
        !decl.key_configured(),
        "a keyless config holds no credential to send"
    );
    assert_eq!(decl.telemetry_slug(), "openrouter");
    assert_eq!(
        bearer(&decl).await,
        None,
        "the platform credential stays home"
    );
}

/// A company that declares nothing and rides the platform's endpoint is on
/// the managed route, and says so.
///
/// The console sends a keyless managed save as a *revert* — a managed brain
/// with no key of its own is the platform default rather than an override —
/// so this arm is what answers the operator immediately after they choose
/// "Managed (TinyHumans)" and press Save. Answering `openrouter` (the
/// provider underneath the platform endpoint) is the second way that choice
/// used to disappear from the card, and the one a stored-override fix alone
/// does not reach.
#[tokio::test]
async fn the_platform_default_reports_itself_as_the_managed_route() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.selected_provider(), LEGACY_MANAGED);
    assert_eq!(
        decl.provider, DEFAULT_PROVIDER,
        "and it still resolves to, and is billed as, proxied OpenRouter"
    );
    assert!(decl.is_proxied());
    assert_eq!(decl.telemetry_slug(), "subscription");
    assert_eq!(decl.source, InferenceSource::Default);
}

/// The operator's own choice survives the round trip, so a console can echo
/// it back.
///
/// `provider` answers "where does this resolve to", which is the right
/// question everywhere but the read-back: `managed` and `openrouter` resolve
/// identically, so reporting the resolved kind made "Managed (TinyHumans)"
/// impossible to hold on screen — the console seeds its provider select from
/// the status verbatim, so a `managed` save that read back `openrouter`
/// snapped the select (and the managed-only Connect button with it) straight
/// back. The two facts are now separate fields rather than one field asked
/// two questions.
#[tokio::test]
async fn a_saved_managed_choice_reads_back_as_managed() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: LEGACY_MANAGED.to_string(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();

    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decl.selected_provider(),
        LEGACY_MANAGED,
        "the console asked what was chosen"
    );
    assert_eq!(
        decl.provider, DEFAULT_PROVIDER,
        "and resolution is unchanged — every request path still sees OpenRouter"
    );
    assert!(decl.is_proxied());
    assert_eq!(decl.telemetry_slug(), "subscription");
}

/// Every other route reports one answer to both questions, so nothing but
/// the managed alias can drift between them.
#[tokio::test]
async fn a_non_aliased_choice_reads_back_unchanged() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    for kind in ["openrouter", "ollama", "openai_compatible"] {
        save_runtime_config(
            &company,
            &secrets,
            &RuntimeInference {
                provider: kind.to_string(),
                base_url: Some("http://127.0.0.1:9/v1".into()),
                models: BTreeMap::new(),
            },
        )
        .await
        .unwrap();
        let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(decl.selected_provider(), kind);
        assert_eq!(decl.provider, kind);
    }
}

/// `tinyhumans` is the wizard's spelling of the same route. It has to
/// canonicalize to the one spelling the console renders, or the select is
/// handed a value its own provider table has no row for and falls back.
#[test]
fn both_spellings_of_the_managed_route_canonicalize() {
    assert_eq!(selected_kind(LEGACY_MANAGED), LEGACY_MANAGED);
    assert_eq!(selected_kind("tinyhumans"), LEGACY_MANAGED);
    assert_eq!(selected_kind("  managed  "), LEGACY_MANAGED);
    assert_eq!(selected_kind("openrouter"), "openrouter");
    assert_eq!(
        selected_kind(""),
        DEFAULT_PROVIDER,
        "naming nothing is not naming the managed route"
    );
}

/// A committed manifest still saying `provider = "managed"` resolves as
/// proxied OpenRouter rather than failing. It was valid when written, and the
/// intent — "the platform's brain" — is exactly what proxied OpenRouter is.
#[tokio::test]
async fn a_legacy_managed_manifest_aliases_to_proxied_openrouter() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    let decl = resolve_effective(&company, &inference(LEGACY_MANAGED), Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.provider, DEFAULT_PROVIDER);
    assert!(decl.is_proxied());
    assert_eq!(decl.base_url, "https://env.example/openai/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
    assert_eq!(
        decl.selected_provider(),
        LEGACY_MANAGED,
        "the alias resolves onto OpenRouter without the console losing which \
         route was actually named"
    );
    assert!(
        validate_inference(&inference(LEGACY_MANAGED)).is_empty(),
        "and it still validates"
    );

    // The same alias applies to a stored runtime blob, which an operator
    // cannot hand-edit — the case that would otherwise strand a tenant.
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: LEGACY_MANAGED.into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.source, InferenceSource::Runtime);
    assert_eq!(decl.provider, DEFAULT_PROVIDER);
    assert!(decl.is_proxied());
}

/// A stored runtime blob naming a provider this build does not know fails
/// loudly rather than resolving to whatever the fallback happened to be.
#[tokio::test]
async fn an_unknown_stored_provider_is_an_error_not_a_silent_fallback() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "telepathy".into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    let err = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .expect_err("unknown provider must fail");
    let msg = err.to_string();
    assert!(msg.contains("telepathy"), "{msg}");
    assert!(msg.contains("openrouter"), "names what is valid: {msg}");
}

/// Issue #585: the company's own key is the admin's to set, and a key stored
/// through the console wins over the deploy-time env credential — otherwise
/// the only way to pay for a tenant is an environment variable the admin
/// cannot reach.
///
/// **What changed with `managed`'s removal.** Under `managed`, a console key
/// kept the *platform* endpoint, so an admin could bill their own account
/// through the proxy. `openrouter` is dual-mode instead: a key means an
/// OpenRouter key, so it goes direct to OpenRouter — sending an `sk-or-…` to
/// the platform proxy would simply be rejected. An admin who wants the
/// platform endpoint with a credential of their own now names
/// `openai_compatible` with that `base_url`.
#[tokio::test]
async fn a_console_key_wins_over_the_env_credential_and_goes_direct() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openrouter".into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    store_key(&company, &secrets, "company-key").await.unwrap();

    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .expect("runtime openrouter resolves");
    assert_eq!(decl.source, InferenceSource::Runtime);
    assert_eq!(decl.provider, "openrouter");
    assert_eq!(bearer(&decl).await.as_deref(), Some("company-key"));
    assert!(decl.key_configured());
    assert!(!decl.is_proxied(), "the tenant's own account pays");
    assert_eq!(decl.base_url, OPENROUTER_BASE_URL);

    // Clearing it falls back to the subscription rather than 401ing — the
    // property that makes a key genuinely optional in both directions.
    clear_key(&company, &secrets).await.unwrap();
    let decl = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .expect("still resolves with no key");
    assert!(decl.is_proxied());
    assert_eq!(decl.base_url, "https://env.example/openai/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
}

/// Issue #585 adds a second writer to `key_configured` — an admin setting the
/// company's key from the console — alongside the platform default the
/// manager injects. #636's `effective_status` split exists precisely to keep
/// the console's `keyConfigured` reporting *tenant* config and never the
/// platform token. Nothing asserted the two stay distinguishable, so this
/// does: on one company, the same call answers differently depending on
/// which source is in play.
#[tokio::test]
async fn a_console_key_and_the_platform_default_are_distinguishable() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let platform = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "managed".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Nothing tenant-scoped is stored yet. The platform-aware resolve is
    // credentialled — that is the value the console must NOT surface — while
    // the tenant-only resolve the read route uses reports "no key".
    let with_platform =
        resolve_effective(&company, &Inference::default(), Some(&platform), &secrets)
            .await
            .unwrap()
            .expect("platform default resolves");
    let tenant_only = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("runtime config resolves");
    assert!(
        with_platform.key_configured(),
        "the platform token is a real credential"
    );
    assert!(
        !tenant_only.key_configured(),
        "an injected platform token must never light up the console's `keyConfigured`"
    );

    // An admin sets the company's key. Now both agree — and the bearer the
    // agents present is the tenant's, not the platform's.
    store_key(&company, &secrets, "company-key").await.unwrap();
    let with_platform =
        resolve_effective(&company, &Inference::default(), Some(&platform), &secrets)
            .await
            .unwrap()
            .expect("platform default resolves");
    let tenant_only = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("runtime config resolves");
    assert!(
        tenant_only.key_configured(),
        "a console-set key is tenant config"
    );
    assert_eq!(bearer(&with_platform).await.as_deref(), Some("company-key"));
}

#[tokio::test]
async fn no_source_resolves_to_none() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap();
    assert!(
        decl.is_none(),
        "no source means the managed/echo brain stays"
    );
}

#[tokio::test]
async fn clearing_runtime_reverts_to_manifest() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let manifest = inference("openrouter");
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "ollama".into(),
            base_url: Some("http://localhost:11434/v1".into()),
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        resolve_effective(&company, &manifest, None, &secrets)
            .await
            .unwrap()
            .unwrap()
            .provider,
        "ollama"
    );
    clear_runtime_config(&company, &secrets).await.unwrap();
    assert_eq!(
        resolve_effective(&company, &manifest, None, &secrets)
            .await
            .unwrap()
            .unwrap()
            .provider,
        "openrouter"
    );
}

// ---- write-only key ----------------------------------------------------

#[tokio::test]
async fn key_is_write_only_and_never_serialized() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "sk-super-secret")
        .await
        .unwrap();
    let decl = resolve_effective(&company, &inference("openrouter"), None, &secrets)
        .await
        .unwrap()
        .unwrap();
    // The key resolves for request building…
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-super-secret"));
    // …but never appears in the Debug rendering.
    let debug = format!("{decl:?}");
    assert!(!debug.contains("sk-super-secret"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
}

#[tokio::test]
async fn cleared_key_reads_back_unconfigured() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    store_key(&company, &secrets, "tok").await.unwrap();
    assert!(key_configured(&company, &secrets, None).await.unwrap());
    clear_key(&company, &secrets).await.unwrap();
    assert!(!key_configured(&company, &secrets, None).await.unwrap());
}

#[tokio::test]
async fn manifest_api_key_secret_is_the_fallback_key() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    // Only the manifest-named key holds a token; canonical key is cold.
    secrets
        .set(
            &company,
            "byo/openrouter",
            SecretValue("named-secret".into()),
        )
        .await
        .unwrap();
    let mut manifest = inference("openrouter");
    manifest.api_key_secret = Some("byo/openrouter".into());
    let decl = resolve_effective(&company, &manifest, None, &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bearer(&decl).await.as_deref(), Some("named-secret"));
}

// ---- validation --------------------------------------------------------

#[test]
fn absent_section_is_inert() {
    assert!(validate_inference(&Inference::default()).is_empty());
}

#[test]
fn valid_configs_pass() {
    assert!(validate_inference(&inference("managed")).is_empty());
    assert!(validate_inference(&inference("openrouter")).is_empty());
    let mut ollama = inference("ollama");
    ollama.base_url = Some("http://localhost:11434/v1".into());
    assert!(
        validate_inference(&ollama).is_empty(),
        "{:?}",
        validate_inference(&ollama)
    );
}

#[test]
fn unknown_provider_is_rejected() {
    let problems = validate_inference(&inference("gpt5"));
    assert!(
        problems.iter().any(|p| p.contains("provider")),
        "{problems:?}"
    );
}

#[test]
fn ollama_and_openai_compatible_require_base_url() {
    let ollama = validate_inference(&inference("ollama"));
    assert!(
        ollama
            .iter()
            .any(|p| p.contains("base_url") && p.contains("required"))
    );
    let compat = validate_inference(&inference("openai_compatible"));
    assert!(
        compat
            .iter()
            .any(|p| p.contains("base_url") && p.contains("required"))
    );
}

#[test]
fn non_http_base_url_is_rejected() {
    let mut m = inference("openai_compatible");
    m.base_url = Some("ftp://x/v1".into());
    let problems = validate_inference(&m);
    assert!(problems.iter().any(|p| p.contains("http")), "{problems:?}");
}

#[test]
fn a_base_url_carrying_a_credential_is_rejected_and_never_echoed() {
    // Same rule as `api_key_secret` below, one field over: a credential
    // belongs in the write-only key slot, and a `base_url` is stored as
    // written and read back by every console reader.
    let mut m = inference("openai_compatible");
    m.base_url = Some("http://alice:hunter2@127.0.0.1:8597/v1".into());
    let problems = validate_inference(&m);
    assert!(
        problems.iter().any(|p| p.contains("username or password")),
        "{problems:?}"
    );
    // The refusal is the one moment this value is guaranteed to be shown to
    // somebody, so it must not quote the credential back.
    for problem in &problems {
        assert!(
            !problem.contains("hunter2") && !problem.contains("alice"),
            "a rejection echoed the credential it was rejecting: {problem}"
        );
    }

    // A malformed URL is quoted back redacted too — and the malformed ones
    // are the likeliest to have been typed by hand with a password in them.
    let mut bad = inference("openai_compatible");
    bad.base_url = Some("ftp://alice:hunter2@127.0.0.1/v1".into());
    for problem in validate_inference(&bad) {
        assert!(
            !problem.contains("hunter2"),
            "a rejection echoed the credential it was rejecting: {problem}"
        );
    }
}

#[test]
fn inline_credential_in_key_name_is_rejected() {
    let mut m = inference("openrouter");
    m.api_key_secret = Some("sk-or-v1-abcdef0123456789".into());
    let problems = validate_inference(&m);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("names a secret-store key")),
        "{problems:?}"
    );

    // A long opaque token with no separators is also caught.
    let mut m2 = inference("openrouter");
    m2.api_key_secret = Some("abcdefghijklmnopqrstuvwxyz0123456789ABCDEF".into());
    assert!(!validate_inference(&m2).is_empty());

    // A structured key name is accepted.
    let mut ok = inference("openrouter");
    ok.api_key_secret = Some("byo/openrouter".into());
    assert!(
        validate_inference(&ok).is_empty(),
        "{:?}",
        validate_inference(&ok)
    );
}

/// The isolation property named harnesses exist for: two `built_in`
/// harnesses on one company resolve independently, so one can ride the
/// subscription while the other runs on a key of its own.
#[tokio::test]
async fn two_harnesses_on_one_company_resolve_independently() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://env.example/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    let embedded = HarnessScope::default_harness("embedded");
    let deep = HarnessScope::named("deep");

    // Only `deep` gets a key.
    store_key_scoped(&company, &secrets, "sk-or-deep", &deep)
        .await
        .unwrap();

    let d = resolve_effective_scoped(
        &company,
        &inference("openrouter"),
        Some(&env),
        &secrets,
        &deep,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!d.is_proxied(), "deep pays its own way");
    assert_eq!(d.base_url, OPENROUTER_BASE_URL);
    assert_eq!(bearer(&d).await.as_deref(), Some("sk-or-deep"));

    let e = resolve_effective_scoped(
        &company,
        &inference("openrouter"),
        Some(&env),
        &secrets,
        &embedded,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(e.is_proxied(), "embedded is untouched by deep's key");
    assert_eq!(e.base_url, "https://env.example/v1");
    assert_eq!(bearer(&e).await.as_deref(), Some("platform-key"));
}

/// The default harness keeps the flat legacy keys, so a tenant whose console
/// already wrote `inference/key` keeps working with no migration — the store
/// has no rename, so getting this wrong would orphan every running company.
#[tokio::test]
async fn the_default_harness_reads_the_legacy_flat_keys() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();

    // Written the pre-harness way.
    store_key(&company, &secrets, "legacy-key").await.unwrap();
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openrouter".into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();

    // Read back through the scoped path, as the default harness.
    let scope = HarnessScope::default_harness("embedded");
    assert_eq!(scope.key_key(), KEY_KEY);
    assert_eq!(scope.config_key(), RUNTIME_CONFIG_KEY);

    let decl =
        resolve_effective_scoped(&company, &Inference::default(), None, &secrets, &scope)
            .await
            .unwrap()
            .expect("the legacy config resolves");
    assert_eq!(decl.source, InferenceSource::Runtime);
    assert_eq!(bearer(&decl).await.as_deref(), Some("legacy-key"));

    // A named harness namespaces instead, and sees none of it.
    let named = HarnessScope::named("deep");
    assert_eq!(named.key_key(), "harness/deep/inference/key");
    assert_eq!(named.config_key(), "harness/deep/inference/config");
    assert!(
        load_runtime_config_scoped(&company, &secrets, &named)
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn provider_slugs_map_as_documented() {
    assert_eq!(provider_slug("openrouter"), "openrouter");
    assert_eq!(provider_slug("openai_compatible"), "byok");
    assert_eq!(provider_slug("ollama"), "ollama");
    // The legacy kind slugs as what it now is, so historical usage rows and
    // new ones aggregate together.
    assert_eq!(provider_slug(LEGACY_MANAGED), "openrouter");
    // An unknown kind is never folded into a real provider's attribution.
    assert_eq!(provider_slug("mystery"), "unknown");
}

#[test]
fn effective_base_url_defaults_per_provider() {
    assert_eq!(
        effective_base_url(LEGACY_MANAGED, None),
        OPENROUTER_BASE_URL
    );
    assert_eq!(effective_base_url("openrouter", None), OPENROUTER_BASE_URL);
    assert_eq!(effective_base_url("ollama", None), OLLAMA_DEFAULT_BASE_URL);
    assert_eq!(
        effective_base_url("openrouter", Some("https://proxy/v1")),
        "https://proxy/v1"
    );
}

/// The defect: `lmstudio` and `omlx` had no arm, so the `_ =>` fallback
/// handed a **local** runtime OpenRouter's URL — and the decl carries the
/// credential the operator typed for the machine on their desk. A local
/// runtime's turns, and its key, would have left the host.
#[test]
fn a_local_runtime_never_falls_back_to_a_third_party_endpoint() {
    for kind in ["lmstudio", "omlx", "openai_compatible", "some-unknown-kind"] {
        let resolved = effective_base_url(kind, None);
        assert_ne!(
            resolved, OPENROUTER_BASE_URL,
            "{kind} must not inherit a third-party endpoint"
        );
        assert!(
            resolved.is_empty(),
            "{kind} has no guessable endpoint, so it must fail loudly: {resolved}"
        );
    }
    // An override is still honoured, which is the whole of how these kinds
    // are meant to be addressed.
    assert_eq!(
        effective_base_url("lmstudio", Some("http://localhost:1234/v1")),
        "http://localhost:1234/v1"
    );
}

#[test]
fn setup_accepts_the_localhost_spelling_local_model_apps_display() {
    assert_eq!(
        normalize_setup_base_url("ollama", Some("localhost:6969")),
        Some("http://localhost:6969/v1".to_string())
    );
    assert_eq!(
        normalize_setup_base_url("openai_compatible", Some("http://127.0.0.1:1234/v1/")),
        Some("http://127.0.0.1:1234/v1".to_string())
    );
    assert_eq!(
        normalize_setup_base_url("openai_compatible", Some("https://llm.test/api")),
        Some("https://llm.test/api".to_string())
    );
}

#[test]
fn setup_normalisation_reads_an_uppercase_scheme_as_a_scheme() {
    // Never a second scheme in front of the first: that shape is how a
    // credential once hid from `endpoint_has_credentials`.
    assert_eq!(
        normalize_setup_base_url("openai_compatible", Some("HTTP://127.0.0.1:1234")).as_deref(),
        Some("HTTP://127.0.0.1:1234/v1")
    );
    assert_eq!(
        normalize_setup_base_url("ollama", Some("HTTPS://llm.test/api")).as_deref(),
        Some("HTTPS://llm.test/api")
    );
    let credentialed =
        normalize_setup_base_url("openai_compatible", Some("HTTP://alice:hunter2@host/v1"))
            .expect("normalised");
    assert!(
        catalogue::endpoint_has_credentials(&credentialed),
        "`{credentialed}` must still read as carrying a credential"
    );
    // A single-slash scheme is a scheme, not a host: it is repaired to
    // `http://` rather than having a second one prepended, so the credential
    // stays in the first authority where the refusal reads it.
    let single_slash =
        normalize_setup_base_url("openai_compatible", Some("http:/alice:hunter2@host/v1"))
            .expect("normalised");
    assert_eq!(single_slash, "http://alice:hunter2@host/v1");
    assert!(
        catalogue::endpoint_has_credentials(&single_slash),
        "`{single_slash}` must still read as carrying a credential"
    );
    assert_eq!(
        normalize_setup_base_url("ollama", Some("http:/localhost:11434")).as_deref(),
        Some("http://localhost:11434/v1")
    );
    assert_eq!(
        normalize_setup_base_url("openai_compatible", Some("HTTPS:///llm.test/api")).as_deref(),
        Some("HTTPS://llm.test/api")
    );
}

// ---- first-run probe (decl_for_probe) ----------------------------------

fn managed_env() -> EnvDefault {
    EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::from_value("platform-key"),
    }
}

/// The managed card sends `provider = "managed"` and, because it has no URL
/// field, whatever `base_url` a previously-picked provider left in the form —
/// `openrouter.ai` here. The probe must ignore that stale endpoint and reach
/// the managed endpoint with the managed credential. On the pre-fix code this
/// went direct to `openrouter.ai` with no credential and 401'd.
#[tokio::test]
async fn managed_probe_ignores_stale_base_url_and_uses_managed_endpoint() {
    let env = managed_env();
    let decl = decl_for_probe(
        "managed",
        Some("https://openrouter.ai/api/v1"),
        None,
        Some(&env),
    );
    assert_eq!(decl.base_url, "https://env.example/openai/v1");
    assert!(decl.is_proxied());
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
}

/// A managed probe where the operator supplied their own TinyHumans key still
/// reaches the managed endpoint — not `openrouter.ai` — carrying that key.
#[tokio::test]
async fn managed_probe_with_own_key_keeps_the_managed_endpoint() {
    let env = managed_env();
    let decl = decl_for_probe(
        "managed",
        Some("https://openrouter.ai/api/v1"),
        Some("th-key"),
        Some(&env),
    );
    assert_eq!(decl.base_url, "https://env.example/openai/v1");
    assert!(decl.is_proxied());
    assert_eq!(bearer(&decl).await.as_deref(), Some("th-key"));
}

/// A host holding no managed credential probes the managed endpoint honestly
/// unauthenticated — so the failure names `api.tinyhumans.ai`, not the stale
/// `openrouter.ai` the form carried over.
#[tokio::test]
async fn managed_probe_without_env_default_reports_the_platform_endpoint() {
    let decl = decl_for_probe("managed", Some("https://openrouter.ai/api/v1"), None, None);
    assert_eq!(decl.base_url, PLATFORM_BASE_URL);
    assert_eq!(bearer(&decl).await, None);
}

/// The real providers must keep honouring the form's `base_url` and `key` —
/// the managed diversion must not over-correct them.
#[tokio::test]
async fn other_provider_probes_still_honour_the_form_endpoint_and_key() {
    let openrouter =
        decl_for_probe("openrouter", Some("https://proxy/v1"), Some("or-key"), None);
    assert_eq!(openrouter.base_url, "https://proxy/v1");
    assert!(!openrouter.is_proxied());
    assert_eq!(bearer(&openrouter).await.as_deref(), Some("or-key"));

    let compatible = decl_for_probe(
        "openai_compatible",
        Some("https://llm.test/v1"),
        Some("k"),
        None,
    );
    assert_eq!(compatible.base_url, "https://llm.test/v1");
    assert_eq!(bearer(&compatible).await.as_deref(), Some("k"));

    let ollama = decl_for_probe("ollama", None, None, None);
    assert_eq!(ollama.base_url, OLLAMA_DEFAULT_BASE_URL);
    assert_eq!(bearer(&ollama).await, None);
}

/// A keyless `openrouter` with its own `base_url` override still goes direct
/// and keyless — the platform credential must never ride an arbitrary
/// endpoint. Unchanged by the managed fix.
#[tokio::test]
async fn keyless_openrouter_override_probe_stays_direct_and_keyless() {
    let env = managed_env();
    let decl = decl_for_probe(
        "openrouter",
        Some("https://attacker.example/v1"),
        None,
        Some(&env),
    );
    assert_eq!(decl.base_url, "https://attacker.example/v1");
    assert!(!decl.is_proxied());
    assert_eq!(bearer(&decl).await, None);
}

// ---- the managed credential chain (issue #2266) -------------------------
//
// ```text
//   1. provider/tinyhumans/key   a key pasted specifically for inference
//   2. inference/key             the legacy address, read-only
//   3. tinyhumans/key            the company's account identity
//   4. instance identity         TINYHUMANS_TOKEN_FILE, else TINYHUMANS_API_KEY
//   5. nothing                   fail closed
// ```
//
// Steps 3 and 4 apply **only** when the vendor at the other end is the
// identity's own vendor. The OpenRouter test below is the important one.

async fn write(secrets: &MemSecrets, key: &str, value: &str) {
    secrets
        .set(&CompanyId::new("acme"), key, SecretValue(value.into()))
        .await
        .unwrap();
}

async fn resolve_managed(secrets: &MemSecrets) -> InferenceDecl {
    let company = CompanyId::new("acme");
    let config = RuntimeInference {
        provider: "managed".into(),
        base_url: None,
        models: BTreeMap::new(),
    };
    save_runtime_config(&company, secrets, &config)
        .await
        .unwrap();
    resolve_effective(
        &company,
        &Inference::default(),
        Some(&managed_env()),
        secrets,
    )
    .await
    .unwrap()
    .expect("a managed config resolves")
}

#[tokio::test]
async fn managed_with_a_pasted_inference_key_uses_it() {
    let secrets = MemSecrets::default();
    write(
        &secrets,
        &store::provider_key_key(MANAGED_SLUG),
        "sk-not-a-real-key",
    )
    .await;
    // Present but outranked, so the ordering is actually exercised.
    write(&secrets, crate::company::company_key::KEY_KEY, "th-account").await;

    let decl = resolve_managed(&secrets).await;
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key"));
    assert_eq!(decl.base_url, managed_env().base_url);
}

#[tokio::test]
async fn managed_falls_back_to_the_company_account_key_and_keeps_the_platform_endpoint() {
    // The substance of #2266: a company key set in the console reached
    // Composio and never reached inference, so setting it moved the app
    // connections onto the company's account and left every agent turn —
    // the expensive half — on whoever runs the server.
    let secrets = MemSecrets::default();
    write(&secrets, crate::company::company_key::KEY_KEY, "th-account").await;

    let decl = resolve_managed(&secrets).await;
    assert_eq!(bearer(&decl).await.as_deref(), Some("th-account"));
    // **Assert the endpoint, not only the bearer.** Sending a `th_…` key to
    // openrouter.ai is the shipped bug this chain must not reproduce, and a
    // test that checked the bearer alone is exactly how it shipped.
    assert_eq!(decl.base_url, managed_env().base_url);
    assert!(
        !decl.base_url.contains("openrouter.ai"),
        "{}",
        decl.base_url
    );
}

#[tokio::test]
async fn managed_with_neither_uses_the_instance_identity() {
    let secrets = MemSecrets::default();
    let decl = resolve_managed(&secrets).await;
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
    assert_eq!(decl.base_url, managed_env().base_url);
}

#[tokio::test]
async fn openrouter_never_receives_the_company_identity_or_the_instance_one() {
    // THE test. An identity flows to a surface only when the vendor at the
    // other end is the identity's own vendor: a `th_…` key means nothing to
    // OpenRouter, and presenting it there is both a failed request and a
    // credential disclosed to a third party.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    write(&secrets, crate::company::company_key::KEY_KEY, "th-account").await;

    let config = RuntimeInference {
        provider: "openrouter".into(),
        // An explicit endpoint is what makes this unambiguously the tenant's
        // own OpenRouter rather than the platform proxy in front of it.
        base_url: Some(OPENROUTER_BASE_URL.into()),
        models: BTreeMap::new(),
    };
    save_runtime_config(&company, &secrets, &config)
        .await
        .unwrap();
    let decl = resolve_effective(
        &company,
        &Inference::default(),
        Some(&managed_env()),
        &secrets,
    )
    .await
    .unwrap()
    .expect("an openrouter config resolves");

    assert_eq!(decl.base_url, OPENROUTER_BASE_URL);
    assert!(!decl.is_proxied());
    let presented = bearer(&decl).await;
    assert_ne!(
        presented.as_deref(),
        Some("th-account"),
        "the company identity leaked to a vendor"
    );
    assert_ne!(
        presented.as_deref(),
        Some("platform-key"),
        "the instance identity leaked to a vendor"
    );
    assert_eq!(
        presented, None,
        "no credential at all is the correct answer here"
    );
}

#[tokio::test]
async fn a_legacy_company_reads_the_flat_slot_and_one_save_moves_it() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    write(&secrets, KEY_KEY, "sk-not-a-real-key").await;

    let decl = resolve_managed(&secrets).await;
    assert_eq!(
        bearer(&decl).await.as_deref(),
        Some("sk-not-a-real-key"),
        "the legacy address is still read, so an untouched company keeps working"
    );

    // One save through the provider store converges the address.
    let zero = store::list_providers(&company, &secrets).await.unwrap()[0].clone();
    store::store_provider_key(&company, &secrets, &zero, "sk-not-a-real-key")
        .await
        .unwrap();
    assert_eq!(
        secrets.get(&company, KEY_KEY).await.unwrap(),
        Some(SecretValue(String::new())),
        "and clears the old one, so no secret is orphaned"
    );
    assert_eq!(
        secrets
            .get(&company, &store::provider_key_key(MANAGED_SLUG))
            .await
            .unwrap(),
        Some(SecretValue("sk-not-a-real-key".into()))
    );
}

// ---- the managed row's honest state -------------------------------------

#[test]
fn managed_reports_which_step_of_the_chain_answers() {
    // Not a boolean, and not "always on". The row that renders this used to
    // claim permanent availability, inherited from a design where the same
    // company runs the managed backend — here it needs a credential and can
    // resolve to nothing.
    let env = managed_env();
    let company = Credential::from_company_key("th-account");

    assert_eq!(
        managed_source(true, &company, Some(&env)),
        ManagedSource::ProviderKey,
        "a key pasted for inference outranks everything below it"
    );
    assert_eq!(
        managed_source(false, &company, Some(&env)),
        ManagedSource::CompanyAccount,
    );
    assert_eq!(
        managed_source(false, &Credential::None, Some(&env)),
        ManagedSource::Instance,
        "the server's account pays, and the row has to say so"
    );
    assert_eq!(
        managed_source(false, &Credential::None, None),
        ManagedSource::None,
        "nothing resolves — not set up, and not a green badge"
    );
}

#[test]
fn the_two_paying_states_are_not_collapsed() {
    // An operator deciding whether to connect their account needs to know
    // which one they are on. "On" for both hides the decision.
    let env = managed_env();
    assert_ne!(
        managed_source(
            false,
            &Credential::from_company_key("th-account"),
            Some(&env)
        ),
        managed_source(false, &Credential::None, Some(&env)),
    );
}

#[test]
fn an_env_default_that_would_yield_nothing_is_not_availability() {
    // `configured()` rather than presence: a projected-token source reports
    // itself present while its file can still yield nothing, and what
    // decides availability is whether a value would reach the wire.
    let empty = EnvDefault {
        base_url: "https://env.example/openai/v1".into(),
        credential: Credential::None,
    };
    assert_eq!(
        managed_source(false, &Credential::None, Some(&empty)),
        ManagedSource::None
    );
}

// ---- the provider list actually reaches the resolver ---------------------
//
// Every other test in this module seeds `inference/config` or exercises the
// store in isolation, which is exactly how a feature comes to be fully built
// on both sides and connected in neither: the write routes populated
// `inference/providers`, the status route rendered it, and nothing on the
// turn path ever read it. A company that added a provider through the
// console had configured its *display*, not its company — the chat pane said
// "no model configured" and was telling the truth.
//
// These write a provider through the store with NO legacy blob anywhere and
// assert a turn resolves to it.

async fn add_indexed(secrets: &MemSecrets, slug: &str, key: &str) {
    let company = CompanyId::new("acme");
    store::put_provider(
        &company,
        secrets,
        store::ProviderDraft {
            slug: slug.to_string(),
            label: slug.to_string(),
            kind: "openai_compatible".to_string(),
            base_url: format!("https://{slug}.example/v1"),
            models: BTreeMap::new(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    secrets
        .set(
            &company,
            &store::provider_key_key(slug),
            SecretValue(key.to_string()),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_provider_added_through_the_console_resolves_for_a_turn() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "acme", "sk-not-a-real-key").await;
    // Deliberately no `inference/config`: this is what a company that only
    // ever used the provider list looks like on disk.
    assert!(
        load_runtime_config(&company, &secrets)
            .await
            .unwrap()
            .is_none(),
        "the legacy blob must be absent for this test to mean anything"
    );

    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("a company with a provider resolves");
    assert_eq!(decl.base_url, "https://acme.example/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key"));
    assert!(decl.key_configured());
}

#[tokio::test]
async fn the_marked_default_is_the_one_a_turn_reaches() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "second", "sk-not-a-real-key-2").await;

    // No marker: list order, which is the behaviour that predates the marker.
    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.base_url, "https://first.example/v1");

    store::set_default_slug(&company, &secrets, "second")
        .await
        .unwrap();
    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        decl.base_url, "https://second.example/v1",
        "marking a default has to move where a turn actually goes, not just a badge"
    );
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key-2"));
}

#[tokio::test]
async fn a_disabled_provider_is_not_where_a_turn_goes() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "off", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "on", "sk-not-a-real-key-2").await;
    store::set_enabled(&company, &secrets, "off", false)
        .await
        .unwrap();

    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.base_url, "https://on.example/v1");
}

#[tokio::test]
async fn the_legacy_blob_still_wins_when_it_is_the_only_thing_there() {
    // Entry zero sorts first in the list, so a company that had one provider
    // before any of this existed keeps resolving exactly where it did. The
    // whole entry-zero design exists to make that true without a migration.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openai_compatible".into(),
            base_url: Some("https://legacy.example/v1".into()),
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    store_key(&company, &secrets, "sk-not-a-real-key")
        .await
        .unwrap();

    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decl.base_url, "https://legacy.example/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key"));
    assert_eq!(decl.source, InferenceSource::Runtime);
}

#[tokio::test]
async fn nothing_configured_still_resolves_to_nothing() {
    // The list being empty must not become a way to resolve *something*.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    assert!(
        resolve_effective(&company, &Inference::default(), None, &secrets)
            .await
            .unwrap()
            .is_none()
    );
}

// ---- the routing table actually reaches the resolver ---------------------
//
// The same shape as the block above, one layer along, and found the same
// way: the Routing tab wrote `inference/routes`, the status route rendered
// it, `provider_for_workload` decided over it in isolation — and the turn
// path never asked. Every row of that screen persisted, survived a reload,
// and changed nothing about where a turn went. A control that visibly fails
// is a bug; a control that reports success and is inert is a lie, and it is
// the harder one to find because nothing looks wrong.

/// The route a tier resolves to, as the wire model the plan would carry.
fn wire_model(decl: &InferenceDecl, tier: &str) -> String {
    model_on_the_wire(decl, tier).expect("a real model id")
}

async fn route(secrets: &MemSecrets, tier: &str, raw: &str) {
    let company = CompanyId::new("acme");
    let mut routes = store::load_routes(&company, secrets).await.unwrap();
    routes.insert(tier.to_string(), resolve::ProviderRef::parse(raw));
    store::save_routes(&company, secrets, &routes)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_route_sends_its_workload_to_the_provider_and_model_it_names() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "second", "sk-not-a-real-key-2").await;
    route(&secrets, "chat-v1", "second:deepseek/deepseek-v4-flash").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .expect("a routed workload resolves");
    assert_eq!(
        decl.base_url, "https://second.example/v1",
        "the route names the provider, so the turn goes there and not to the primary"
    );
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key-2"));
    assert_eq!(
        wire_model(&decl, "chat-v1"),
        "deepseek/deepseek-v4-flash",
        "the model the route pinned is the model that goes on the wire"
    );
}

#[tokio::test]
async fn the_route_beats_the_default_providers_own_tier_map() {
    // The sharpest form of the bug, and the one an operator hit: Use Your
    // Own Models wrote `anthropic:claude-sonnet-5` into all four tiers, and
    // the next turn sent `anthropic/claude-opus-5` to OpenRouter — neither
    // their provider nor their model. The route was inert in both
    // dimensions, and the two failures hid each other: with a route naming
    // the provider that was already the default, "route honoured" and "route
    // ignored" look identical. This asserts both halves at once by pointing
    // the route at a provider that is *not* the marked default, and pinning
    // a model the default's own tier map would answer differently.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();

    let mut openrouter_tiers = BTreeMap::new();
    openrouter_tiers.insert("chat-v1".to_string(), "anthropic/claude-opus-5".to_string());
    store::put_provider(
        &company,
        &secrets,
        store::ProviderDraft {
            slug: "openrouter".into(),
            label: "OpenRouter".into(),
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            models: openrouter_tiers,
            enabled: true,
        },
    )
    .await
    .unwrap();
    secrets
        .set(
            &company,
            &store::provider_key_key("openrouter"),
            SecretValue("sk-not-a-real-key-or".into()),
        )
        .await
        .unwrap();
    store::put_provider(
        &company,
        &secrets,
        store::ProviderDraft {
            slug: "anthropic".into(),
            label: "Anthropic".into(),
            kind: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            models: BTreeMap::new(),
            enabled: true,
        },
    )
    .await
    .unwrap();
    secrets
        .set(
            &company,
            &store::provider_key_key("anthropic"),
            SecretValue("sk-not-a-real-key-ant".into()),
        )
        .await
        .unwrap();
    store::set_default_slug(&company, &secrets, "openrouter")
        .await
        .unwrap();
    route(&secrets, "chat-v1", "anthropic:claude-sonnet-5").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .expect("the routed workload resolves");
    assert_eq!(
        decl.base_url, "https://api.anthropic.com/v1",
        "the route names anthropic, so the turn goes to anthropic and not to the default"
    );
    assert_eq!(
        bearer(&decl).await.as_deref(),
        Some("sk-not-a-real-key-ant")
    );
    assert_eq!(
        wire_model(&decl, "chat-v1"),
        "claude-sonnet-5",
        "the route's model beats the default provider's own tier map"
    );
}

#[tokio::test]
async fn a_workload_with_no_route_still_falls_through_to_the_primary() {
    // The other half of the property: pinning one row must not move the
    // others. A fix that routed everything through the chat row would pass
    // the test above and be worse than the bug.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "second", "sk-not-a-real-key-2").await;
    route(&secrets, "chat-v1", "second:deepseek/deepseek-v4-flash").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "reasoning-v1",
    )
    .await
    .unwrap()
    .expect("an unrouted workload resolves");
    assert_eq!(decl.base_url, "https://first.example/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key-1"));
    // Neither row has a model of its own configured, so — keys rework,
    // issue #2306, slice 2d — an unmapped tier on either is refused
    // rather than guessed; the property under test is that this decl is
    // "first"'s, never "second"'s, which `base_url`/`bearer` above
    // already prove. Confirmed here too: if "second"'s routed model ever
    // leaked onto this decl, this would resolve to it instead of erroring.
    assert!(
        model_on_the_wire(&decl, "reasoning-v1").is_err(),
        "another row's pinned model must not leak onto this one"
    );
}

/// Round-3a review P1-4: a bare-slug default naming a provider this
/// company no longer has must fail the turn closed — never silently
/// hand it to `resolve::primary`'s first-enabled fallback, which is a
/// different account than the one the operator named.
#[tokio::test]
async fn a_default_naming_a_deleted_provider_fails_the_turn_closed() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    store::set_default_slug(&company, &secrets, "gone")
        .await
        .unwrap();

    let err = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .expect_err("a default naming a provider this company does not have must fail closed");
    let text = err.to_string();
    assert!(text.contains("gone"), "{text}");
    assert!(text.contains("removed"), "{text}");
}

/// Same decision, for a default naming a provider that still exists but
/// is switched off.
#[tokio::test]
async fn a_default_naming_a_switched_off_provider_fails_the_turn_closed() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    store::put_provider(
        &company,
        &secrets,
        store::ProviderDraft {
            slug: "second".into(),
            label: "Second".into(),
            kind: "openai_compatible".into(),
            base_url: "https://second.example/v1".into(),
            models: BTreeMap::new(),
            enabled: false,
        },
    )
    .await
    .unwrap();
    store::set_default_slug(&company, &secrets, "second")
        .await
        .unwrap();

    let err = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .expect_err("a default naming a switched-off provider must fail closed, not fall back");
    let text = err.to_string();
    assert!(text.contains("Second"), "{text}");
    assert!(text.contains("turned off"), "{text}");
}

#[tokio::test]
async fn a_route_naming_managed_resolves_through_the_managed_chain() {
    // `managed` is a word in the route grammar, not a provider slug. Read as
    // a slug it resolves to nothing and the workload fails closed — which is
    // what the Managed mode button writes into every row.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://platform.example/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    route(&secrets, "agentic-v1", "managed").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "agentic-v1",
    )
    .await
    .unwrap()
    .expect("a managed route resolves");
    assert!(decl.is_proxied(), "the managed route rides the platform");
    assert_eq!(decl.base_url, "https://platform.example/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("platform-key"));
}

#[tokio::test]
async fn a_named_harness_that_configured_itself_outranks_the_company_provider_list() {
    // `docs/spec/runtime/providers.md` has always said runtime, then
    // manifest, then default — **within a harness**. Putting the company's
    // provider list unconditionally above that inverted it, so connecting
    // the company's first provider in the console silently re-pointed a
    // harness with an account of its own at the company's, and charged it.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;

    // The default harness reads the company's list, which is the whole
    // point of the list existing.
    let default_scope = HarnessScope::default_harness("embedded");
    let shared = resolve_effective_scoped(
        &company,
        &inference("openrouter"),
        None,
        &secrets,
        &default_scope,
    )
    .await
    .unwrap()
    .expect("the default harness resolves through the connected provider");
    assert_eq!(shared.base_url, "https://first.example/v1");

    // A named harness that declared `[harness.inference]` of its own does
    // not. It resolves through what it declared.
    let own = HarnessScope::named("deep").declaring_own_inference(true);
    let mine =
        resolve_effective_scoped(&company, &inference("openrouter"), None, &secrets, &own)
            .await
            .unwrap()
            .expect("a harness with its own section resolves through it");
    assert_ne!(
        mine.base_url, "https://first.example/v1",
        "the company's connected provider must not outrank this harness's own section"
    );
    assert_eq!(
        mine.base_url, PLATFORM_BASE_URL,
        "with a section of its own and no key in it, this harness rides the subscription"
    );

    // And a named harness that declared nothing still inherits the
    // company's list — the carve-out is for a statement, not for a name.
    let inherits = HarnessScope::named("shallow");
    let theirs = resolve_effective_scoped(
        &company,
        &inference("openrouter"),
        None,
        &secrets,
        &inherits,
    )
    .await
    .unwrap()
    .expect("a harness with nothing of its own inherits");
    assert_eq!(theirs.base_url, "https://first.example/v1");
}

#[tokio::test]
async fn a_named_harness_reads_its_own_credential_before_the_company_wide_one() {
    // Step 1 of the chain is company-wide, so for a harness with inference
    // of its own it is a different owner's credential wearing the same
    // slug. A company connecting OpenRouter would otherwise have its key
    // substituted for the harness's — and a harness with its own base_url
    // would present it to a different gateway.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let named = HarnessScope::named("deep");

    secrets
        .set(
            &company,
            &provider_key_key("openrouter"),
            SecretValue("sk-company".into()),
        )
        .await
        .unwrap();
    // With nothing of its own, the harness inherits — which is what it did
    // before harness scopes existed.
    assert_eq!(
        load_inference_key_scoped(&company, &secrets, "openrouter", None, &named)
            .await
            .unwrap(),
        "sk-company"
    );

    store_key_scoped(&company, &secrets, "sk-deep", &named)
        .await
        .unwrap();
    assert_eq!(
        load_inference_key_scoped(&company, &secrets, "openrouter", None, &named)
            .await
            .unwrap(),
        "sk-deep",
        "the harness's own key wins once it has one"
    );
    // And the company's own resolution is untouched by either.
    assert_eq!(
        load_inference_key_scoped(
            &company,
            &secrets,
            "openrouter",
            None,
            &HarnessScope::default()
        )
        .await
        .unwrap(),
        "sk-company"
    );
}

#[tokio::test]
async fn managed_never_reads_a_legacy_slot_that_belongs_to_a_vendor_account() {
    // `inference/key` is one address with two possible owners. For a company
    // upgraded from a BYOK config it holds that vendor's key, and reading it
    // as managed's sent an OpenRouter credential to the platform URL — a
    // credential presented to an account that does not own it.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    // Entry zero is a vendor account, with its key still in the flat slot.
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openrouter".into(),
            base_url: None,
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    secrets
        .set(&company, KEY_KEY, SecretValue("sk-or-byok".into()))
        .await
        .unwrap();

    assert_eq!(
        load_managed_key(&company, &secrets, &HarnessScope::default())
            .await
            .unwrap(),
        "",
        "the vendor's key is not managed's to present"
    );
    // And the general reader still finds it for the row it belongs to.
    assert_eq!(
        load_inference_key_scoped(
            &company,
            &secrets,
            "openrouter",
            None,
            &HarnessScope::default()
        )
        .await
        .unwrap(),
        "sk-or-byok"
    );

    // Same rule a scope along: a named harness's own slot holds that
    // harness's credential for whatever it declared, which managed has no
    // more claim on than it does on entry zero's.
    let named = HarnessScope::named("deep");
    store_key_scoped(&company, &secrets, "sk-deep-byok", &named)
        .await
        .unwrap();
    assert_eq!(
        load_managed_key(&company, &secrets, &named).await.unwrap(),
        "",
        "a named harness's key is not managed's to present either"
    );
}

#[tokio::test]
async fn a_managed_key_on_a_fresh_company_is_what_its_turns_present() {
    // The console's Managed row writes `provider/tinyhumans/key`, and the
    // default branch read only `DEFAULT_PROVIDER`'s slot — so the key was
    // stored, reported as the step that answers, and never sent anywhere.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://platform.example/v1".into(),
        credential: Credential::from_value("instance-identity"),
    };

    // Nothing configured: the instance identity is what answers.
    let before = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .expect("the platform default resolves");
    assert_eq!(bearer(&before).await.as_deref(), Some("instance-identity"));

    secrets
        .set(
            &company,
            &provider_key_key(MANAGED_SLUG),
            SecretValue("sk-managed".into()),
        )
        .await
        .unwrap();
    let after = resolve_effective(&company, &Inference::default(), Some(&env), &secrets)
        .await
        .unwrap()
        .expect("the platform default still resolves");
    assert_eq!(
        bearer(&after).await.as_deref(),
        Some("sk-managed"),
        "the key the operator pasted is the one the turn presents"
    );
}

#[tokio::test]
async fn switching_managed_off_never_makes_the_status_unreadable() {
    // The refusal is a statement about a *turn*. Putting it in the shared
    // resolver put it in every status read too, so a company with nothing
    // but managed could switch it off and then get a 500 from
    // `GET …/inference` — no page, and therefore no switch to turn it back
    // on with. A read has to be able to describe the state that refuses.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://platform.example/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    store::set_managed_enabled(&company, &secrets, false)
        .await
        .unwrap();

    let described = resolve_effective(&company, &inference("managed"), Some(&env), &secrets)
        .await
        .expect("a status read must still resolve");
    assert!(
        described.is_some(),
        "the console has to render the row that switches it back on"
    );

    // And the turn path still refuses, which is the point of the switch.
    let err = resolve_effective_for_tier(
        &company,
        &inference("managed"),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .expect_err("a turn must not ride a switched-off managed");
    assert!(err.to_string().contains("switched off"), "{err}");
}

#[tokio::test]
async fn an_unset_workload_stops_falling_back_to_managed_once_it_is_switched_off() {
    // The unset row does not take the `Managed` branch — it falls through
    // the primary to the legacy chain — so honouring the switch only there
    // left a company whose environment resolves to the platform spending
    // after it had been told to stop. The console's own sentence for this
    // state is "Managed is switched off, so it is not a fallback."
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://platform.example/v1".into(),
        credential: Credential::from_value("platform-key"),
    };

    // On, and nothing connected: the platform is the fallback.
    let decl = resolve_effective_for_tier(
        &company,
        &inference("managed"),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .expect("managed is the fallback while it is on");
    assert!(decl.is_proxied());

    store::set_managed_enabled(&company, &secrets, false)
        .await
        .unwrap();
    let err = resolve_effective_for_tier(
        &company,
        &inference("managed"),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .expect_err("a switched-off managed must not keep serving unset workloads");
    assert!(err.to_string().contains("switched off"), "{err}");

    // A company on its own key is untouched by the switch: it was never
    // riding the platform, so there is nothing here to refuse.
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    let own = resolve_effective_for_tier(
        &company,
        &inference("managed"),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .expect("a connected provider is not managed");
    assert_eq!(own.base_url, "https://first.example/v1");
}

#[tokio::test]
async fn a_route_naming_managed_fails_closed_once_managed_is_switched_off() {
    // The switch is a statement about spend — "stop billing this account" —
    // and a switch that only moves a badge on the settings page keeps
    // billing it. That is the defect the routing table itself was added to
    // fix, one provider along: the page agreed and the spend continued.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    let env = EnvDefault {
        base_url: "https://platform.example/v1".into(),
        credential: Credential::from_value("platform-key"),
    };
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    route(&secrets, "agentic-v1", "managed").await;
    store::set_managed_enabled(&company, &secrets, false)
        .await
        .unwrap();

    let err = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "agentic-v1",
    )
    .await
    .expect_err("a switched-off managed row must not keep serving turns");
    let message = err.to_string();
    assert!(message.contains("switched off"), "{message}");
    assert!(message.contains("agentic"), "{message}");

    // And switching it back on restores it, so the refusal is the switch
    // rather than a route that has been broken by being touched.
    store::set_managed_enabled(&company, &secrets, true)
        .await
        .unwrap();
    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        Some(&env),
        &secrets,
        &HarnessScope::default(),
        "agentic-v1",
    )
    .await
    .unwrap()
    .expect("a managed route resolves again once it is switched back on");
    assert_eq!(decl.base_url, "https://platform.example/v1");
}

#[tokio::test]
async fn a_route_naming_a_provider_that_is_gone_fails_closed() {
    // An unset workload falls back, because nobody chose anything for it. A
    // route is a choice with a workload attached, so it fails rather than
    // quietly spending on an account the operator did not name.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    route(&secrets, "chat-v1", "ghost:gpt-5").await;

    let err = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .expect_err("a route naming nothing must not silently fall back");
    let message = err.to_string();
    assert!(message.contains("ghost"), "{message}");
    assert!(message.contains("chat"), "{message}");
}

#[tokio::test]
async fn a_route_naming_a_switched_off_provider_fails_closed_too() {
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "parked", "sk-not-a-real-key-2").await;
    store::set_enabled(&company, &secrets, "parked", false)
        .await
        .unwrap();
    route(&secrets, "vision-v1", "parked").await;

    let err = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "vision-v1",
    )
    .await
    .expect_err("a parked route is a choice that no longer works");
    assert!(err.to_string().contains("parked"), "{err}");
}

#[tokio::test]
async fn coding_reads_the_agentic_route_rather_than_one_of_its_own() {
    // The alias, asserted on the path that matters. A coding turn arrives
    // carrying `agentic-v1`, so this is really a statement about the tier
    // the route is keyed on — and it is the reason routes are keyed on tiers
    // rather than on workload names.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    add_indexed(&secrets, "second", "sk-not-a-real-key-2").await;
    route(&secrets, "agentic-v1", "second").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "agentic-v1",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(decl.base_url, "https://second.example/v1");
}

// ---- a routed-managed company is a configured company ---------------------
//
// The third instance of tonight's shape, and the one that reached furthest:
// `resolve_effective_scoped` is what `RuntimeBuilder::build` asks "is
// anything configured at all", and it had exactly two branches — the
// provider list, then the legacy chain. Managed lives in neither. It has no
// row in `inference/providers` (it resolves through a credential chain, not
// a record), and a company configured through the console's Managed row
// writes neither the runtime blob nor a manifest block: its credential goes
// to `provider/tinyhumans/key` and its choice goes to `inference/routes`.
//
// So a company routing every tier to `managed`, with a managed key stored,
// resolved `None` — and got the offline echo brain. Restarting the host did
// not help, because a fresh boot ran the identical computation. The
// turn-time resolver knew how to resolve those rows the whole time.

/// Stores a managed credential at the address the console's managed key
/// route writes — the new per-provider slot, not the legacy flat one.
async fn managed_key(secrets: &MemSecrets, key: &str) {
    secrets
        .set(
            &CompanyId::new("acme"),
            &store::provider_key_key(MANAGED_SLUG),
            SecretValue(key.to_string()),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_company_routed_to_managed_resolves_rather_than_landing_on_echo() {
    // The reported company, reproduced exactly: one provider, switched off,
    // every tier routed to `managed`, and a managed key stored.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "anthropic", "sk-not-a-real-key-anthropic").await;
    store::set_enabled(&company, &secrets, "anthropic", false)
        .await
        .unwrap();
    for tier in ["chat-v1", "reasoning-v1", "agentic-v1", "vision-v1"] {
        route(&secrets, tier, "managed").await;
    }
    managed_key(&secrets, "sk-not-a-real-key-managed").await;

    // The unrouted resolver — the one `RuntimeBuilder::build` calls, and the
    // one that used to answer `None` here and strand the company on echo.
    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("a company routed to managed with a managed key is configured");
    assert_eq!(
        bearer(&decl).await.as_deref(),
        Some("sk-not-a-real-key-managed"),
        "the credential is the managed key, reached through the managed chain"
    );
    assert!(
        decl.is_proxied(),
        "managed rides the platform endpoint, which is what entitles it to the chain"
    );

    // And it agrees with the turn-time path, which knew all along — the two
    // must not be able to disagree about whether this company can think.
    let routed = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .expect("the routed path resolves too");
    assert_eq!(routed.base_url, decl.base_url);
    assert_eq!(bearer(&routed).await, bearer(&decl).await);
}

#[tokio::test]
async fn routing_to_managed_while_the_switch_is_off_resolves_to_nothing() {
    // The boot path and the turn path have to give the same answer, which is
    // the whole reason this branch calls `managed_decl` rather than forming a
    // second opinion. `resolve_effective_for_tier` *refuses* an explicit
    // `managed` route while the switch is off, so a boot that reported this
    // company configured would select the harness brain and then hand every
    // turn to a resolver that errors — inference that looks live on the
    // status card and fails on contact.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    for tier in ["chat-v1", "reasoning-v1", "agentic-v1", "vision-v1"] {
        route(&secrets, tier, "managed").await;
    }
    managed_key(&secrets, "sk-not-a-real-key-managed").await;
    store::set_managed_enabled(&company, &secrets, false)
        .await
        .unwrap();

    assert!(
        resolve_effective(&company, &Inference::default(), None, &secrets)
            .await
            .unwrap()
            .is_none(),
        "a switched-off Managed is not somewhere a workload can be routed, \
         so it is not what makes this company configured either"
    );

    // The turn path's refusal is the other half of the same statement.
    assert!(
        resolve_effective_for_tier(
            &company,
            &Inference::default(),
            None,
            &secrets,
            &HarnessScope::default(),
            "chat-v1",
        )
        .await
        .is_err(),
        "and the routed path refuses, which is the answer boot now matches"
    );

    // Switching it back on restores it, so the gate is the switch and not
    // the credential — which is untouched throughout.
    store::set_managed_enabled(&company, &secrets, true)
        .await
        .unwrap();
    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("switched back on, the same rows resolve");
    assert_eq!(
        bearer(&decl).await.as_deref(),
        Some("sk-not-a-real-key-managed")
    );
}

#[tokio::test]
async fn routing_to_managed_with_nothing_behind_it_still_resolves_to_nothing() {
    // The other half, and the one that keeps the echo brain meaningful: the
    // new branch must widen "configured" only where something can actually
    // answer. A company that picked Managed and put no credential behind it
    // — no pasted key, no company account, no instance identity, because no
    // env default is passed — has configured nothing, and reporting it
    // configured would take it off the echo brain with nothing to think
    // with. That is the mirror-image bug, and it is worse.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    for tier in ["chat-v1", "reasoning-v1", "agentic-v1", "vision-v1"] {
        route(&secrets, tier, "managed").await;
    }

    assert!(
        resolve_effective(&company, &Inference::default(), None, &secrets)
            .await
            .unwrap()
            .is_none(),
        "a managed route with no credential behind it configures nothing"
    );
}

#[tokio::test]
async fn an_unset_route_is_not_a_managed_route() {
    // `ProviderRef::Default` is an absence, not a choice. It maps to
    // `Resolution::Primary` — the provider list, then the legacy chain, both
    // of which the new branch runs after. Counting it as managed would
    // report every company with a managed key configured regardless of what
    // its routing table says, and would quietly disagree with where the turn
    // actually goes.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    managed_key(&secrets, "sk-not-a-real-key-managed").await;

    assert!(
        !resolve::any_route_is_managed(&store::load_routes(&company, &secrets).await.unwrap()),
        "an empty routing table names managed nowhere"
    );
    assert!(
        resolve_effective(&company, &Inference::default(), None, &secrets)
            .await
            .unwrap()
            .is_none(),
        "a stored managed key with no route pointing at it does not configure the company"
    );
}

#[tokio::test]
async fn the_managed_branch_runs_last_and_changes_no_company_that_already_resolved() {
    // Precedence, asserted rather than assumed: the new branch is a tail, so
    // a company whose provider list already answers keeps answering there
    // even with every tier routed to managed and a managed key stored.
    // Widening a resolver is only safe if it can turn `None` into `Some` and
    // nothing else.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;
    for tier in ["chat-v1", "reasoning-v1", "agentic-v1", "vision-v1"] {
        route(&secrets, tier, "managed").await;
    }
    managed_key(&secrets, "sk-not-a-real-key-managed").await;

    let decl = resolve_effective(&company, &Inference::default(), None, &secrets)
        .await
        .unwrap()
        .expect("the provider list still answers");
    assert_eq!(decl.base_url, "https://first.example/v1");
    assert_eq!(bearer(&decl).await.as_deref(), Some("sk-not-a-real-key-1"));
}

#[tokio::test]
async fn a_route_naming_entry_zero_reaches_the_legacy_config() {
    // Entry zero is the legacy blob wearing a provider's clothes. A route
    // that names it has to reach the blob's own endpoint and credential —
    // not whichever indexed provider happens to be the primary.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    save_runtime_config(
        &company,
        &secrets,
        &RuntimeInference {
            provider: "openai_compatible".into(),
            base_url: Some("https://legacy.example/v1".into()),
            models: BTreeMap::new(),
        },
    )
    .await
    .unwrap();
    store_key(&company, &secrets, "sk-not-a-real-key-legacy")
        .await
        .unwrap();
    add_indexed(&secrets, "second", "sk-not-a-real-key-2").await;
    store::set_default_slug(&company, &secrets, "second")
        .await
        .unwrap();

    let zero = store::list_providers(&company, &secrets).await.unwrap()[0].clone();
    route(&secrets, "chat-v1", &format!("{}:gpt-5", zero.slug)).await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "chat-v1",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(decl.base_url, "https://legacy.example/v1");
    assert_eq!(
        bearer(&decl).await.as_deref(),
        Some("sk-not-a-real-key-legacy")
    );
    assert_eq!(wire_model(&decl, "chat-v1"), "gpt-5");
}

#[tokio::test]
async fn a_tier_nobody_routes_resolves_exactly_as_it_always_did() {
    // `embedding-v1` and friends have no row on the Routing tab. They must
    // not acquire one by accident, and they must not fail closed either.
    let company = CompanyId::new("acme");
    let secrets = MemSecrets::default();
    add_indexed(&secrets, "first", "sk-not-a-real-key-1").await;

    let decl = resolve_effective_for_tier(
        &company,
        &Inference::default(),
        None,
        &secrets,
        &HarnessScope::default(),
        "embedding-v1",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(decl.base_url, "https://first.example/v1");
}
