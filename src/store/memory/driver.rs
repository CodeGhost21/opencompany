//! Driver selection: configuration in, a bound [`MemoryProvider`] out.
//!
//! # Class is decided here, not by the driver
//!
//! [`DriverClass`] comes from the host's configuration and is cross-checked
//! against the reserved id table before anything opens. The contract crate
//! excludes class on purpose — a driver that self-reported it could claim to be
//! embedded and skip the egress and trust checks the class gates — so this
//! module never asks a provider what it is.
//!
//! # Why `embedded` does not go through this seam yet
//!
//! [`MemoryMode::Embedded`] deliberately resolves to the existing `EngineCortex`
//! overlay rather than to a provider built here, and that is a durability
//! decision rather than an unfinished edge.
//!
//! The obvious construction — `tinymemory_tinycortex::provider(…)` over a
//! `tinycortex::memory::Memory` backend — cannot currently be durable: the only
//! concrete `Memory` implementation in the vendored engine is
//! `InMemoryMemoryStore`, a `BTreeMap` behind an `RwLock`. Binding it would
//! swap today's per-company SQLite workspaces under `<data_dir>/memory/` for a
//! store that is empty after every restart, and it would do so *silently* — the
//! reads would all succeed, returning nothing.
//!
//! This deployment already refuses that class of failure out loud (see the
//! ephemeral-`/data` refusal in `crate::store::select`), so quietly introducing
//! it through a contract migration would be a strange thing to do. Moving the
//! embedded engine onto the provider seam needs a durable `Memory`
//! implementation over the engine's KV tier first; until that exists, `embedded`
//! keeps the durable path it has always had, and `remote` and `null` — which
//! have no incumbent to regress — bind providers here.

use std::sync::Arc;

use tinymemory::registry::{
    COGNEE_DRIVER_ID, ConfigLabels, DriverClass, DriverEntry, DriverRegistry, MEM0_DRIVER_ID,
    NULL_DRIVER_ID, SUPERMEMORY_DRIVER_ID, TRUSTED,
};
use tinymemory_api::null::NullMemoryProvider;
use tinymemory_api::provider::MemoryProvider;

use crate::Result;
use crate::error::OpenCompanyError;

/// Which engine backs memory, as an operator selects it.
///
/// The wire values of `OPENCOMPANY_MEMORY`, minus the legacy spellings that
/// `crate::store::select::MemoryBackend` maps onto these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryMode {
    /// The engine runs in-pod against `OPENCOMPANY_DATA_DIR`. No network call,
    /// works with a read-only root filesystem.
    Embedded,
    /// A hosted memory service behind a URL and a credential.
    Remote,
    /// Writes accepted and discarded, reads empty.
    ///
    /// `/dev/null` semantics, for a deployment that wants the ports wired and
    /// nothing retained. Never selected as a fallback when a configured driver
    /// fails to bind — a company that believes it is remembering and is not is
    /// the failure this whole surface exists to prevent.
    Null,
}

/// Everything needed to open a driver, already resolved from env + manifest.
///
/// Holds the credential, so it is not `Debug` — see the manual impl below.
#[derive(Clone)]
pub struct MemoryDriverConfig {
    /// The selected mode.
    pub mode: MemoryMode,
    /// The driver id (`supermemory`, `mem0`, `cognee`, `null`, …).
    ///
    /// `None` takes the mode's default: `null` for [`MemoryMode::Null`], and for
    /// [`MemoryMode::Remote`] there is no default — an unnamed remote engine is
    /// a refusal, because guessing which hosted service an operator meant is
    /// not a recoverable mistake.
    pub driver_id: Option<String>,
    /// Base URL of the hosted service. Required for [`MemoryMode::Remote`].
    pub url: Option<String>,
    /// The outbound credential. Required for [`MemoryMode::Remote`].
    pub api_key: Option<String>,
}

impl std::fmt::Debug for MemoryDriverConfig {
    /// Renders the mode and driver id; never the URL, never the credential.
    ///
    /// The URL is withheld alongside the key rather than treated as harmless:
    /// a self-hosted memory endpoint is internal topology, and this type is
    /// reachable from boot logging and error paths where a bare `{:?}` is one
    /// keystroke away.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryDriverConfig")
            .field("mode", &self.mode)
            .field("driver_id", &self.driver_id)
            .field("url", &self.url.as_ref().map(|_| "<set>"))
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .finish()
    }
}

/// A refusal to bind, phrased for the operator who has to fix it.
#[derive(Debug)]
pub struct MemoryDriverError(String);

impl std::fmt::Display for MemoryDriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<MemoryDriverError> for OpenCompanyError {
    fn from(error: MemoryDriverError) -> Self {
        Self::Config(error.0)
    }
}

/// The config-section names the registry echoes in its refusals.
fn labels() -> ConfigLabels<'static> {
    ConfigLabels {
        section: "OPENCOMPANY_MEMORY",
        drivers: "[memory]",
        driver_entry: "[memory]",
    }
}

/// Opens the configured driver, returning it with the class the *host* assigned.
///
/// `Ok(None)` means [`MemoryMode::Embedded`]: the caller keeps the existing
/// `EngineCortex` overlay rather than binding a provider here. See the module
/// docs for why that is a durability decision and not an omission.
///
/// # Errors
///
/// Every failure names the knob to change. A missing credential is a refusal,
/// never a silent downgrade to the embedded engine: a company that thinks it is
/// writing to its hosted memory and is not is worse off than one that fails to
/// start, because the first failure is invisible until the memory is needed.
pub fn open_driver(
    config: &MemoryDriverConfig,
) -> Result<Option<(Arc<dyn MemoryProvider>, DriverClass)>> {
    let bound: (Arc<dyn MemoryProvider>, DriverClass) = match config.mode {
        MemoryMode::Embedded => return Ok(None),
        MemoryMode::Null => {
            let admission = admit(NULL_DRIVER_ID, DriverClass::Null)?;
            (
                Arc::new(NullMemoryProvider::new()) as Arc<dyn MemoryProvider>,
                admission,
            )
        }
        MemoryMode::Remote => {
            let driver_id = config
                .driver_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    MemoryDriverError(format!(
                        "OPENCOMPANY_MEMORY=remote requires OPENCOMPANY_MEMORY_DRIVER (or \
                         [memory].provider) naming the hosted engine — one of {}. There is no \
                         default: binding the wrong hosted engine writes a company's memory \
                         somewhere it cannot be read back from.",
                        SUPPORTED_REMOTE_DRIVERS.join(", ")
                    ))
                })?;
            let url = require(
                config.url.as_deref(),
                "OPENCOMPANY_MEMORY=remote requires OPENCOMPANY_MEMORY_URL (or [memory].base_url) \
                 naming the hosted engine's endpoint",
            )?;
            let key = require(
                config.api_key.as_deref(),
                "OPENCOMPANY_MEMORY=remote requires a credential: set OPENCOMPANY_MEMORY_API_KEY, \
                 or name a SecretStore key with [memory].api_key_secret",
            )?;
            let class = admit(driver_id, DriverClass::External)?;
            (remote_provider(driver_id, url, key)?, class)
        }
    };
    audit_capabilities(bound.0.as_ref())?;
    Ok(Some(bound))
}

/// Refuses a driver that over-claims what it implements, and reports one that
/// under-claims.
///
/// `capabilities()` is a hand-written claim; `provides()` is derived from the
/// accessors and cannot drift. Comparing them is the contract's own honesty
/// check, and `tinymemory_api::provider::audit` says explicitly to run it "at
/// bind time" — which nothing here was doing.
///
/// The two directions are not the same failure, so they are not treated the
/// same:
///
/// - **Advertised but absent** refuses the bind. The host registers RPC methods
///   and assembles agent tools from the *claim* and never re-checks, so an
///   over-claim becomes a surface that exists, is offered to an agent, and fails
///   on first call — inside a tenant, at the moment the memory is needed.
/// - **Present but unadvertised** only warns. The family works; nothing routes
///   to it, because routing follows the claim. Upstream calls that dead surface
///   from a forgotten `capabilities()` entry. Refusing a boot over it would turn
///   an upstream oversight into a tenant outage, which is a worse trade than
///   running with one family unreachable.
///
/// Structurally neither should fire: every adapter reachable from here is
/// composed through `MemoryTraitProvider`, which derives the advertisement from
/// the accessors. It runs anyway because that guarantee lives upstream, in a
/// submodule this repo pins by gitlink, and a gitlink bump is exactly when it
/// would quietly stop holding.
fn audit_capabilities(provider: &dyn MemoryProvider) -> Result<()> {
    let Err(mismatch) = tinymemory_api::provider::audit_provider(provider) else {
        return Ok(());
    };
    if !mismatch.present_but_unadvertised.is_empty() {
        tracing::warn!(
            driver_id = provider.driver_id(),
            families = %families(&mismatch.present_but_unadvertised),
            "the bound memory driver implements capability families it does not advertise; they \
             are unreachable, because the host routes from the advertised set",
        );
    }
    if !mismatch.advertised_but_absent.is_empty() {
        return Err(MemoryDriverError(format!(
            "the memory driver `{}` advertises capability families it does not implement: {}. \
             Every one of those becomes an agent tool that fails on first call, so the bind is \
             refused here rather than left to surface mid-cycle. This is an adapter bug rather \
             than a configuration mistake — no environment variable lifts it.",
            provider.driver_id(),
            families(&mismatch.advertised_but_absent)
        ))
        .into());
    }
    Ok(())
}

/// Formats capability families for an operator-facing message.
fn families(families: &[tinymemory_api::capabilities::Capability]) -> String {
    families
        .iter()
        .map(|family| family.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns `value`, or the refusal text when it is absent or blank.
fn require<'a>(value: Option<&'a str>, refusal: &str) -> Result<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MemoryDriverError(refusal.to_string()).into())
}

/// Runs the driver id through the registry and pins the class host-side.
///
/// Two checks, both of which have to pass:
///
/// 1. The registry's own admission, which is what refuses an external driver
///    whose trust has not been explicitly raised.
/// 2. That the class the registry resolved matches the class this *mode*
///    implies. `OPENCOMPANY_MEMORY=remote` naming `tinycortex` is a
///    configuration mistake with a security shape — it would run an engine
///    under the checks meant for the other class — so it is refused rather than
///    quietly resolved in the registry's favour.
fn admit(driver_id: &str, expected: DriverClass) -> Result<DriverClass> {
    let registry = DriverRegistry::builtin();
    // Trust is asserted by the host for a driver the host itself selected from
    // its own configuration: reaching this line already means the operator named
    // the engine and supplied its endpoint and credential. The registry's
    // fail-closed trust gate exists for config files that name a driver without
    // meaning to enable it, which is not a state this path can be in.
    let entry = DriverEntry {
        class: Some(expected.as_str()),
        trust_state: TRUSTED,
    };
    let admission = registry
        .admit(driver_id, Some(entry), labels())
        .map_err(|reason| {
            MemoryDriverError(format!(
                "memory driver `{}` was refused: {}",
                reason.configured_driver, reason.reason
            ))
        })?;
    if admission.class != expected {
        return Err(MemoryDriverError(format!(
            "driver `{driver_id}` is class `{}`, but OPENCOMPANY_MEMORY selected a mode that \
             requires class `{}`. Pick a driver matching the mode, or change the mode.",
            admission.class.as_str(),
            expected.as_str()
        ))
        .into());
    }
    Ok(admission.class)
}

/// Builds the HTTP provider for one hosted engine.
///
/// Capability honesty is the adapters' own: each is composed through
/// `MemoryTraitProvider`, which advertises exactly Core + Recall + Portability
/// and leaves every optional accessor `None`. That is the truth about a hosted
/// service — no summary tree, no graph, no taint tier — and it is what makes
/// `audit_provider` pass at bind rather than a call fail later.
fn remote_provider(driver_id: &str, url: &str, key: &str) -> Result<Arc<dyn MemoryProvider>> {
    let provider: Arc<dyn MemoryProvider> = match driver_id {
        SUPERMEMORY_DRIVER_ID => Arc::new(tinymemory_remote::supermemory_provider(
            tinymemory_remote::SupermemoryMemory::new(url, Some(key)).map_err(open_failed)?,
        )),
        MEM0_DRIVER_ID => Arc::new(tinymemory_remote::mem0_provider(
            tinymemory_remote::Mem0Memory::new(url, Some(key)).map_err(open_failed)?,
        )),
        COGNEE_DRIVER_ID => Arc::new(tinymemory_remote::cognee_provider(
            tinymemory_remote::CogneeMemory::new(url, Some(key)).map_err(open_failed)?,
        )),
        // Unreachable in practice: `admit` has already rejected any id the
        // registry does not reserve as External. Kept as a refusal rather than
        // an `unreachable!` so adding a reserved id upstream surfaces here as a
        // clear boot message instead of a panic in a tenant container.
        other => {
            return Err(MemoryDriverError(format!(
                "no HTTP adapter is compiled in for memory driver `{other}`"
            ))
            .into());
        }
    };
    Ok(provider)
}

/// Renders an adapter construction failure without echoing the endpoint.
///
/// The adapters validate the URL at construction, so this is usually a
/// malformed `OPENCOMPANY_MEMORY_URL`. The error text is the adapter's own and
/// is documented not to carry the credential; the endpoint is withheld here for
/// the same reason [`MemoryDriverConfig`]'s `Debug` withholds it.
fn open_failed(error: anyhow::Error) -> OpenCompanyError {
    OpenCompanyError::Config(format!(
        "could not open the configured memory engine: {error}. \
         Check OPENCOMPANY_MEMORY_URL (or [memory].base_url)."
    ))
}

/// The driver ids this build can actually construct, for error text and docs.
pub const SUPPORTED_REMOTE_DRIVERS: [&str; 3] =
    [SUPERMEMORY_DRIVER_ID, MEM0_DRIVER_ID, COGNEE_DRIVER_ID];

#[cfg(test)]
mod test {
    use super::*;
    use tinymemory::registry::TINYCORTEX_DRIVER_ID;

    fn config(mode: MemoryMode) -> MemoryDriverConfig {
        MemoryDriverConfig {
            mode,
            driver_id: None,
            url: None,
            api_key: None,
        }
    }

    /// A driver that claims a family it does not implement.
    ///
    /// Delegates every mandatory method to the null driver and changes exactly
    /// one thing: `capabilities()` adds `Graph`, while `as_graph()` keeps the
    /// contract's `None` default. That is the shape `audit_capabilities` exists
    /// to catch — an adapter whose hand-written claim outran its accessors.
    struct OverClaimer(NullMemoryProvider);

    #[async_trait::async_trait]
    impl tinymemory_api::provider::MemoryCore for OverClaimer {
        async fn store(
            &self,
            namespace: &str,
            key: &str,
            content: &str,
            category: tinymemory_api::types::MemoryCategory,
            session_id: Option<&str>,
            taint: tinymemory_api::types::MemoryTaint,
        ) -> std::result::Result<(), tinymemory_api::error::MemoryError> {
            self.0
                .store(namespace, key, content, category, session_id, taint)
                .await
        }
        async fn get(
            &self,
            namespace: &str,
            key: &str,
        ) -> std::result::Result<
            Option<tinymemory_api::types::MemoryEntry>,
            tinymemory_api::error::MemoryError,
        > {
            self.0.get(namespace, key).await
        }
        async fn forget(
            &self,
            namespace: &str,
            key: &str,
        ) -> std::result::Result<bool, tinymemory_api::error::MemoryError> {
            self.0.forget(namespace, key).await
        }
        async fn list(
            &self,
            namespace: Option<&str>,
            category: Option<&tinymemory_api::types::MemoryCategory>,
            session_id: Option<&str>,
        ) -> std::result::Result<
            Vec<tinymemory_api::types::MemoryEntry>,
            tinymemory_api::error::MemoryError,
        > {
            self.0.list(namespace, category, session_id).await
        }
        async fn namespaces(
            &self,
        ) -> std::result::Result<
            Vec<tinymemory_api::types::NamespaceSummary>,
            tinymemory_api::error::MemoryError,
        > {
            self.0.namespaces().await
        }
    }

    #[async_trait::async_trait]
    impl tinymemory_api::provider::MemoryRecall for OverClaimer {
        async fn recall(
            &self,
            query: &str,
            limit: usize,
            opts: &tinymemory_api::recall::OwnedRecallOpts,
            scope: Option<&tinymemory_api::provider::SourceScope>,
        ) -> std::result::Result<
            Vec<tinymemory_api::types::MemoryEntry>,
            tinymemory_api::error::MemoryError,
        > {
            self.0.recall(query, limit, opts, scope).await
        }
    }

    #[async_trait::async_trait]
    impl tinymemory_api::provider::MemoryPortability for OverClaimer {
        async fn export_page(
            &self,
            cursor: Option<&str>,
            limit: usize,
        ) -> std::result::Result<
            tinymemory_api::provider::ExportPage,
            tinymemory_api::error::MemoryError,
        > {
            self.0.export_page(cursor, limit).await
        }
        async fn import_records(
            &self,
            records: Vec<tinymemory_api::provider::ExportRecord>,
        ) -> std::result::Result<
            tinymemory_api::provider::ImportOutcome,
            tinymemory_api::error::MemoryError,
        > {
            self.0.import_records(records).await
        }
    }

    #[async_trait::async_trait]
    impl MemoryProvider for OverClaimer {
        fn driver_id(&self) -> &str {
            "over-claimer"
        }
        fn capabilities(&self) -> tinymemory_api::capabilities::Capabilities {
            let mut claimed = self.0.capabilities();
            claimed.insert(tinymemory_api::capabilities::Capability::Graph);
            claimed
        }
        async fn health(&self) -> tinymemory_api::health::MemoryHealth {
            self.0.health().await
        }
    }

    #[test]
    fn an_over_claiming_driver_is_refused_and_names_the_family() {
        let error = audit_capabilities(&OverClaimer(NullMemoryProvider::new()))
            .expect_err("a driver advertising Graph without implementing it must be refused")
            .to_string();
        assert!(error.contains("over-claimer"), "{error}");
        assert!(error.contains("graph"), "{error}");
    }

    #[test]
    fn an_honest_driver_passes_the_audit() {
        // The other half of the gate: it must not refuse the drivers we ship.
        audit_capabilities(&NullMemoryProvider::new()).expect("the null driver is honest");
    }

    #[test]
    fn embedded_keeps_the_existing_overlay() {
        assert!(
            open_driver(&config(MemoryMode::Embedded))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn null_binds_and_is_class_null() {
        let (provider, class) = open_driver(&config(MemoryMode::Null)).unwrap().unwrap();
        assert_eq!(class, DriverClass::Null);
        assert_eq!(provider.driver_id(), NULL_DRIVER_ID);
    }

    #[test]
    fn the_null_driver_passes_its_capability_audit() {
        let (provider, _) = open_driver(&config(MemoryMode::Null)).unwrap().unwrap();
        assert!(tinymemory_api::provider::audit_provider(provider.as_ref()).is_ok());
    }

    #[test]
    fn remote_without_a_driver_id_refuses_and_names_the_knob() {
        let mut cfg = config(MemoryMode::Remote);
        cfg.url = Some("https://memory.example".into());
        cfg.api_key = Some("k".into());
        let error = open_driver(&cfg).err().unwrap().to_string();
        assert!(error.contains("OPENCOMPANY_MEMORY_DRIVER"), "{error}");
    }

    #[test]
    fn remote_without_a_url_refuses_and_names_the_knob() {
        let mut cfg = config(MemoryMode::Remote);
        cfg.driver_id = Some(SUPERMEMORY_DRIVER_ID.into());
        cfg.api_key = Some("k".into());
        let error = open_driver(&cfg).err().unwrap().to_string();
        assert!(error.contains("OPENCOMPANY_MEMORY_URL"), "{error}");
    }

    #[test]
    fn remote_without_a_credential_refuses_and_names_both_ways_to_supply_one() {
        let mut cfg = config(MemoryMode::Remote);
        cfg.driver_id = Some(SUPERMEMORY_DRIVER_ID.into());
        cfg.url = Some("https://memory.example".into());
        let error = open_driver(&cfg).err().unwrap().to_string();
        assert!(error.contains("OPENCOMPANY_MEMORY_API_KEY"), "{error}");
        assert!(error.contains("api_key_secret"), "{error}");
    }

    #[test]
    fn a_blank_credential_is_treated_as_missing() {
        // An env var set to the empty string is the shape a broken deployment
        // template produces, and it must not read as "configured".
        let mut cfg = config(MemoryMode::Remote);
        cfg.driver_id = Some(SUPERMEMORY_DRIVER_ID.into());
        cfg.url = Some("https://memory.example".into());
        cfg.api_key = Some("   ".into());
        assert!(open_driver(&cfg).is_err());
    }

    #[test]
    fn remote_refuses_an_embedded_driver_id() {
        // Class is host-side: naming the embedded engine under the remote mode
        // would run it under the wrong checks.
        let mut cfg = config(MemoryMode::Remote);
        cfg.driver_id = Some(TINYCORTEX_DRIVER_ID.into());
        cfg.url = Some("https://memory.example".into());
        cfg.api_key = Some("k".into());
        let error = open_driver(&cfg).err().unwrap().to_string();
        assert!(error.contains("class"), "{error}");
    }

    #[test]
    fn remote_refuses_an_unknown_driver_id() {
        let mut cfg = config(MemoryMode::Remote);
        cfg.driver_id = Some("definitely-not-an-engine".into());
        cfg.url = Some("https://memory.example".into());
        cfg.api_key = Some("k".into());
        assert!(open_driver(&cfg).is_err());
    }

    #[test]
    fn every_supported_driver_id_binds() {
        for id in SUPPORTED_REMOTE_DRIVERS {
            let mut cfg = config(MemoryMode::Remote);
            cfg.driver_id = Some(id.to_string());
            cfg.url = Some("https://memory.example".into());
            cfg.api_key = Some("k".into());
            let (provider, class) = open_driver(&cfg)
                .unwrap_or_else(|error| panic!("{id} did not bind: {error}"))
                .unwrap();
            assert_eq!(class, DriverClass::External, "{id}");
            assert_eq!(provider.driver_id(), id);
            assert!(
                tinymemory_api::provider::audit_provider(provider.as_ref()).is_ok(),
                "{id} failed its capability audit"
            );
        }
    }

    #[test]
    fn debug_never_renders_the_credential_or_the_endpoint() {
        let cfg = MemoryDriverConfig {
            mode: MemoryMode::Remote,
            driver_id: Some(SUPERMEMORY_DRIVER_ID.into()),
            url: Some("https://memory.internal.example".into()),
            api_key: Some("sk-super-secret-value".into()),
        };
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("sk-super-secret-value"), "{rendered}");
        assert!(!rendered.contains("memory.internal.example"), "{rendered}");
    }
}
