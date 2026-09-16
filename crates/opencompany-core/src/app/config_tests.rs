use super::*;
use crate::company::CompanyManifest;

fn manifest_with_brain(mode: &str) -> CompanyManifest {
    let toml_src = format!("[company]\nname = \"X\"\n[brain]\nmode = \"{mode}\"\n");
    toml::from_str(&toml_src).expect("valid manifest")
}

fn default_manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"X\"\n").expect("valid manifest")
}

#[test]
fn defaults_fill_in_when_nothing_set() {
    let env = MapEnv::default();
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();

    assert_eq!(cfg.api_url, DEFAULT_API_URL);
    assert_eq!(cfg.tinyplace_api_url, DEFAULT_TINYPLACE_API_URL);
    assert_eq!(cfg.bind, DEFAULT_BIND);
    assert_eq!(cfg.brain_mode, BrainMode::Hosted);
    assert!(cfg.tinyhumans_credential.is_none());
    assert!(cfg.github_token.is_none());
    assert!(!cfg.cycles_available());

    // The manifest always supplies a brain mode, so its layer is Manifest.
    assert_eq!(prov.layer("brain_mode"), Some(ConfigLayer::Manifest));
    assert_eq!(prov.layer("api_url"), Some(ConfigLayer::Default));
    assert_eq!(prov.layer("bind"), Some(ConfigLayer::Default));
}

#[test]
fn data_dir_from_treats_empty_as_unset() {
    use std::ffi::OsString;
    // An empty OPENCOMPANY_DATA_DIR falls back to $HOME/.opencompany, not cwd.
    assert_eq!(
        data_dir_from(
            Some(OsString::from("")),
            Some(OsString::from("/home/u")),
            None
        ),
        PathBuf::from("/home/u/.opencompany")
    );
    // A set value is used verbatim.
    assert_eq!(
        data_dir_from(
            Some(OsString::from("/data")),
            Some(OsString::from("/home/u")),
            None
        ),
        PathBuf::from("/data")
    );
    // Neither set → the relative default.
    assert_eq!(
        data_dir_from(None, None, None),
        PathBuf::from(".opencompany")
    );
    // Windows: `USERPROFILE` stands in, so the data dir does not become a
    // relative path resolved against the working directory. Must agree with
    // `store::paths::resolve_home_from`, or a Windows host would split its
    // bundles from its workspace.
    assert_eq!(
        data_dir_from(None, None, Some(OsString::from("C:\\Users\\ada"))),
        PathBuf::from("C:\\Users\\ada").join(".opencompany")
    );
}

#[test]
fn resolve_propagates_workspace_to_runtime_config() {
    let env = MapEnv::default();
    let file = ConfigFile {
        workspace: WorkspaceSection {
            git_enabled: Some(true),
            clear_tmp_on_startup: Some(false),
            ..WorkspaceSection::default()
        },
        ..ConfigFile::default()
    };
    let (cfg, _) = resolve(&env, Some(&file), &default_manifest()).unwrap();
    assert!(!cfg.workspace.clear_tmp_on_startup);
    assert!(cfg.workspace.git_enabled);

    // An absent `[workspace]` section resolves to the default (clear on boot).
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();
    assert!(cfg.workspace.clear_tmp_on_startup);
    assert!(!cfg.workspace.git_enabled);
}

#[test]
fn default_mcp_servers_resolve_from_config_toml_and_are_normalized() {
    // Issue #527: the config layer is the whole "no code change" claim, so
    // it is asserted rather than trusted — a list that silently failed to
    // resolve looks identical to one nobody configured.
    fn entry(name: &str, endpoint: &str) -> crate::company::McpServer {
        crate::company::McpServer {
            name: name.to_string(),
            endpoint: endpoint.to_string(),
            ..Default::default()
        }
    }
    let env = MapEnv::default();

    // A clean entry reaches RuntimeConfig.
    let file = ConfigFile {
        default_mcp_servers: vec![entry("deepwiki", "https://deepwiki.example/mcp")],
        ..ConfigFile::default()
    };
    let (cfg, _) = resolve(&env, Some(&file), &default_manifest()).unwrap();
    assert_eq!(cfg.default_mcp_servers.len(), 1);
    assert_eq!(cfg.default_mcp_servers[0].name, "deepwiki");

    // An unshippable entry is dropped here, at the boundary, rather than
    // thinning the list on every company's first agent turn — and it does
    // not take the good one with it, nor fail the boot.
    let file = ConfigFile {
        default_mcp_servers: vec![
            entry("leaky", "https://api.example/mcp?apiKey=leaked"),
            entry("clean", "https://clean.example/mcp"),
        ],
        ..ConfigFile::default()
    };
    let (cfg, _) = resolve(&env, Some(&file), &default_manifest()).unwrap();
    let names: Vec<&str> = cfg
        .default_mcp_servers
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, vec!["clean"]);

    // Absent section => no defaults, and emphatically not a built-in list.
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();
    assert!(cfg.default_mcp_servers.is_empty());
}

#[test]
fn default_mcp_servers_parse_from_the_toml_array_of_tables() {
    // Pins the wire name operators actually type. A rename would compile
    // fine and silently stop reading their config.
    let file: ConfigFile = toml::from_str(
        r#"
        [[default_mcp_server]]
        name = "deepwiki"
        endpoint = "https://mcp.deepwiki.com/mcp"
        description = "Docs for public repos."
        "#,
    )
    .expect("parses");
    assert_eq!(file.default_mcp_servers.len(), 1);
    assert_eq!(file.default_mcp_servers[0].name, "deepwiki");
    assert_eq!(
        file.default_mcp_servers[0].endpoint,
        "https://mcp.deepwiki.com/mcp"
    );
}

#[test]
fn env_beats_config_toml_beats_manifest_beats_default() {
    // brain_mode: env wins over everything.
    let env = MapEnv::new([
        ("OPENCOMPANY_BRAIN_MODE", "sidecar"),
        ("OPENCOMPANY_BIND", "0.0.0.0:9000"),
    ]);
    let file = ConfigFile {
        brain_mode: Some("hosted".into()),
        bind: Some("127.0.0.1:1111".into()),
        api_url: Some("https://toml.example".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &manifest_with_brain("hosted")).unwrap();

    assert_eq!(cfg.brain_mode, BrainMode::Sidecar);
    assert_eq!(prov.layer("brain_mode"), Some(ConfigLayer::Env));
    assert_eq!(cfg.bind, "0.0.0.0:9000");
    assert_eq!(prov.layer("bind"), Some(ConfigLayer::Env));

    // api_url only in config.toml, so config.toml wins over the default.
    assert_eq!(cfg.api_url, "https://toml.example");
    assert_eq!(prov.layer("api_url"), Some(ConfigLayer::ConfigToml));
}

fn manifest_with_auth(mode: &str) -> CompanyManifest {
    let toml_src = format!("[company]\nname = \"X\"\n[users]\nmode = \"{mode}\"\n");
    toml::from_str(&toml_src).expect("valid manifest")
}

/// A manifest naming no mode signs people in by email — which is what every
/// company did before the mode existed, so no deployment changes behaviour
/// by upgrading.
#[test]
fn auth_mode_defaults_to_email() {
    let env = MapEnv::default();
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
    assert_eq!(cfg.auth_mode, AuthMode::Email);
    // The manifest always supplies one (serde fills the default), exactly
    // as it does for the brain mode.
    assert_eq!(prov.layer("auth_mode"), Some(ConfigLayer::Manifest));
}

#[test]
fn manifest_supplies_auth_mode_when_env_and_toml_absent() {
    let env = MapEnv::default();
    let (cfg, prov) = resolve(&env, None, &manifest_with_auth("wallet")).unwrap();
    assert_eq!(cfg.auth_mode, AuthMode::Wallet);
    assert_eq!(prov.layer("auth_mode"), Some(ConfigLayer::Manifest));
}

#[test]
fn config_toml_beats_the_manifest_for_auth_mode() {
    let env = MapEnv::default();
    let file = ConfigFile {
        auth_mode: Some("none".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &manifest_with_auth("email")).unwrap();
    assert_eq!(cfg.auth_mode, AuthMode::None);
    assert_eq!(prov.layer("auth_mode"), Some(ConfigLayer::ConfigToml));
}

/// The host has the last word. A packaged desktop build and a hosting
/// platform both need to guarantee a mode across whatever a company's
/// manifest happens to say.
#[test]
fn env_beats_everything_for_auth_mode() {
    let env = MapEnv::new([("OPENCOMPANY_AUTH_MODE", "wallet")]);
    let file = ConfigFile {
        auth_mode: Some("none".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &manifest_with_auth("email")).unwrap();
    assert_eq!(cfg.auth_mode, AuthMode::Wallet);
    assert_eq!(prov.layer("auth_mode"), Some(ConfigLayer::Env));
}

/// Not a silent fallback to email: "the sign-in you configured is not the
/// one you got" is invisible from a running host, so it fails at boot.
#[test]
fn an_unknown_auth_mode_is_a_config_error() {
    let env = MapEnv::new([("OPENCOMPANY_AUTH_MODE", "walet")]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("email, wallet, none"), "{message}");
    assert!(message.contains("walet"), "{message}");
}

#[test]
fn auth_mode_predicates_match_the_variants() {
    assert!(AuthMode::Email.has_login() && AuthMode::Email.uses_email());
    // A wallet company has a sign-in, but no mailbox anywhere in it.
    assert!(AuthMode::Wallet.has_login() && !AuthMode::Wallet.uses_email());
    assert!(!AuthMode::None.has_login() && !AuthMode::None.uses_email());
}

#[test]
fn config_toml_beats_manifest_for_brain_mode() {
    let env = MapEnv::default();
    let file = ConfigFile {
        brain_mode: Some("sidecar".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &manifest_with_brain("hosted")).unwrap();
    assert_eq!(cfg.brain_mode, BrainMode::Sidecar);
    assert_eq!(prov.layer("brain_mode"), Some(ConfigLayer::ConfigToml));
}

#[test]
fn manifest_supplies_brain_mode_when_env_and_toml_absent() {
    let env = MapEnv::default();
    let (cfg, prov) = resolve(&env, None, &manifest_with_brain("sidecar")).unwrap();
    assert_eq!(cfg.brain_mode, BrainMode::Sidecar);
    assert_eq!(prov.layer("brain_mode"), Some(ConfigLayer::Manifest));
}

#[test]
fn credential_from_env_enables_cycles() {
    let env = MapEnv::new([("TINYHUMANS_API_KEY", "th_live_abc123")]);
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();

    assert!(cfg.tinyhumans_credential.is_some());
    assert!(cfg.cycles_available());
    assert_eq!(prov.layer("tinyhumans_credential"), Some(ConfigLayer::Env));
}

/// The hosted path: no static key at all, just a platform-projected token
/// file. Cycles must still be available — the instance can *obtain* a token —
/// and the source reads `attested`.
#[test]
fn projected_token_file_alone_enables_cycles() {
    // The path must actually exist: the projected tier is selected on
    // existence, not on the variable merely being set.
    let dir = tempfile::Builder::new()
        .prefix("oc-cfg-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-token").unwrap();

    let env = MapEnv::new([(
        crate::company::credentials::TOKEN_FILE_ENV,
        path.to_str().unwrap(),
    )]);
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();

    assert!(cfg.tinyhumans_credential.is_none(), "no static secret held");
    assert_eq!(cfg.tinyhumans_token_file.as_deref(), Some(path.as_path()));
    assert!(cfg.credential_available());
    assert!(cfg.cycles_available());
    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::Attested
    );
    assert_eq!(prov.layer("tinyhumans_token_file"), Some(ConfigLayer::Env));
}

/// The docker case: a leftover `TINYHUMANS_TOKEN_FILE` naming a path nothing
/// mounted must NOT report an identity this instance cannot present. Reporting
/// `attested` here would also make `cycles_available` true with no obtainable
/// bearer, so hosted cognition would be gated on a credential that does not
/// exist. Regression test for the config surface disagreeing with
/// `TinyhumansTokenSource::from_env`.
#[test]
fn a_token_file_that_does_not_exist_is_not_attested() {
    // A real directory, but the token path inside it is never created: the
    // fixture must name something that does not exist.
    let dir = tempfile::Builder::new()
        .prefix("oc-absent-")
        .tempdir()
        .expect("tempdir");
    let missing = dir.path().join("token");
    assert!(!missing.exists(), "fixture path must not exist");

    let env = MapEnv::new([(
        crate::company::credentials::TOKEN_FILE_ENV,
        missing.to_str().unwrap(),
    )]);
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();

    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::None,
        "an unmounted path must not read as attested"
    );
    assert!(!cfg.credential_available());
    assert!(!cfg.cycles_available());
}

/// Same unmounted path, but a static key is present: the source degrades to
/// the static tier rather than to `none`, matching `from_env`'s fallback.
#[test]
fn a_missing_token_file_degrades_to_the_static_tier() {
    // A real directory, but the token path inside it is never created: the
    // fixture must name something that does not exist.
    let dir = tempfile::Builder::new()
        .prefix("oc-absent-")
        .tempdir()
        .expect("tempdir");
    let missing = dir.path().join("token");
    let env = MapEnv::new([
        (
            crate::company::credentials::TOKEN_FILE_ENV,
            missing.to_str().unwrap(),
        ),
        (crate::company::credentials::API_KEY_ENV, "th_static"),
    ]);
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();

    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::Static
    );
    assert!(cfg.credential_available());
}

/// Precedence: a projected file that exists outranks a leftover static key.
#[test]
fn projected_file_outranks_a_static_key_for_the_source() {
    let dir = tempfile::Builder::new()
        .prefix("oc-cfg-")
        .tempdir()
        .expect("tempdir");
    let path = dir.path().join("token");
    std::fs::write(&path, "projected-token").unwrap();

    let env = MapEnv::new([
        (
            crate::company::credentials::TOKEN_FILE_ENV,
            path.to_str().unwrap(),
        ),
        (crate::company::credentials::API_KEY_ENV, "th_static"),
    ]);
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();
    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::Attested
    );

    // Docker development keeps working on the static tier alone.
    let static_only = MapEnv::new([(crate::company::credentials::API_KEY_ENV, "th_static")]);
    let (cfg, _) = resolve(&static_only, None, &default_manifest()).unwrap();
    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::Static
    );

    // Neither tier configured → nothing obtainable, no cycles.
    let (cfg, _) = resolve(&MapEnv::default(), None, &default_manifest()).unwrap();
    assert_eq!(
        cfg.credential_source(),
        crate::company::CredentialSource::None
    );
    assert!(!cfg.credential_available());
}

#[test]
fn public_url_and_tinyplace_url_resolve_by_precedence() {
    // public_url: env wins; tinyplace_api_url only in config.toml.
    let env = MapEnv::new([("OPENCOMPANY_PUBLIC_URL", "https://public.example")]);
    let file = ConfigFile {
        public_url: Some("https://toml.example".into()),
        tinyplace_api_url: Some("https://tp.toml".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &default_manifest()).unwrap();

    assert_eq!(cfg.public_url.as_deref(), Some("https://public.example"));
    assert_eq!(prov.layer("public_url"), Some(ConfigLayer::Env));
    assert_eq!(cfg.tinyplace_api_url, "https://tp.toml");
    assert_eq!(
        prov.layer("tinyplace_api_url"),
        Some(ConfigLayer::ConfigToml)
    );
}

#[test]
fn public_url_defaults_to_none() {
    let env = MapEnv::default();
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
    assert!(cfg.public_url.is_none());
    assert_eq!(prov.layer("public_url"), Some(ConfigLayer::Default));
}

#[test]
fn credential_from_config_toml_when_env_absent() {
    let env = MapEnv::default();
    let file = ConfigFile {
        tinyhumans_api_key: Some("th_from_toml".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &default_manifest()).unwrap();
    assert_eq!(
        cfg.tinyhumans_credential.as_ref().unwrap().expose(),
        "th_from_toml"
    );
    assert_eq!(
        prov.layer("tinyhumans_credential"),
        Some(ConfigLayer::ConfigToml)
    );
}

#[test]
fn debug_redacts_secrets() {
    let env = MapEnv::new([
        ("TINYHUMANS_API_KEY", "th_super_secret_value"),
        ("GITHUB_TOKEN", "ghp_secret_token"),
    ]);
    let (cfg, _) = resolve(&env, None, &default_manifest()).unwrap();
    let rendered = format!("{cfg:?}");
    assert!(!rendered.contains("th_super_secret_value"));
    assert!(!rendered.contains("ghp_secret_token"));
    assert!(rendered.contains("set"));
}

#[test]
fn invalid_brain_mode_is_a_config_error() {
    let env = MapEnv::new([("OPENCOMPANY_BRAIN_MODE", "quantum")]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    assert_eq!(err.code(), "config_error");
    assert!(err.to_string().contains("quantum"));
}

#[test]
fn empty_env_value_is_ignored() {
    let env = MapEnv::new([("OPENCOMPANY_BIND", "")]);
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
    assert_eq!(cfg.bind, DEFAULT_BIND);
    assert_eq!(prov.layer("bind"), Some(ConfigLayer::Default));
}

// -----------------------------------------------------------------
// resolve_base_url (AD-001 / AD-014): a hosted tenant is handed its
// whole environment by the platform that provisions it, so an unset
// api_url/tinyplace_api_url must refuse to boot instead of silently
// becoming production. Every other deployment kind is unaffected — the
// pinned decision in `defaults_fill_in_when_nothing_set` above still
// holds for an undeclared (self-hosted) process.
// -----------------------------------------------------------------

fn hosted_tenant_env<const N: usize>(pairs: [(&str, &str); N]) -> MapEnv {
    let mut all = vec![("OPENCOMPANY_DEPLOYMENT", "hosted-tenant")];
    all.extend(pairs);
    MapEnv::new(all)
}

#[test]
fn hosted_tenant_refuses_to_boot_with_no_api_url() {
    let env = hosted_tenant_env([]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    assert_eq!(err.code(), "config_error");
    let message = err.to_string();
    assert!(message.contains("TINYHUMANS_API_URL"), "{message}");
}

/// **The tiny.place hub is opt-in, so an unset URL is not a
/// misconfiguration.**
///
/// `maybe_build_economy` returns `None` before reading this unless the
/// manifest sets `place.discoverable` and names a handle — `discoverable`
/// defaults to false and no shipped company turns it on — and on the path
/// that does read it, it takes this same default when handed `None`.
///
/// Refusing here stopped every hosted tenant from booting over a URL it
/// would never have built a client for. `api_url` keeps its refusal: that
/// backend is reached unconditionally, so a silent production default
/// there is a destination nobody chose.
#[test]
fn hosted_tenant_defaults_tinyplace_api_url_rather_than_refusing() {
    // api_url set so the resolve under test turns on tinyplace_api_url
    // alone, not the sibling field checked above.
    let env = hosted_tenant_env([("TINYHUMANS_API_URL", "https://api.tinyhumans.ai")]);
    let (cfg, prov) = resolve(&env, None, &default_manifest())
        .expect("an unset tiny.place URL is not a boot failure");
    assert_eq!(cfg.tinyplace_api_url, DEFAULT_TINYPLACE_API_URL);
    assert_eq!(
        prov.layer("tinyplace_api_url"),
        Some(ConfigLayer::Default),
        "and it is recorded as the default it is"
    );
}

/// The sibling still refuses, so this change narrowed the rule rather
/// than removing it.
#[test]
fn hosted_tenant_still_refuses_a_missing_api_url_with_tinyplace_set() {
    let env = hosted_tenant_env([("TINYPLACE_API_URL", "https://staging-api.tiny.place")]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    assert_eq!(err.code(), "config_error");
    let message = err.to_string();
    assert!(message.contains("TINYHUMANS_API_URL"), "{message}");
}

#[test]
fn hosted_tenant_treats_an_empty_api_url_as_unset() {
    let env = hosted_tenant_env([
        ("TINYHUMANS_API_URL", "   "),
        ("TINYPLACE_API_URL", "https://api.tiny.place"),
    ]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    assert!(err.to_string().contains("TINYHUMANS_API_URL"));
}

#[test]
fn hosted_tenant_uses_an_explicitly_set_api_url_unchanged() {
    let env = hosted_tenant_env([
        ("TINYHUMANS_API_URL", "https://staging-api.tinyhumans.ai"),
        ("TINYPLACE_API_URL", "https://staging-api.tiny.place"),
    ]);
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
    assert_eq!(cfg.api_url, "https://staging-api.tinyhumans.ai");
    assert_eq!(prov.layer("api_url"), Some(ConfigLayer::Env));
    assert_eq!(cfg.tinyplace_api_url, "https://staging-api.tiny.place");
    assert_eq!(prov.layer("tinyplace_api_url"), Some(ConfigLayer::Env));
}

/// A hosted tenant may also state the URL in `config.toml` rather than
/// the environment — the gate is "was it stated", not "which layer".
#[test]
fn hosted_tenant_accepts_api_url_from_config_toml() {
    let env = hosted_tenant_env([]);
    let file = ConfigFile {
        api_url: Some("https://staging-api.tinyhumans.ai".into()),
        tinyplace_api_url: Some("https://staging-api.tiny.place".into()),
        ..ConfigFile::default()
    };
    let (cfg, prov) = resolve(&env, Some(&file), &default_manifest()).unwrap();
    assert_eq!(cfg.api_url, "https://staging-api.tinyhumans.ai");
    assert_eq!(prov.layer("api_url"), Some(ConfigLayer::ConfigToml));
}

/// The other side of the split: self-hosted and desktop deployments keep
/// defaulting. Forcing every plain `serve` to name a backend it never
/// had to before would break the documented zero-config quickstart for a
/// deployment kind that owns the choice by construction.
#[test]
fn self_hosted_and_desktop_still_default_api_url_when_unset() {
    for kind in ["self-hosted", ""] {
        let env = MapEnv::new([("OPENCOMPANY_DEPLOYMENT", kind)]);
        let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
        assert_eq!(cfg.api_url, DEFAULT_API_URL, "deployment={kind:?}");
        assert_eq!(prov.layer("api_url"), Some(ConfigLayer::Default));
    }

    let env = MapEnv::new([("OPENCOMPANY_DEPLOYMENT", "desktop")]);
    let (cfg, prov) = resolve(&env, None, &default_manifest()).unwrap();
    assert_eq!(cfg.api_url, DEFAULT_API_URL);
    assert_eq!(cfg.tinyplace_api_url, DEFAULT_TINYPLACE_API_URL);
    assert_eq!(prov.layer("api_url"), Some(ConfigLayer::Default));
}

/// The tenant-namespace inference (`OPENCOMPANY_TENANT_ID` alone, no
/// explicit `OPENCOMPANY_DEPLOYMENT`) names a hosted tenant too — see
/// `Deployment::from_env` — so it must gate the same as an explicit
/// declaration rather than being read as self-hosted.
#[test]
fn tenant_namespace_alone_also_gates_as_hosted_tenant() {
    let env = MapEnv::new([("OPENCOMPANY_TENANT_ID", "acme")]);
    let err = resolve(&env, None, &default_manifest()).unwrap_err();
    assert!(err.to_string().contains("TINYHUMANS_API_URL"));
}

// -----------------------------------------------------------------
// resolve_serve_bind: the layers `serve` actually honours.
//
// Before issue #425 `serve` read only its `--bind` flag, so
// `OPENCOMPANY_BIND` moved `doctor`'s report but never the listener.
// `serve_bind_env_beats_config_toml` is the regression test for exactly
// that: it is red against the flag-only behaviour.
// -----------------------------------------------------------------

#[test]
fn serve_bind_flag_beats_env_and_config_toml() {
    let env = MapEnv::new([("OPENCOMPANY_BIND", "127.0.0.1:2222")]);
    let (bind, source) = resolve_serve_bind(
        Some("127.0.0.1:1111".into()),
        &env,
        Some("127.0.0.1:3333".into()),
    );
    assert_eq!(bind, "127.0.0.1:1111");
    assert_eq!(source, "--bind");
}

#[test]
fn serve_bind_env_beats_config_toml() {
    let env = MapEnv::new([("OPENCOMPANY_BIND", "127.0.0.1:2222")]);
    let (bind, source) = resolve_serve_bind(None, &env, Some("127.0.0.1:3333".into()));
    assert_eq!(bind, "127.0.0.1:2222");
    assert_eq!(source, "OPENCOMPANY_BIND");
}

#[test]
fn serve_bind_empty_env_falls_through() {
    // Same empty-is-unset convention `empty_env_value_is_ignored` pins for
    // the `resolve` chain: an exported-but-blank variable must not shadow
    // the layer beneath it.
    let env = MapEnv::new([("OPENCOMPANY_BIND", "")]);
    let (bind, source) = resolve_serve_bind(None, &env, Some("127.0.0.1:3333".into()));
    assert_eq!(bind, "127.0.0.1:3333");
    assert_eq!(source, "config.toml");

    // With nothing under it either, an empty variable reaches the default.
    let (bind, source) = resolve_serve_bind(None, &env, None);
    assert_eq!(bind, DEFAULT_BIND);
    assert_eq!(source, "default");
}

#[test]
fn serve_bind_config_toml_used_when_no_flag_or_env() {
    let env = MapEnv::default();
    let (bind, source) = resolve_serve_bind(None, &env, Some("127.0.0.1:3333".into()));
    assert_eq!(bind, "127.0.0.1:3333");
    assert_eq!(source, "config.toml");
}

#[test]
fn serve_bind_defaults_to_loopback_when_nothing_set() {
    let env = MapEnv::default();
    let (bind, source) = resolve_serve_bind(None, &env, None);
    assert_eq!(bind, DEFAULT_BIND);
    assert_eq!(source, "default");
    // The default must stay loopback: a wildcard bind is only ever reached
    // by explicit operator intent (flag, variable, or config entry).
    assert!(
        bind.starts_with("127.0.0.1:"),
        "default bind must be loopback"
    );
}

#[test]
fn serve_bind_honours_a_wildcard_only_from_an_explicit_layer() {
    // The hosted manager injects `OPENCOMPANY_BIND=0.0.0.0:8080`; that must
    // reach the listener, and be attributed to the variable.
    let env = MapEnv::new([("OPENCOMPANY_BIND", "0.0.0.0:8080")]);
    let (bind, source) = resolve_serve_bind(None, &env, None);
    assert_eq!(bind, "0.0.0.0:8080");
    assert_eq!(source, "OPENCOMPANY_BIND");
}

#[test]
fn config_file_load_returns_none_when_absent() {
    let dir = std::env::temp_dir().join(format!("oc-cfg-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(ConfigFile::load(&dir).unwrap().is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_file_load_parses_toml() {
    let dir = std::env::temp_dir().join(format!("oc-cfg-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(CONFIG_FILE),
        "brain_mode = \"sidecar\"\napi_url = \"https://x\"\n",
    )
    .unwrap();
    let file = ConfigFile::load(&dir).unwrap().unwrap();
    assert_eq!(file.brain_mode.as_deref(), Some("sidecar"));
    assert_eq!(file.api_url.as_deref(), Some("https://x"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn workspace_section_defaults_to_clearing_tmp() {
    // Absent `[workspace]` → default (clear on startup).
    assert!(WorkspaceSection::default().resolve().clear_tmp_on_startup);
    // An explicit opt-out is honored.
    let section = WorkspaceSection {
        clear_tmp_on_startup: Some(false),
        ..WorkspaceSection::default()
    };
    assert!(!section.resolve().clear_tmp_on_startup);
}

#[test]
fn workspace_section_parses_quotas() {
    let section = WorkspaceSection {
        storage_quota_gb: Some(2.0),
        tmp_quota_gb: Some(0.0), // non-positive → unlimited
        ..WorkspaceSection::default()
    };
    let cfg = section.resolve();
    assert_eq!(cfg.storage_quota_bytes, Some(2 * 1024 * 1024 * 1024));
    assert_eq!(cfg.tmp_quota_bytes, None);
    // Absent → unlimited.
    assert_eq!(
        WorkspaceSection::default().resolve().storage_quota_bytes,
        None
    );
}

#[test]
fn config_file_parses_workspace_section() {
    let dir = std::env::temp_dir().join(format!("oc-cfg-ws-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(CONFIG_FILE),
        "[workspace]\ngit_enabled = true\nclear_tmp_on_startup = false\n",
    )
    .unwrap();
    let file = ConfigFile::load(&dir).unwrap().unwrap();
    assert_eq!(file.workspace.clear_tmp_on_startup, Some(false));
    assert_eq!(file.workspace.git_enabled, Some(true));
    assert!(!file.workspace.resolve().clear_tmp_on_startup);
    assert!(file.workspace.resolve().git_enabled);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn malformed_config_file_is_a_config_error() {
    let dir = std::env::temp_dir().join(format!("oc-cfg-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(CONFIG_FILE), "not = = valid").unwrap();
    let err = ConfigFile::load(&dir).unwrap_err();
    assert_eq!(err.code(), "config_error");
    std::fs::remove_dir_all(&dir).ok();
}

// -----------------------------------------------------------------------
// write_config_toml
// -----------------------------------------------------------------------

fn write_dir(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("oc-write-{tag}-"))
        .tempdir()
        .expect("tempdir")
}

/// Writing into a data root that has no `config.toml` yet — the first-run
/// case — creates the file, and it parses back through the normal reader.
#[test]
fn writing_creates_the_file_when_absent() {
    let dir = write_dir("new");
    let path = write_config_toml(
        dir.path(),
        &[
            ("bind", ConfigValue::Str("0.0.0.0:9000".into())),
            ("auth_mode", ConfigValue::Str("none".into())),
        ],
    )
    .unwrap();

    assert_eq!(path, dir.path().join(CONFIG_FILE));
    let file = ConfigFile::load(dir.path()).unwrap().unwrap();
    assert_eq!(file.bind.as_deref(), Some("0.0.0.0:9000"));
    assert_eq!(file.auth_mode.as_deref(), Some("none"));
}

/// The reason this goes through `toml_edit` at all: the shipped file's
/// commented `[[default_mcp_server]]` PLACEHOLDER block is documentation an
/// operator is meant to read and uncomment, and serializing a `ConfigFile`
/// back out would delete it along with every other comment. Untouched keys
/// must survive too.
#[test]
fn writing_preserves_comments_and_untouched_keys() {
    let dir = write_dir("comments");
    std::fs::write(
        dir.path().join(CONFIG_FILE),
        "# The instance's bind address.\n\
         bind = \"127.0.0.1:8080\"\n\
         api_url = \"https://api.example.test\"\n\
         \n\
         # PLACEHOLDER — uncomment to ship a default tool server.\n\
         # [[default_mcp_server]]\n\
         # name = \"deepwiki\"\n",
    )
    .unwrap();

    write_config_toml(
        dir.path(),
        &[("bind", ConfigValue::Str("0.0.0.0:9000".into()))],
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
    assert!(text.contains("# The instance's bind address."));
    assert!(text.contains("# PLACEHOLDER — uncomment to ship a default tool server."));
    assert!(text.contains("# [[default_mcp_server]]"));
    assert!(text.contains("# name = \"deepwiki\""));
    assert!(
        text.contains("api_url = \"https://api.example.test\""),
        "an untouched key must survive verbatim"
    );
    assert!(text.contains("0.0.0.0:9000"), "the edit must land");
    assert!(
        !text.contains("127.0.0.1:8080"),
        "the old value must be replaced, not duplicated"
    );
}

/// A dotted key writes into `[workspace]`, creating the table when needed,
/// and the result resolves through `WorkspaceSection`.
#[test]
fn writing_reaches_into_the_workspace_table() {
    let dir = write_dir("ws");
    write_config_toml(
        dir.path(),
        &[
            ("workspace.clear_tmp_on_startup", ConfigValue::Bool(false)),
            ("workspace.max_blob_mb", ConfigValue::Float(64.0)),
        ],
    )
    .unwrap();

    let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
    assert!(
        text.contains("[workspace]"),
        "the table header must be explicit, not implicit: {text}"
    );

    let file = ConfigFile::load(dir.path()).unwrap().unwrap();
    assert_eq!(file.workspace.clear_tmp_on_startup, Some(false));
    assert_eq!(file.workspace.max_blob_mb, Some(64.0));
    assert!(!file.workspace.resolve().clear_tmp_on_startup);
}

/// `Unset` removes the key rather than writing `""`. The difference matters:
/// an absent key falls through to the next precedence layer, where a blank
/// string would be a set-but-empty value.
#[test]
fn unset_removes_the_key_so_the_next_layer_applies() {
    let dir = write_dir("unset");
    std::fs::write(
        dir.path().join(CONFIG_FILE),
        "auth_mode = \"wallet\"\nbind = \"0.0.0.0:9000\"\n",
    )
    .unwrap();

    write_config_toml(dir.path(), &[("auth_mode", ConfigValue::Unset)]).unwrap();

    let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
    assert!(!text.contains("auth_mode"), "the key must be gone: {text}");

    let file = ConfigFile::load(dir.path()).unwrap().unwrap();
    assert!(file.auth_mode.is_none());
    assert_eq!(file.bind.as_deref(), Some("0.0.0.0:9000"));

    // And with the key gone, resolution falls through to the manifest.
    let mut manifest = default_manifest();
    manifest.users.mode = "wallet".into();
    let (cfg, prov) = resolve(&MapEnv::default(), Some(&file), &manifest).unwrap();
    assert_eq!(cfg.auth_mode, AuthMode::Wallet);
    assert_eq!(prov.layer("auth_mode"), Some(ConfigLayer::Manifest));
}

/// Clearing a key out of a `[workspace]` table that does not exist must not
/// materialize an empty table just to delete nothing out of it.
#[test]
fn unset_does_not_materialize_a_missing_table() {
    let dir = write_dir("unset-ws");
    write_config_toml(dir.path(), &[("workspace.max_blob_mb", ConfigValue::Unset)]).unwrap();
    let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
    assert!(!text.contains("[workspace]"), "no empty table: {text}");
}

/// Merging into a document that could not be parsed would overwrite whatever
/// the operator actually had there, so a malformed file is refused — the
/// same contract `ConfigFile::load` holds.
#[test]
fn writing_refuses_a_malformed_existing_file() {
    let dir = write_dir("bad");
    std::fs::write(dir.path().join(CONFIG_FILE), "not = = valid").unwrap();

    let err = write_config_toml(
        dir.path(),
        &[("bind", ConfigValue::Str("0.0.0.0:9000".into()))],
    )
    .unwrap_err();
    assert_eq!(err.code(), "config_error");

    let text = std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
    assert_eq!(text, "not = = valid", "the original must be left alone");
}

/// The write is atomic via a same-directory temp file and `rename`. Nothing
/// may be left behind for the next boot (or the next write) to trip over —
/// checked by name pattern rather than the old fixed `config.toml.tmp`,
/// since the temp name is now made unique per call.
#[test]
fn writing_leaves_no_temp_file_behind() {
    let dir = write_dir("tmp");
    write_config_toml(
        dir.path(),
        &[("bind", ConfigValue::Str("0.0.0.0:9000".into()))],
    )
    .unwrap();
    let leftover: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect();
    assert!(leftover.is_empty(), "left behind: {leftover:?}");
}

/// A write that fails after the temp file's directory disappears out from
/// under it must not leave anything behind, and must report the failure
/// rather than panic — the failure half of
/// `writing_leaves_no_temp_file_behind` above, forced deterministically
/// (absent-parent, like `store::fs::durable_append_reports_an_unwritable_path`)
/// rather than by injecting a permission failure, which behaves
/// differently depending on whether the test runs as root.
#[test]
fn a_write_that_fails_leaves_no_temp_file_behind() {
    let dir = write_dir("write-fails");
    let root = dir.path().join("gone");
    // `existing` reads NotFound as "no config yet" and proceeds, so the
    // failure below comes from the temp file's own `std::fs::write`
    // rather than from the initial read.
    let err = write_config_toml(&root, &[("bind", ConfigValue::Str("0.0.0.0:9000".into()))])
        .unwrap_err();
    assert_eq!(err.code(), "config_error");
    assert!(
        !root.exists(),
        "a write into a missing directory must not create it or anything in it"
    );
}

/// Two writers racing the same directory must not clobber each other: each
/// call's edits land, and neither call's temp file collides with the
/// other's — the bug CodeRabbit flagged on #908 (`config.rs:549`).
#[test]
fn concurrent_writes_to_the_same_directory_do_not_clobber_each_other() {
    let dir = write_dir("concurrent");
    let path = dir.path().to_path_buf();

    let a = std::thread::spawn({
        let path = path.clone();
        move || {
            for i in 0..25 {
                write_config_toml(
                    &path,
                    &[("bind", ConfigValue::Str(format!("0.0.0.0:{}", 9000 + i)))],
                )
                .unwrap();
            }
        }
    });
    let b = std::thread::spawn({
        let path = path.clone();
        move || {
            for i in 0..25 {
                write_config_toml(
                    &path,
                    &[(
                        "public_url",
                        ConfigValue::Str(format!("https://h{i}.example")),
                    )],
                )
                .unwrap();
            }
        }
    });
    a.join().unwrap();
    b.join().unwrap();

    // Both threads' edits target different keys, so a clobbered write would
    // show up as one key or the other missing from the final file — not as
    // a torn/unparseable file, which `ConfigFile::load` would already catch.
    let file = ConfigFile::load(&path).unwrap().unwrap();
    assert!(file.bind.is_some(), "the bind writer's edits went missing");
    assert!(
        file.public_url.is_some(),
        "the public_url writer's edits went missing"
    );

    // No stray temp file from either racer.
    let leftover: Vec<_> = std::fs::read_dir(&path)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect();
    assert!(leftover.is_empty(), "left behind: {leftover:?}");
}

/// Setup completion is recorded in the file, so "has this instance been set
/// up" survives a new browser and travels with the data root.
#[test]
fn setup_completion_round_trips() {
    let dir = write_dir("done");
    assert!(
        ConfigFile::load(dir.path()).unwrap().is_none(),
        "a fresh data root has no config at all"
    );

    write_config_toml(
        dir.path(),
        &[("setup_completed_at", ConfigValue::Int(1_755_000_000_000))],
    )
    .unwrap();

    let file = ConfigFile::load(dir.path()).unwrap().unwrap();
    assert_eq!(file.setup_completed_at, Some(1_755_000_000_000));
}
