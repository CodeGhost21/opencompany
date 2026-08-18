//! Config-driven storage backend selection.
//!
//! The five storage ports are the entire persistence contract; this module is
//! the one place that maps a backend *name* onto concrete port
//! implementations. `serve` (and platform provisioning) resolve a
//! [`StorageKind`] from `OPENCOMPANY_STORAGE`, open the backend once, and
//! inject the same [`StorageHandles`] into every company's `RuntimeBuilder` —
//! the kernel itself never names an engine.
//!
//! Backends behind disabled cargo features fail loudly at open time rather
//! than silently falling back to the filesystem.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;
use crate::error::OpenCompanyError;
use crate::ports::artifacts::ArtifactStore;
use crate::ports::context::ContextStore;
use crate::ports::events::EventLog;
use crate::ports::facts::FactStore;
use crate::ports::inbox::InboxStore;
use crate::ports::journal::JournalStore;
use crate::ports::ledgers::LedgerStore;
use crate::ports::login_codes::LoginCodeStore;
use crate::ports::memory::MemoryStore;
use crate::ports::notifications::NotificationStore;
use crate::ports::read_state::ReadStateStore;
use crate::ports::run_output::WorkflowRunOutputStore;
use crate::ports::runs::RunStore;
use crate::ports::schedule_fires::ScheduleFireStore;
use crate::ports::secrets::SecretStore;
use crate::ports::sessions::SessionStore;
use crate::ports::skills_state::SkillStateStore;
use crate::ports::store::CompanyStore;
use crate::ports::tasks::TaskStore;
use crate::ports::types::CompanyId;
use crate::ports::usage::UsageMeter;
use crate::ports::users::UserStore;
use crate::ports::workflow_revisions::WorkflowRevisionStore;
use crate::ports::workspace::WorkspaceStore;

/// Which storage backend hosts the durable ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StorageKind {
    /// Per-company filesystem bundles (the default; no external service).
    #[default]
    Fs,
    /// One SQLite database file under the data dir (`sqlite` feature).
    Sqlite,
    /// A MongoDB database on a shared cluster (`mongodb` feature) — the
    /// multi-tenant platform backend.
    Mongodb,
}

impl StorageKind {
    /// The backend's name, for `/spec`. Stable wire strings — a client keys
    /// behaviour off these, so they are not `Debug` output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fs => "fs",
            Self::Sqlite => "sqlite",
            Self::Mongodb => "mongodb",
        }
    }

    /// Whether this backend keeps [`SecretStore`] material as **plaintext on
    /// the container's own filesystem** (issue #752).
    ///
    /// `fs` writes one plaintext file per secret under
    /// `<data-dir>/companies/<slug>/secrets/` — [`FsSecretStore`] says so in its
    /// own doc comment, and `sqlite` puts the same bytes in a database file on
    /// the same disk. `mongodb` is the only backend that keeps them out of the
    /// container, in the tenant database.
    ///
    /// This matters because of who else is on that filesystem. An agent holding
    /// `shell` runs as the same uid as the server process, in the same
    /// container, so "plaintext on disk" means "readable by a prompt-injected
    /// agent" — there is no boundary in between, and
    /// `docs/spec/security/agent-isolation.md` is explicit that none is planned
    /// inside a tenant. A repository credential parked there is a credential the
    /// agent can read and use directly, without going through any tool the host
    /// gates.
    ///
    /// New backends default to the safe answer by being added to the `true` arm
    /// unless they demonstrably keep secrets off the local disk.
    ///
    /// [`FsSecretStore`]: crate::store::FsSecretStore
    /// [`SecretStore`]: crate::ports::SecretStore
    pub fn secrets_are_plaintext_on_disk(self) -> bool {
        match self {
            Self::Fs | Self::Sqlite => true,
            Self::Mongodb => false,
        }
    }
}

/// The refusal every repository-credential gate raises on a backend that keeps
/// secrets as plaintext on the container's disk (issue #752).
///
/// One function rather than a message per call site: the bind route, the boot
/// check and the agent-build gate all refuse the *same* deployment condition,
/// and an operator who reads it in the console then reads it again in the boot
/// log should not have to work out whether they are two problems.
///
/// Written to be self-service — it names the condition, the risk in one clause,
/// and both ways out — because the operator hitting it is mid-task with a token
/// in their clipboard, and "storage backend not supported" would send them to
/// the issue tracker instead of to a fix.
pub fn plaintext_secret_refusal(kind: StorageKind) -> String {
    format!(
        "this host keeps secrets on its own filesystem (OPENCOMPANY_STORAGE={}), so a \
         repository credential would sit there in plaintext — readable by the same uid the \
         agent shell runs as, which is not a boundary this deployment has. Repository \
         credentials are refused here. Either point this host at MongoDB \
         (OPENCOMPANY_STORAGE=mongodb plus OPENCOMPANY_MONGODB_URI, which keeps secrets in \
         the tenant database), or drop the `repo` grant from the company's [tools] allow \
         list and from every agent that names it. See docs/spec/runtime/storage.md.",
        kind.as_str()
    )
}

impl std::str::FromStr for StorageKind {
    type Err = OpenCompanyError;
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "fs" | "" => Ok(Self::Fs),
            "sqlite" => Ok(Self::Sqlite),
            "mongodb" | "mongo" => Ok(Self::Mongodb),
            other => Err(OpenCompanyError::Config(format!(
                "OPENCOMPANY_STORAGE must be 'fs', 'sqlite', or 'mongodb', got '{other}'"
            ))),
        }
    }
}

/// Which engine backs the memory + context ports, independent of the base
/// [`StorageKind`].
///
/// Memory is a separable concern: `OPENCOMPANY_STORAGE` picks the durable base
/// (companies, events, secrets, …) while `OPENCOMPANY_MEMORY` can swap just the
/// two knowledge ports onto a dedicated memory engine. This is why TinyCortex
/// is *not* a [`StorageKind`] — it only implements memory + context, not the
/// other durable ports, so it layers on top rather than replacing the base.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MemoryBackend {
    /// Memory + context come from the base [`StorageKind`] (the default; fs
    /// substring recall, or the sqlite/mongodb store).
    #[default]
    Store,
    /// The embedded engine, in-pod: ranked token-overlap recall over a
    /// compounding chunk store on top of whatever base backend is selected
    /// (`tinycortex` feature). Spelled `embedded` or `tinycortex`.
    Tinycortex,
    /// A hosted memory service behind a URL and a credential, bound through the
    /// `MemoryProvider` contract (`tinymemory` feature).
    ///
    /// Missing credentials refuse at boot. There is deliberately no fall back to
    /// the embedded engine: a company that believes it is writing to its hosted
    /// memory and is not is worse off than one that fails to start, because
    /// nothing surfaces the mistake until the memory is needed.
    Remote,
    /// Writes accepted and discarded, reads empty (`tinymemory` feature).
    Null,
}

impl MemoryBackend {
    /// The stable wire string for status output.
    ///
    /// [`Self::Tinycortex`] reports `embedded`, the vocabulary issue #914
    /// introduces. The legacy spelling stays accepted on the way *in* — see
    /// [`FromStr`](std::str::FromStr) — but only one name is reported out, so a
    /// client never has to know both.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Tinycortex => "embedded",
            Self::Remote => "remote",
            Self::Null => "null",
        }
    }
}

impl std::str::FromStr for MemoryBackend {
    type Err = OpenCompanyError;
    /// Parses `OPENCOMPANY_MEMORY`.
    ///
    /// `embedded` is a synonym for `tinycortex`, not a replacement: the older
    /// spelling keeps parsing forever. Renaming it would break every deployment
    /// that already sets it — including hosted tenants whose environment is
    /// injected by the control plane — for a cosmetic gain. The same reasoning
    /// already applies to `cortex` and to `mongo` on
    /// [`StorageKind`](StorageKind::from_str).
    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "store" | "" => Ok(Self::Store),
            "tinycortex" | "cortex" | "embedded" => Ok(Self::Tinycortex),
            "remote" => Ok(Self::Remote),
            "null" => Ok(Self::Null),
            other => Err(OpenCompanyError::Config(format!(
                "OPENCOMPANY_MEMORY must be 'store', 'embedded' (or its older spelling \
                 'tinycortex'), 'remote', or 'null', got '{other}'"
            ))),
        }
    }
}

/// The memory + context ports of a selected memory engine, ready to overlay
/// onto a company's builder after the base [`StorageHandles`] via
/// [`RuntimeBuilder::with_memory_overlay`](crate::runtime::RuntimeBuilder::with_memory_overlay).
#[derive(Clone)]
pub struct MemoryOverlay {
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextStore>,
    /// The operator's facts, when the selected engine serves them too.
    ///
    /// `None` for the embedded overlay, which implements memory + context only
    /// and leaves facts on the base backend. A provider-backed engine covers all
    /// three ports, so it fills this in rather than splitting one company's
    /// memory across two engines.
    pub facts: Option<Arc<dyn FactStore>>,
    /// What is bound, for status output.
    pub descriptor: MemoryDescriptor,
}

impl std::fmt::Debug for MemoryOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryOverlay")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// What memory engine is live, in terms safe to show an operator.
///
/// Deliberately carries no endpoint and no credential. `driver_id` is safe to
/// surface — the contract's own docs treat it as an identity, not a secret —
/// while the URL and the key are not, and this type is what reaches `/spec`,
/// which is unauthenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryDescriptor {
    /// The selected mode (`store`, `embedded`, `remote`, `null`).
    pub backend: MemoryBackend,
    /// The bound engine's own name, when one is bound.
    pub driver_id: String,
    /// The capability families the bound driver negotiated, so an operator can
    /// see what the engine does *not* support before a cycle finds out.
    pub capabilities: Vec<String>,
}

/// Durable company → tenant ownership, for shared-database platform mode.
/// Backends that can persist ownership (MongoDB today) expose it here so the
/// in-memory `AppState` map can be hydrated at boot and updated on provision.
#[async_trait]
pub trait OwnershipStore: Send + Sync {
    async fn set_owner(&self, id: &CompanyId, tenant: &str) -> Result<()>;
    async fn remove_owner(&self, id: &CompanyId) -> Result<()>;
    async fn owners(&self) -> Result<Vec<(CompanyId, String)>>;
}

/// One opened backend's implementations of every durable port, ready to be
/// injected into `RuntimeBuilder::with_stores`.
#[derive(Clone)]
pub struct StorageHandles {
    pub company: Arc<dyn CompanyStore>,
    pub events: Arc<dyn EventLog>,
    pub memory: Arc<dyn MemoryStore>,
    pub context: Arc<dyn ContextStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub inbox: Arc<dyn InboxStore>,
    pub tasks: Arc<dyn TaskStore>,
    /// The company's declared ledgers and their append-only event logs.
    pub ledgers: Arc<dyn LedgerStore>,
    pub workspace: Arc<dyn WorkspaceStore>,
    pub facts: Arc<dyn FactStore>,
    pub artifacts: Arc<dyn ArtifactStore>,
    /// First-class task-run records and their step traces (#242).
    pub runs: Arc<dyn RunStore>,
    /// Per-workflow edit history for rollback (#274).
    pub workflow_revisions: Arc<dyn WorkflowRevisionStore>,
    /// Durable cross-replica scheduler fire claims (#241).
    pub schedule_fires: Arc<dyn ScheduleFireStore>,
    /// Durable, console-facing per-node run output snapshots (#596).
    pub run_outputs: Arc<dyn WorkflowRunOutputStore>,
    pub usage: Arc<dyn UsageMeter>,
    pub skills: Arc<dyn SkillStateStore>,
    /// Per-person, per-channel read markers (#755).
    pub read_state: Arc<dyn ReadStateStore>,
    /// Durable notifications with per-person read state (#749).
    pub notifications: Arc<dyn NotificationStore>,
    pub users: Arc<dyn UserStore>,
    pub sessions: Arc<dyn SessionStore>,
    pub login_codes: Arc<dyn LoginCodeStore>,
    /// The runtime journal's durable sink (#726): at-most-once effect keys, the
    /// parked-approval queue, grants, and cycle brackets.
    ///
    /// Not `Option`, unlike [`ownership`](Self::ownership): a backend that
    /// cannot hold the journal cannot host a company at all, and a `None` here
    /// would be an invitation to fall back to the filesystem — which is exactly
    /// the bug (#726). On a mongodb tenant `/data` is ephemeral scratch, so a
    /// silent fs journal there loses every committed key and every parked
    /// approval the next time the container is replaced.
    pub journal: Arc<dyn JournalStore>,
    /// Present when the backend persists company → tenant ownership.
    pub ownership: Option<Arc<dyn OwnershipStore>>,
}

impl std::fmt::Debug for StorageHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageHandles")
            .field("ownership", &self.ownership.is_some())
            .finish_non_exhaustive()
    }
}

/// Connection settings for [`open_storage`]. `fs` needs nothing beyond the
/// runtime's home directory (handled by the builder's defaults), so it yields
/// `None` handles.
#[derive(Clone, Default)]
pub struct StorageSettings {
    pub kind: StorageKind,
    /// MongoDB connection string (`OPENCOMPANY_MONGODB_URI`).
    pub mongodb_uri: Option<String>,
    /// MongoDB database name (`OPENCOMPANY_MONGODB_DB`); the hosting layer
    /// sets a per-tenant name (e.g. `oc-<tenant>`) on a shared cluster.
    pub mongodb_db: Option<String>,
    /// Tenant identity for shared-single-DB deployments
    /// (`OPENCOMPANY_TENANT_ID`). When set, company ids are namespaced with
    /// this value so that many tenants sharing one logical database never
    /// collide on the `companies` unique index. Unset means the id-namespacing
    /// no-op: single-tenant / db-per-tenant behavior is unchanged.
    pub tenant_id: Option<String>,
    /// Which engine backs the memory + context ports (`OPENCOMPANY_MEMORY`),
    /// overlaid on top of `kind`. Defaults to [`MemoryBackend::Store`] (the base
    /// backend's own memory), so unset changes nothing.
    pub memory_backend: MemoryBackend,
    /// The instance workspace root (`OPENCOMPANY_DATA_DIR`), when known. Threaded
    /// through so a persistent memory engine can root each company's storage
    /// under `<data_dir>/memory/`. `None` (the [`Default`]) selects the offline
    /// in-memory engine — the shape tests and no-data-dir callers get.
    pub data_dir: Option<PathBuf>,
    /// Operator's explicit durability assertion for the data dir
    /// (`OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL`). The in-pod TinyCortex engine is
    /// refused by default under `OPENCOMPANY_STORAGE=mongodb`, because the hosted
    /// model treats `/data` as ephemeral scratch there and engine memory would be
    /// silently lost on restart. Setting this flag is the operator asserting that
    /// they have mounted a genuinely persistent volume at the data dir, which
    /// lifts the refusal for the mongodb+tinycortex combination. `false` (the
    /// [`Default`], and the safe default) keeps the silent-memory-loss guard.
    pub allow_ephemeral_memory: bool,
    /// Which engine to bind for `OPENCOMPANY_MEMORY=remote`
    /// (`OPENCOMPANY_MEMORY_DRIVER`): `supermemory`, `mem0`, `cognee`.
    ///
    /// A manifest `[memory].provider` outranks this, on the same principle as
    /// `[inference].provider` — the manifest travels with the company, the
    /// environment describes the host.
    pub memory_driver: Option<String>,
    /// Operator's explicit acceptance that the hosted memory adapters are not
    /// yet conformance-proven (`OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE`).
    ///
    /// `OPENCOMPANY_MEMORY=remote` routes a company's entire memory at a
    /// third-party HTTP service through an adapter that upstream covers with a
    /// handful of happy-path tests — no error mapping, no pagination, no taint
    /// preservation, no `Unsupported` behaviour. tinymemory#18 §E1 names a
    /// driver conformance suite as the gate for turning this on at all, and
    /// until that suite runs against those adapters, "it compiled" is the
    /// strongest thing anyone can say about them.
    ///
    /// So the mode exists, refuses by default, and lifts on an explicit
    /// assertion — the same shape as
    /// [`allow_ephemeral_memory`](Self::allow_ephemeral_memory), and for the
    /// same reason: the failure is silent and lands on a tenant's memory.
    /// `false` (the [`Default`]) keeps the guard.
    ///
    /// This is a gate on *confidence*, not on configuration, so it is expected
    /// to be deleted rather than lived with. Once the conformance suite covers
    /// the adapters, this field and its refusal go.
    pub allow_unproven_remote: bool,
    /// The hosted engine's endpoint (`OPENCOMPANY_MEMORY_URL`).
    pub memory_url: Option<String>,
    /// The hosted engine's credential (`OPENCOMPANY_MEMORY_API_KEY`).
    ///
    /// A raw credential, so it is kept out of [`Debug`] — see the impl below.
    /// The manifest form, `[memory].api_key_secret`, names a
    /// [`SecretStore`](crate::ports::SecretStore) key instead and is preferred:
    /// it is the convention every other integration follows, and it keeps the
    /// credential out of the process environment. This env var exists because
    /// the hosted manager injects environment, not manifests.
    pub memory_api_key: Option<String>,
}

impl std::fmt::Debug for StorageSettings {
    /// Renders everything except the two credentials.
    ///
    /// `StorageSettings` is printed at boot (`src/bin/opencompany.rs`), so a
    /// derived `Debug` would put a memory credential and a MongoDB connection
    /// string into the startup log of every tenant container.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageSettings")
            .field("kind", &self.kind)
            .field("mongodb_uri", &self.mongodb_uri.as_ref().map(|_| "<set>"))
            .field("mongodb_db", &self.mongodb_db)
            .field("tenant_id", &self.tenant_id)
            .field("memory_backend", &self.memory_backend)
            .field("data_dir", &self.data_dir)
            .field("allow_ephemeral_memory", &self.allow_ephemeral_memory)
            .field("allow_unproven_remote", &self.allow_unproven_remote)
            .field("memory_driver", &self.memory_driver)
            .field("memory_url", &self.memory_url.as_ref().map(|_| "<set>"))
            .field(
                "memory_api_key",
                &self.memory_api_key.as_ref().map(|_| "<set>"),
            )
            .finish()
    }
}

/// Parses env var `key` into `T`. Absent → `Ok(None)` (the caller applies its
/// default); a set-but-non-UTF-8 value is a hard [`OpenCompanyError::Config`]
/// rather than a silent fallback to the default.
fn parse_env<T>(key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr<Err = OpenCompanyError>,
{
    match std::env::var(key) {
        Ok(raw) => Ok(Some(raw.parse()?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(OpenCompanyError::Config(format!(
            "{key} is set but is not valid UTF-8"
        ))),
    }
}

/// Reads a boolean opt-in env flag. Truthy values (case-insensitive, trimmed):
/// `1`, `true`, `yes`, `on`. Anything else — including unset — is `false`.
fn env_flag(key: &str) -> bool {
    std::env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

impl StorageSettings {
    /// Reads the CLI-surface storage env vars (`OPENCOMPANY_STORAGE`,
    /// `OPENCOMPANY_MONGODB_URI`, `OPENCOMPANY_MONGODB_DB`,
    /// `OPENCOMPANY_TENANT_ID`, `OPENCOMPANY_MEMORY`, `OPENCOMPANY_DATA_DIR`,
    /// `OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL`,
    /// `OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE`).
    pub fn from_env() -> Result<Self> {
        let kind: StorageKind = parse_env("OPENCOMPANY_STORAGE")?.unwrap_or_default();
        let memory_backend: MemoryBackend = parse_env("OPENCOMPANY_MEMORY")?.unwrap_or_default();
        let non_empty = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
        Ok(Self {
            kind,
            mongodb_uri: non_empty("OPENCOMPANY_MONGODB_URI"),
            mongodb_db: non_empty("OPENCOMPANY_MONGODB_DB"),
            tenant_id: non_empty("OPENCOMPANY_TENANT_ID"),
            memory_backend,
            data_dir: Some(crate::app::config::data_dir_from_env()),
            allow_ephemeral_memory: env_flag("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL"),
            allow_unproven_remote: env_flag("OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE"),
            memory_driver: non_empty("OPENCOMPANY_MEMORY_DRIVER"),
            memory_url: non_empty("OPENCOMPANY_MEMORY_URL"),
            memory_api_key: non_empty("OPENCOMPANY_MEMORY_API_KEY"),
        })
    }
}

/// Opens the selected backend once. `Ok(None)` means "use the builder's fs
/// defaults"; a selected-but-unavailable backend is an error, never a silent
/// fs fallback.
pub async fn open_storage(
    settings: &StorageSettings,
    data_dir: &Path,
) -> Result<Option<StorageHandles>> {
    match settings.kind {
        StorageKind::Fs => Ok(None),
        StorageKind::Sqlite => open_sqlite(data_dir),
        StorageKind::Mongodb => open_mongodb(settings).await,
    }
}

/// Opens the memory + context overlay selected by `OPENCOMPANY_MEMORY`.
///
/// `Ok(None)` means [`MemoryBackend::Store`] — the base backend keeps its own
/// memory, no overlay. A selected-but-unavailable engine (feature disabled) is
/// an error, never a silent fallback, mirroring [`open_storage`].
pub fn open_memory_overlay(settings: &StorageSettings) -> Result<Option<MemoryOverlay>> {
    match settings.memory_backend {
        MemoryBackend::Store => Ok(None),
        MemoryBackend::Tinycortex => open_tinycortex(settings),
        MemoryBackend::Remote | MemoryBackend::Null => open_provider(settings),
    }
}

/// Opens a [`MemoryProvider`](tinymemory_api::provider::MemoryProvider)-backed
/// overlay for the `remote` and `null` modes.
///
/// Unlike [`open_tinycortex`], this one also carries a `FactStore`: the provider
/// contract covers all three memory ports, so there is no reason to leave the
/// operator's facts on the base backend and split a company's memory across two
/// engines.
#[cfg(feature = "tinymemory")]
fn open_provider(settings: &StorageSettings) -> Result<Option<MemoryOverlay>> {
    use crate::store::memory::{BoundMemory, MemoryDriverConfig, MemoryMode, open_driver};

    if settings.memory_backend == MemoryBackend::Remote && !settings.allow_unproven_remote {
        return Err(OpenCompanyError::Config(
            "OPENCOMPANY_MEMORY=remote routes this company's entire memory at a hosted service \
             through an adapter that is not yet covered by a driver conformance suite \
             (tinymemory#18 §E1): its error mapping, pagination, and provenance preservation are \
             untested, and a memory engine that loses provenance or silently drops a page fails \
             in ways nothing surfaces until the memory is needed. If you accept that for this \
             deployment, set OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE=1 to assert it. Otherwise \
             use OPENCOMPANY_MEMORY=embedded for the durable in-pod engine, or the default \
             OPENCOMPANY_MEMORY=store."
                .to_string(),
        ));
    }
    let mode = match settings.memory_backend {
        MemoryBackend::Remote => MemoryMode::Remote,
        MemoryBackend::Null => MemoryMode::Null,
        // Unreachable: the caller matched these two arms.
        MemoryBackend::Store | MemoryBackend::Tinycortex => return Ok(None),
    };
    let config = MemoryDriverConfig {
        mode,
        driver_id: settings.memory_driver.clone(),
        url: settings.memory_url.clone(),
        api_key: settings.memory_api_key.clone(),
    };
    let Some((provider, class)) = open_driver(&config)? else {
        return Ok(None);
    };
    let bound = BoundMemory::bind(provider, class)?;
    // Announce the bind: which engine, and — the part an operator cannot infer —
    // the class the *host* assigned it, since that is what decides whether the
    // egress and external-trust checks apply. Names the engine and its
    // capabilities, never the endpoint or the credential.
    tracing::info!(
        driver_id = bound.driver_id(),
        class = bound.class().as_str(),
        capabilities = ?bound.capability_names(),
        "memory engine bound"
    );
    if settings.memory_backend == MemoryBackend::Null {
        // Loud, once, at open: `null` is a legitimate choice but a surprising
        // one to inherit from a stale environment, and every read returning
        // empty is indistinguishable from a company that has not learned
        // anything yet.
        tracing::warn!(
            "OPENCOMPANY_MEMORY=null is bound: memory writes are accepted and discarded, and \
             every read is empty. Nothing this company is told will be remembered."
        );
    }
    Ok(Some(MemoryOverlay {
        memory: bound.memory(),
        context: bound.context(),
        facts: Some(bound.facts()),
        descriptor: MemoryDescriptor {
            backend: settings.memory_backend,
            driver_id: bound.driver_id().to_string(),
            capabilities: bound
                .capability_names()
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
    }))
}

/// Without the `tinymemory` feature the two provider-backed modes cannot be
/// served, so they refuse rather than silently resolving to something else.
#[cfg(not(feature = "tinymemory"))]
fn open_provider(settings: &StorageSettings) -> Result<Option<MemoryOverlay>> {
    Err(OpenCompanyError::Config(format!(
        "OPENCOMPANY_MEMORY={} requires a build with the `tinymemory` feature",
        settings.memory_backend.as_str()
    )))
}

/// Opens the TinyCortex overlay. With a `data_dir` present it is the persistent,
/// in-pod [`EngineCortex`](crate::store::tinycortex_engine::EngineCortex) rooted
/// at `<data_dir>/memory/`; without one (tests, no-data-dir callers) it is the
/// offline in-memory backend.
///
/// Two boot-time contracts are enforced here rather than left to silently
/// surprise an operator at runtime:
///
/// 1. **Refuse-to-open on ephemeral `/data`, unless durability is asserted.**
///    `OPENCOMPANY_STORAGE=mongodb` makes the container's data dir ephemeral
///    scratch (the database is the durable base), so an in-pod engine rooted
///    there would lose *all* memory on every restart. That is silent data loss,
///    so by default this combination is a hard [`OpenCompanyError::Config`] — we
///    never open a doomed engine. But storage-kind is only a *proxy* for
///    "ephemeral `/data`": a mongodb deployment that HAS mounted a persistent
///    volume at the data dir is perfectly safe. So the refusal is an explicit
///    durability contract, not a hard-coded storage-kind rejection: an operator
///    who has mounted a durable volume sets
///    `OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL=1` (surfaced as
///    [`StorageSettings::allow_ephemeral_memory`]) to assert it, and the engine
///    opens. Unset (the safe default) still refuses the mongodb+tinycortex combo.
/// 2. **Meaning tier, with a loud degraded-mode fallback.** A hosted embeddings
///    backend is resolved from the environment (188c2); when one is present each
///    stored chunk is embedded and recall runs vector-first (cosine) with a
///    lexical top-up. When **no** backend resolves, recall degrades to *lexical*
///    (substring/recency token-overlap) — **not** the vector/semantic recall the
///    `tinycortex` name implies — and that is announced once, loudly, at open so
///    it is never mistaken for real embedding recall.
#[cfg(feature = "tinycortex")]
fn open_tinycortex(settings: &StorageSettings) -> Result<Option<MemoryOverlay>> {
    let (memory, context) = match &settings.data_dir {
        Some(dir) => {
            // Refuse-to-open contract: the engine persists to `<data_dir>/memory`
            // on the local container filesystem. Under `OPENCOMPANY_STORAGE=mongodb`
            // the hosting model treats `/data` as ephemeral scratch (the durable
            // base is the database), so engine memory would be silently lost on
            // every restart. Refusing to open beats warning-then-losing-data: the
            // failure mode we are guarding against is exactly a quiet memory wipe on
            // restart. But storage-kind is only a proxy for "ephemeral /data" — a
            // mongodb deploy with a genuinely persistent volume is safe — so the
            // operator can lift the refusal by explicitly asserting durability via
            // OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL. See docs/spec/runtime/storage.md.
            if settings.kind == StorageKind::Mongodb && !settings.allow_ephemeral_memory {
                return Err(OpenCompanyError::Config(
                    "OPENCOMPANY_MEMORY=tinycortex needs a persistent volume at the data dir, but \
                     OPENCOMPANY_STORAGE=mongodb makes /data ephemeral scratch by default, so \
                     in-pod memory would be silently lost on restart. If you have mounted a \
                     genuinely persistent volume at OPENCOMPANY_DATA_DIR, set \
                     OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL=1 to assert its durability and open the \
                     engine anyway. Otherwise use OPENCOMPANY_STORAGE=fs or sqlite (durable /data), \
                     or keep memory on the base store with OPENCOMPANY_MEMORY=store."
                        .into(),
                ));
            }
            // Meaning tier (188c2): resolve a hosted embeddings backend from the
            // environment when one is configured, so recall is vector-first
            // (semantic) rather than lexical-only. `None` (no hosted credential, or
            // a default build without the `openhuman` harness) keeps the lexical
            // path — the embeddings client lives in the openhuman-gated harness, so
            // the type is only reachable there.
            let embeddings = hosted_embeddings_backend();
            // Loud, one-time degraded-mode contract: with no embeddings backend
            // recall is lexical (substring/recency token-overlap), NOT the
            // vector/semantic recall the name implies. Announce it once at open so
            // it is never mistaken for real embedding recall.
            if embeddings.is_none() {
                tracing::warn!(
                    data_dir = %dir.display(),
                    "OPENCOMPANY_MEMORY=tinycortex is running in DEGRADED lexical fallback mode: no \
                     embeddings backend resolved, so recall is substring/recency token-overlap, \
                     NOT vector/semantic recall. Configure a hosted embeddings backend for \
                     semantic recall.",
                );
            }
            crate::store::tinycortex_engine::engine_with_embeddings(dir.join("memory"), embeddings)
        }
        None => crate::store::tinycortex::in_memory(),
    };
    Ok(Some(MemoryOverlay {
        memory,
        context,
        // The embedded engine implements memory + context only; facts stay on
        // the base backend, as they always have.
        facts: None,
        descriptor: MemoryDescriptor {
            backend: MemoryBackend::Tinycortex,
            // The literal rather than `tinymemory::registry::TINYCORTEX_DRIVER_ID`:
            // this arm compiles under `tinycortex` alone, which does not pull the
            // registry in. It is the same reserved id, and `tinymemory`'s own
            // constant is pinned to this string by its builtin table.
            driver_id: "tinycortex".to_string(),
            // The in-pod engine is driven directly rather than through a bound
            // provider, so there is no negotiated capability set to report. An
            // empty list says "not negotiated", which is the truth; claiming the
            // mandatory three here would be reporting a bind that did not happen.
            capabilities: Vec::new(),
        },
    }))
}

/// Resolves the hosted embeddings backend for the memory meaning tier from the
/// process environment, as an `Arc<dyn EmbeddingBackend>` the vector store
/// consumes. Only the `openhuman` build can build one (that is where the hosted
/// embeddings client lives); every other build gets `None` and lexical recall.
#[cfg(feature = "tinycortex")]
fn hosted_embeddings_backend()
-> Option<Arc<dyn tinycortex::memory::store::vectors::embedding::EmbeddingBackend>> {
    #[cfg(feature = "openhuman")]
    {
        use tinycortex::memory::store::vectors::embedding::EmbeddingBackend;
        crate::harness::embeddings::hosted_embeddings_from_env(&crate::app::config::ProcessEnv)
            .map(|backend| Arc::new(backend) as Arc<dyn EmbeddingBackend>)
    }
    #[cfg(not(feature = "openhuman"))]
    {
        None
    }
}

#[cfg(not(feature = "tinycortex"))]
fn open_tinycortex(_settings: &StorageSettings) -> Result<Option<MemoryOverlay>> {
    Err(OpenCompanyError::Config(
        "OPENCOMPANY_MEMORY=tinycortex requires a build with the `tinycortex` feature".into(),
    ))
}

#[cfg(feature = "sqlite")]
fn open_sqlite(data_dir: &Path) -> Result<Option<StorageHandles>> {
    let store = Arc::new(crate::store::SqliteStore::open(
        data_dir.join("opencompany.db"),
    )?);
    Ok(Some(StorageHandles {
        company: store.clone(),
        events: store.clone(),
        memory: store.clone(),
        context: store.clone(),
        secrets: store.clone(),
        inbox: store.clone(),
        tasks: store.clone(),
        ledgers: store.clone(),
        workspace: store.clone(),
        facts: store.clone(),
        artifacts: store.clone(),
        runs: store.clone(),
        workflow_revisions: store.clone(),
        schedule_fires: store.clone(),
        run_outputs: store.clone(),
        usage: store.clone(),
        skills: store.clone(),
        read_state: store.clone(),
        notifications: store.clone(),
        users: store.clone(),
        sessions: store.clone(),
        login_codes: store.clone(),
        journal: store,
        ownership: None,
    }))
}

#[cfg(not(feature = "sqlite"))]
fn open_sqlite(_data_dir: &Path) -> Result<Option<StorageHandles>> {
    Err(OpenCompanyError::Config(
        "OPENCOMPANY_STORAGE=sqlite requires a build with the `sqlite` feature".into(),
    ))
}

#[cfg(feature = "mongodb")]
async fn open_mongodb(settings: &StorageSettings) -> Result<Option<StorageHandles>> {
    let uri = settings.mongodb_uri.as_deref().ok_or_else(|| {
        OpenCompanyError::Config(
            "OPENCOMPANY_STORAGE=mongodb requires OPENCOMPANY_MONGODB_URI".into(),
        )
    })?;
    let db = settings.mongodb_db.as_deref().unwrap_or("opencompany");
    let store = Arc::new(crate::store::MongoStore::connect(uri, db).await?);
    Ok(Some(StorageHandles {
        company: store.clone(),
        events: store.clone(),
        memory: store.clone(),
        context: store.clone(),
        secrets: store.clone(),
        inbox: store.clone(),
        tasks: store.clone(),
        ledgers: store.clone(),
        workspace: store.clone(),
        facts: store.clone(),
        artifacts: store.clone(),
        runs: store.clone(),
        workflow_revisions: store.clone(),
        schedule_fires: store.clone(),
        run_outputs: store.clone(),
        usage: store.clone(),
        skills: store.clone(),
        read_state: store.clone(),
        notifications: store.clone(),
        users: store.clone(),
        sessions: store.clone(),
        login_codes: store.clone(),
        journal: store.clone(),
        ownership: Some(store),
    }))
}

#[cfg(not(feature = "mongodb"))]
async fn open_mongodb(_settings: &StorageSettings) -> Result<Option<StorageHandles>> {
    Err(OpenCompanyError::Config(
        "OPENCOMPANY_STORAGE=mongodb requires a build with the `mongodb` feature".into(),
    ))
}

#[cfg(feature = "mongodb")]
#[async_trait]
impl OwnershipStore for crate::store::MongoStore {
    async fn set_owner(&self, id: &CompanyId, tenant: &str) -> Result<()> {
        crate::store::MongoStore::set_owner(self, id, tenant).await
    }
    async fn remove_owner(&self, id: &CompanyId) -> Result<()> {
        crate::store::MongoStore::remove_owner(self, id).await
    }
    async fn owners(&self) -> Result<Vec<(CompanyId, String)>> {
        crate::store::MongoStore::owners(self).await
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_storage_kinds() {
        assert_eq!("fs".parse::<StorageKind>().unwrap(), StorageKind::Fs);
        assert_eq!(
            "sqlite".parse::<StorageKind>().unwrap(),
            StorageKind::Sqlite
        );
        assert_eq!(
            "MongoDB".parse::<StorageKind>().unwrap(),
            StorageKind::Mongodb
        );
        assert!("postgres".parse::<StorageKind>().is_err());
    }

    /// Issue #752: only MongoDB keeps secret material off the container's own
    /// disk, so only MongoDB clears the repository-credential gates.
    #[test]
    fn only_mongodb_keeps_secrets_off_the_local_disk() {
        assert!(StorageKind::Fs.secrets_are_plaintext_on_disk());
        assert!(StorageKind::Sqlite.secrets_are_plaintext_on_disk());
        assert!(!StorageKind::Mongodb.secrets_are_plaintext_on_disk());
        // The default is the refusing side: a host that never resolved a
        // backend must not be treated as one that keeps secrets safely.
        assert!(StorageKind::default().secrets_are_plaintext_on_disk());
    }

    /// The refusal has to be actionable on its own — an operator reading it in
    /// a console toast has nothing else to go on.
    #[test]
    fn the_refusal_names_the_condition_and_both_remedies() {
        let message = plaintext_secret_refusal(StorageKind::Fs);
        assert!(message.contains("OPENCOMPANY_STORAGE=fs"), "{message}");
        assert!(message.contains("OPENCOMPANY_STORAGE=mongodb"), "{message}");
        assert!(message.contains("OPENCOMPANY_MONGODB_URI"), "{message}");
        assert!(message.contains("`repo` grant"), "{message}");
        assert!(message.contains("plaintext"), "{message}");
        // The named kind is the one actually in force, not a hard-coded "fs".
        assert!(
            plaintext_secret_refusal(StorageKind::Sqlite).contains("OPENCOMPANY_STORAGE=sqlite"),
        );
    }

    #[tokio::test]
    async fn fs_selection_uses_builder_defaults() {
        let settings = StorageSettings::default();
        let handles = open_storage(&settings, Path::new("/tmp")).await.unwrap();
        assert!(handles.is_none());
    }

    #[test]
    fn parses_memory_backends() {
        assert_eq!(
            "store".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Store
        );
        assert_eq!("".parse::<MemoryBackend>().unwrap(), MemoryBackend::Store);
        assert_eq!(
            "TinyCortex".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Tinycortex
        );
        assert_eq!(
            "cortex".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Tinycortex
        );
        assert!("redis".parse::<MemoryBackend>().is_err());
    }

    #[test]
    fn the_legacy_spellings_keep_parsing_alongside_the_new_ones() {
        // Issue #914 introduces `embedded`/`remote`/`null`. Renaming would break
        // every deployment already setting `OPENCOMPANY_MEMORY=tinycortex`,
        // including hosted tenants whose environment the control plane injects,
        // so the older spelling is a synonym rather than a casualty.
        assert_eq!(
            "embedded".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Tinycortex
        );
        assert_eq!(
            "tinycortex".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Tinycortex
        );
        assert_eq!(
            "remote".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Remote
        );
        assert_eq!(
            "null".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Null
        );
        assert_eq!(
            "NULL".parse::<MemoryBackend>().unwrap(),
            MemoryBackend::Null
        );
    }

    #[test]
    fn one_spelling_is_reported_out_even_though_two_parse_in() {
        // A client reading status should never have to know both names.
        assert_eq!(MemoryBackend::Tinycortex.as_str(), "embedded");
        assert_eq!(MemoryBackend::Store.as_str(), "store");
        assert_eq!(MemoryBackend::Remote.as_str(), "remote");
        assert_eq!(MemoryBackend::Null.as_str(), "null");
    }

    #[test]
    fn the_parse_refusal_names_every_accepted_value() {
        let error = "redis".parse::<MemoryBackend>().err().unwrap().to_string();
        for value in ["store", "embedded", "tinycortex", "remote", "null"] {
            assert!(error.contains(value), "{value} missing from: {error}");
        }
    }

    #[test]
    fn settings_debug_never_renders_a_credential() {
        // `StorageSettings` is printed at boot, so a derived `Debug` would put a
        // memory credential and a MongoDB connection string in the startup log
        // of every tenant container.
        let settings = StorageSettings {
            mongodb_uri: Some("mongodb://user:hunter2@cluster.example/db".into()),
            memory_url: Some("https://memory.internal.example".into()),
            memory_api_key: Some("sk-memory-super-secret".into()),
            ..StorageSettings::default()
        };
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("sk-memory-super-secret"), "{rendered}");
        assert!(!rendered.contains("memory.internal.example"), "{rendered}");
        // Still useful: it says the values are configured.
        assert!(rendered.contains("<set>"), "{rendered}");
    }

    #[cfg(feature = "tinymemory")]
    #[test]
    fn remote_without_a_url_or_key_refuses_at_open() {
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Remote,
            memory_driver: Some("supermemory".into()),
            // Past the confidence gate, so this asserts the *configuration*
            // refusal rather than tripping over the one before it.
            allow_unproven_remote: true,
            ..StorageSettings::default()
        };
        let error = open_memory_overlay(&settings)
            .expect_err("remote without an endpoint must refuse")
            .to_string();
        assert!(error.contains("OPENCOMPANY_MEMORY_URL"), "{error}");
    }

    #[cfg(feature = "tinymemory")]
    #[test]
    fn remote_refuses_until_the_operator_accepts_an_unproven_adapter() {
        // The hosted adapters have no conformance coverage yet
        // (tinymemory#18 §E1). Routing a tenant's whole memory at one is a
        // decision an operator makes deliberately, not one they arrive at by
        // setting a single variable.
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Remote,
            memory_driver: Some("supermemory".into()),
            memory_url: Some("https://memory.example".into()),
            memory_api_key: Some("k".into()),
            ..StorageSettings::default()
        };
        let error = open_memory_overlay(&settings)
            .expect_err("a fully configured remote engine still needs the assertion")
            .to_string();
        assert!(
            error.contains("OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE"),
            "the refusal must name the knob: {error}"
        );
        // And it must name a way forward that is not "give up".
        assert!(error.contains("OPENCOMPANY_MEMORY=embedded"), "{error}");
    }

    #[cfg(feature = "tinymemory")]
    #[test]
    fn remote_opens_once_the_operator_has_accepted_it() {
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Remote,
            memory_driver: Some("supermemory".into()),
            memory_url: Some("https://memory.example".into()),
            memory_api_key: Some("k".into()),
            allow_unproven_remote: true,
            ..StorageSettings::default()
        };
        let overlay = open_memory_overlay(&settings)
            .expect("an accepted, fully configured remote engine binds")
            .expect("remote yields an overlay");
        assert_eq!(overlay.descriptor.backend, MemoryBackend::Remote);
        assert_eq!(overlay.descriptor.driver_id, "supermemory");
    }

    #[cfg(feature = "tinymemory")]
    #[test]
    fn the_gate_applies_only_to_remote() {
        // `null` retains nothing by design, and `embedded` is the incumbent
        // durable path — neither is routing memory at an unproven third party,
        // so neither is behind this gate.
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Null,
            ..StorageSettings::default()
        };
        assert!(
            open_memory_overlay(&settings).is_ok(),
            "null must not be gated on the remote-adapter assertion"
        );
    }

    #[cfg(feature = "tinymemory")]
    #[test]
    fn null_opens_and_reports_itself() {
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Null,
            ..StorageSettings::default()
        };
        let overlay = open_memory_overlay(&settings)
            .unwrap()
            .expect("null binds an overlay");
        assert_eq!(overlay.descriptor.backend, MemoryBackend::Null);
        assert_eq!(overlay.descriptor.driver_id, "null");
        assert!(overlay.facts.is_some(), "a provider serves facts too");
    }

    #[cfg(not(feature = "tinymemory"))]
    #[test]
    fn the_provider_modes_require_the_feature() {
        for backend in [MemoryBackend::Remote, MemoryBackend::Null] {
            let settings = StorageSettings {
                memory_backend: backend,
                ..StorageSettings::default()
            };
            let error = open_memory_overlay(&settings).err().unwrap().to_string();
            assert!(error.contains("`tinymemory` feature"), "{error}");
        }
    }

    #[test]
    fn default_memory_backend_is_store() {
        assert_eq!(
            StorageSettings::default().memory_backend,
            MemoryBackend::Store
        );
        // Store is the no-op: no overlay, base backend keeps its own memory.
        assert!(
            open_memory_overlay(&StorageSettings::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn from_env_reads_the_remote_memory_knobs() {
        // The four knobs `remote` needs. Without this, a rename in `from_env`
        // would surface as "the engine refuses and names a variable you did
        // set", which reads as a broken deployment rather than a broken parse.
        const KEYS: [&str; 4] = [
            "OPENCOMPANY_MEMORY_ALLOW_UNPROVEN_REMOTE",
            "OPENCOMPANY_MEMORY_DRIVER",
            "OPENCOMPANY_MEMORY_URL",
            "OPENCOMPANY_MEMORY_API_KEY",
        ];
        // SAFETY: single-threaded test; restores prior state.
        let prev: Vec<_> = KEYS.iter().map(|k| (*k, std::env::var(k).ok())).collect();

        unsafe {
            std::env::set_var(KEYS[0], "1");
            std::env::set_var(KEYS[1], "supermemory");
            std::env::set_var(KEYS[2], "https://memory.example");
            std::env::set_var(KEYS[3], "sk-test");
        }
        let settings = StorageSettings::from_env().unwrap();
        assert!(settings.allow_unproven_remote);
        assert_eq!(settings.memory_driver.as_deref(), Some("supermemory"));
        assert_eq!(
            settings.memory_url.as_deref(),
            Some("https://memory.example")
        );
        assert_eq!(settings.memory_api_key.as_deref(), Some("sk-test"));

        // Empty is absent, not an empty credential: `require` would otherwise
        // accept a blank key and defer the failure to the first call.
        unsafe {
            std::env::set_var(KEYS[1], "");
            std::env::set_var(KEYS[3], "");
        }
        let blank = StorageSettings::from_env().unwrap();
        assert_eq!(blank.memory_driver, None);
        assert_eq!(blank.memory_api_key, None);

        unsafe {
            for key in KEYS {
                std::env::remove_var(key);
            }
        }
        let unset = StorageSettings::from_env().unwrap();
        assert!(!unset.allow_unproven_remote);
        assert_eq!(unset.memory_driver, None);
        assert_eq!(unset.memory_url, None);
        assert_eq!(unset.memory_api_key, None);

        unsafe {
            for (key, value) in prev {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn from_env_reads_memory_backend() {
        // SAFETY: single-threaded test; restores prior state.
        let prev = std::env::var("OPENCOMPANY_MEMORY").ok();

        unsafe { std::env::set_var("OPENCOMPANY_MEMORY", "tinycortex") };
        assert_eq!(
            StorageSettings::from_env().unwrap().memory_backend,
            MemoryBackend::Tinycortex
        );

        unsafe { std::env::remove_var("OPENCOMPANY_MEMORY") };
        assert_eq!(
            StorageSettings::from_env().unwrap().memory_backend,
            MemoryBackend::Store
        );

        match prev {
            Some(v) => unsafe { std::env::set_var("OPENCOMPANY_MEMORY", v) },
            None => unsafe { std::env::remove_var("OPENCOMPANY_MEMORY") },
        }
    }

    #[cfg(feature = "tinycortex")]
    #[tokio::test]
    async fn tinycortex_overlay_recalls_stored_chunks() {
        use crate::ports::types::{CompanyId, ContextChunk};

        let settings = StorageSettings {
            memory_backend: MemoryBackend::Tinycortex,
            ..Default::default()
        };
        let overlay = open_memory_overlay(&settings).unwrap().expect("overlay");

        let company = CompanyId::new("acme");
        overlay
            .context
            .put(
                &company,
                ContextChunk {
                    label: "notes/q3".into(),
                    body: "revenue grew in the q3 report".into(),
                },
            )
            .await
            .unwrap();

        // The chunk is recallable by content — the compounding-memory contract.
        let hits = overlay
            .context
            .search(&company, "q3 revenue", 5)
            .await
            .unwrap();
        assert!(hits.iter().any(|h| h.score > 0.0), "expected a ranked hit");

        // Isolation: another company never sees acme's chunk.
        let other = CompanyId::new("globex");
        let leaked = overlay
            .context
            .search(&other, "q3 revenue", 5)
            .await
            .unwrap();
        assert!(leaked.is_empty(), "cross-company recall must not bleed");
    }

    /// Refuse-to-open contract: `OPENCOMPANY_STORAGE=mongodb` makes `/data`
    /// ephemeral, so opening the in-pod engine there would silently lose memory
    /// on restart. That combination must be a hard error, not a warning.
    #[cfg(feature = "tinycortex")]
    #[test]
    fn tinycortex_refuses_ephemeral_mongodb_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let settings = StorageSettings {
            kind: StorageKind::Mongodb,
            memory_backend: MemoryBackend::Tinycortex,
            data_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let err = open_memory_overlay(&settings).expect_err("mongodb /data must refuse to open");
        let msg = err.to_string();
        assert!(
            msg.contains("silently lost on restart"),
            "error must name the silent-memory-loss failure mode, got: {msg}"
        );
    }

    /// The refusal is an explicit durability *contract*, not a hard storage-kind
    /// reject: an operator who has mounted a persistent volume under a mongodb
    /// deployment asserts it via `OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL` (surfaced as
    /// `allow_ephemeral_memory`), and the engine then opens instead of refusing.
    #[cfg(feature = "tinycortex")]
    #[test]
    fn tinycortex_opens_ephemeral_mongodb_when_durability_asserted() {
        let dir = tempfile::tempdir().unwrap();
        let settings = StorageSettings {
            kind: StorageKind::Mongodb,
            memory_backend: MemoryBackend::Tinycortex,
            data_dir: Some(dir.path().to_path_buf()),
            allow_ephemeral_memory: true,
            ..Default::default()
        };
        assert!(
            open_memory_overlay(&settings)
                .unwrap_or_else(|e| panic!("durability-asserted mongodb must open: {e}"))
                .is_some(),
            "asserting durability must lift the mongodb+tinycortex refusal"
        );
    }

    /// The refuse is scoped to the ephemeral-`/data` combination only: durable
    /// base backends (fs, sqlite) still open the engine overlay normally.
    #[cfg(feature = "tinycortex")]
    #[test]
    fn tinycortex_opens_on_durable_fs_and_sqlite() {
        for kind in [StorageKind::Fs, StorageKind::Sqlite] {
            let dir = tempfile::tempdir().unwrap();
            let settings = StorageSettings {
                kind,
                memory_backend: MemoryBackend::Tinycortex,
                data_dir: Some(dir.path().to_path_buf()),
                ..Default::default()
            };
            assert!(
                open_memory_overlay(&settings)
                    .unwrap_or_else(|e| panic!("durable {kind:?} base must open: {e}"))
                    .is_some(),
                "durable {kind:?} base must yield an engine overlay"
            );
        }
    }

    #[cfg(not(feature = "tinycortex"))]
    #[test]
    fn tinycortex_overlay_requires_feature() {
        let settings = StorageSettings {
            memory_backend: MemoryBackend::Tinycortex,
            ..Default::default()
        };
        assert!(open_memory_overlay(&settings).is_err());
    }

    #[test]
    fn from_env_reads_tenant_id() {
        // SAFETY: single-threaded test; restores prior state.
        let prev = std::env::var("OPENCOMPANY_TENANT_ID").ok();

        unsafe { std::env::set_var("OPENCOMPANY_TENANT_ID", "acme") };
        assert_eq!(
            StorageSettings::from_env().unwrap().tenant_id.as_deref(),
            Some("acme")
        );

        // An empty value is filtered out, same as the mongodb vars.
        unsafe { std::env::set_var("OPENCOMPANY_TENANT_ID", "") };
        assert_eq!(StorageSettings::from_env().unwrap().tenant_id, None);

        // Unset leaves it `None` (the id-namespacing no-op).
        unsafe { std::env::remove_var("OPENCOMPANY_TENANT_ID") };
        assert_eq!(StorageSettings::from_env().unwrap().tenant_id, None);

        match prev {
            Some(v) => unsafe { std::env::set_var("OPENCOMPANY_TENANT_ID", v) },
            None => unsafe { std::env::remove_var("OPENCOMPANY_TENANT_ID") },
        }
    }

    #[test]
    fn from_env_reads_data_dir() {
        // SAFETY: single-threaded test; restores prior state.
        let prev = std::env::var("OPENCOMPANY_DATA_DIR").ok();

        // An explicit data dir is threaded straight through into settings.
        unsafe { std::env::set_var("OPENCOMPANY_DATA_DIR", "/srv/oc-data") };
        assert_eq!(
            StorageSettings::from_env().unwrap().data_dir,
            Some(PathBuf::from("/srv/oc-data")),
            "OPENCOMPANY_DATA_DIR must be read into StorageSettings::data_dir"
        );

        match prev {
            Some(v) => unsafe { std::env::set_var("OPENCOMPANY_DATA_DIR", v) },
            None => unsafe { std::env::remove_var("OPENCOMPANY_DATA_DIR") },
        }
    }

    #[test]
    fn from_env_reads_allow_ephemeral_memory() {
        // SAFETY: single-threaded test; restores prior state.
        let prev = std::env::var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL").ok();

        // Unset → the safe default: refuse (flag false).
        unsafe { std::env::remove_var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL") };
        assert!(!StorageSettings::from_env().unwrap().allow_ephemeral_memory);

        // Truthy values set the durability assertion.
        for truthy in ["1", "true", "YES", "On"] {
            unsafe { std::env::set_var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL", truthy) };
            assert!(
                StorageSettings::from_env().unwrap().allow_ephemeral_memory,
                "{truthy:?} must read as durability asserted"
            );
        }

        // Any non-truthy value stays false (fails safe toward refusal).
        for falsy in ["0", "false", "no", ""] {
            unsafe { std::env::set_var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL", falsy) };
            assert!(
                !StorageSettings::from_env().unwrap().allow_ephemeral_memory,
                "{falsy:?} must read as not asserted"
            );
        }

        match prev {
            Some(v) => unsafe { std::env::set_var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL", v) },
            None => unsafe { std::env::remove_var("OPENCOMPANY_MEMORY_ALLOW_EPHEMERAL") },
        }
    }

    #[cfg(feature = "mongodb")]
    #[tokio::test]
    async fn mongodb_selection_requires_uri() {
        let settings = StorageSettings {
            kind: StorageKind::Mongodb,
            ..Default::default()
        };
        assert!(open_storage(&settings, Path::new("/tmp")).await.is_err());
    }
}
