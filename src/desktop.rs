//! The local runtime behind the packaged OpenCompany desktop application.
//!
//! The desktop app is a thin Tauri shell around the existing React console and
//! Axum operator API.  It binds only loopback, persists under the app-data
//! directory supplied by Tauri, and starts from one of the shipped company
//! presets.  There is intentionally no OpenHuman local-AI preset bridge here:
//! OpenCompany owns its local company store and its company presets.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::Serialize;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::company::CompanyManifest;
use crate::runtime::{RuntimeBuilder, company_id_from_name};
use crate::server::cors::CorsConfig;
use crate::{AppConfig, AppState, Result};

/// A company definition bundled into the desktop app.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct DesktopPreset {
    /// Stable identifier used by the desktop host.
    pub id: &'static str,
    /// Human-readable company template name.
    pub name: &'static str,
    manifest: &'static str,
}

macro_rules! preset {
    ($id:literal, $name:literal) => {
        DesktopPreset {
            id: $id,
            name: $name,
            manifest: include_str!(concat!("../companies/", $id, "/company.toml")),
        }
    };
}

/// The product company templates shipped with the desktop app.
pub const PRESETS: &[DesktopPreset] = &[
    preset!("agentic_accounting_firm", "Agentic Accounting Firm"),
    preset!("agentic_consultation_firm", "Agentic Consultation Firm"),
    preset!("agentic_customer_support", "Agentic Customer Support"),
    preset!("agentic_design_studio", "Agentic Design Studio"),
    preset!("agentic_enterprise_sales", "Agentic Enterprise Sales"),
    preset!("agentic_game_business", "Agentic Game Business"),
    preset!("agentic_game_studio", "Agentic Game Studio"),
    preset!("agentic_influencer_business", "Agentic Influencer Business"),
    preset!("agentic_law_firm", "Agentic Law Firm"),
    preset!("agentic_marketing_agency", "Agentic Marketing Agency"),
    preset!("agentic_media_company", "Agentic Media Company"),
    preset!("agentic_pharma_startup", "Agentic Pharma Startup"),
    preset!("agentic_realestate_company", "Agentic Real Estate Company"),
    preset!("agentic_recruiting_company", "Agentic Recruiting Company"),
    preset!("agentic_software_company", "Agentic Software Company"),
    preset!("agentic_venture_capital", "Agentic Venture Capital"),
    preset!("agentic_venture_studio", "Agentic Venture Studio"),
    preset!("signals_opportunity_studio", "Signals Opportunity Studio"),
    preset!("startup_accelerator", "Startup Accelerator"),
];

/// The preset a first-run desktop install uses.
pub const DEFAULT_PRESET_ID: &str = "agentic_marketing_agency";
/// The local standing invite used only to bootstrap the desktop webview session.
pub const DESKTOP_OPERATOR_EMAIL: &str = "operator@opencompany.local";
/// The origin used by Tauri v2's desktop webview.
pub const TAURI_WEBVIEW_ORIGIN: &str = "http://tauri.localhost";

/// Finds a bundled preset by its stable id.
pub fn preset(id: &str) -> Option<&'static DesktopPreset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// Information the webview needs to authenticate to its embedded runtime.
#[derive(Clone, Debug, Serialize)]
pub struct DesktopConfig {
    pub api_url: String,
    pub company: String,
    pub operator_email: &'static str,
}

/// A running local OpenCompany API. Dropping it aborts the loopback server.
pub struct DesktopRuntime {
    config: DesktopConfig,
    server: JoinHandle<Result<()>>,
}

impl DesktopRuntime {
    pub fn config(&self) -> &DesktopConfig {
        &self.config
    }
}

impl Drop for DesktopRuntime {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// The manifest a first-run install starts from: a bundled preset, with the
/// local operator added as a standing admin.
///
/// The admin entry is what makes the company signable-into. `eligibility` in
/// `src/server/users/routes.rs` admits an address only if it is already a user,
/// is named as a bootstrap admin, or holds a redeemable invite — and a manifest
/// that names nobody satisfies none of the three, so a company created without
/// this is a company nobody on this machine can enter (issue #632).
///
/// The address is local-only by construction: a desktop install has no mail
/// transport, so it exists purely for the loopback magic link the shell
/// redeems. It never leaves the device and grants a network caller nothing.
fn first_run_manifest(preset_id: &str) -> Result<CompanyManifest> {
    let preset = preset(preset_id).ok_or_else(|| {
        crate::OpenCompanyError::Config(format!("unknown desktop preset `{preset_id}`"))
    })?;
    let mut manifest: CompanyManifest = toml::from_str(preset.manifest).map_err(|error| {
        crate::OpenCompanyError::Config(format!(
            "bundled desktop preset `{preset_id}` is invalid: {error}"
        ))
    })?;
    if !manifest
        .users
        .admins
        .iter()
        .any(|email| email == DESKTOP_OPERATOR_EMAIL)
    {
        manifest
            .users
            .admins
            .push(DESKTOP_OPERATOR_EMAIL.to_string());
    }
    Ok(manifest)
}

/// Registers every company this data root already holds, seeding the first-run
/// company when it holds none. Returns the registered ids, in listing order.
///
/// ## Why an embedded host has to do this itself
///
/// A company reaches the registry one of two ways: `serve --company <dir>`
/// names one on the command line, or the hosting control plane provisions one
/// over `POST /api/v1/companies`. A packaged desktop has neither — nobody types
/// a flag at a double-clicked application, and provisioning demands the
/// `platform` scope, which `PlatformScope` grants only against a configured
/// `platform_auth` that a prosumer host deliberately does not have.
///
/// So the desktop booted an empty registry, and an empty registry cannot be
/// signed into: sign-in is per-company (`/api/v1/companies/{id}/auth/login`,
/// or the sole-company alias), which leaves a fresh install with a login form
/// addressing a company that does not exist and no way to create one. That is
/// issue #632, and this function is the missing step.
///
/// ## Adoption before seeding
///
/// The persisted bundles are read first and the preset is a *fallback*, because
/// seeding unconditionally would hand the operator a second copy of the starter
/// company on every launch, and skipping adoption would make the one they
/// already have unreachable. The bundle is the only authority here: a desktop
/// company has no source directory to re-read, so what the store wrote at the
/// last shutdown — including console-created desks, agents and workflows, which
/// `RuntimeBuilder::build` carries forward from the persisted record — is what
/// comes back.
///
/// An `archived` company is skipped. Archiving removes a company from the
/// registry on purpose (`src/server/provision.rs`), and re-registering it at
/// the next launch would undo that quietly.
pub async fn bootstrap_companies(
    state: &AppState,
    preset_id: &str,
) -> Result<Vec<crate::ports::types::CompanyId>> {
    let store: std::sync::Arc<dyn crate::ports::CompanyStore> = match state.stores() {
        Some(handles) => handles.company.clone(),
        None => std::sync::Arc::new(crate::store::FsCompanyStore::new(
            state.home().to_path_buf(),
        )),
    };

    let mut registered = Vec::new();
    for summary in store.list().await? {
        if summary.lifecycle == "archived" {
            continue;
        }
        // One unreadable bundle must not cost the operator every other company
        // on the machine, so this warns and moves on rather than failing the
        // boot — the load as much as the build below. The seed further down is
        // what still runs if *nothing* could be adopted.
        let record = match store.load(&summary.id).await {
            Ok(Some(record)) => record,
            // Listed a moment ago and gone now: a bundle removed under us.
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(company = %summary.id, %error, "could not read a stored company");
                continue;
            }
        };
        match register(state, summary.id.clone(), record.manifest, None).await {
            Ok(()) => registered.push(summary.id),
            Err(error) => {
                tracing::warn!(company = %summary.id, %error, "could not adopt a stored company");
            }
        }
    }
    if !registered.is_empty() {
        return Ok(registered);
    }

    let manifest = first_run_manifest(preset_id)?;
    let id = company_id_from_name(&manifest.company.name);
    // Issue #85: record which template this install started from. Only the
    // slug, never a host path — there is no source directory on a packaged
    // install, and provenance is exposed verbatim on the API surfaces.
    let provenance = crate::ports::types::TemplateProvenance {
        source_id: preset_id.to_string(),
        version: None,
        path: None,
    };
    register(state, id.clone(), manifest, Some(provenance)).await?;
    tracing::info!(company = %id, preset = preset_id, "seeded the first-run company");
    Ok(vec![id])
}

/// Builds one company over the instance home and puts it in the registry.
async fn register(
    state: &AppState,
    id: crate::ports::types::CompanyId,
    manifest: CompanyManifest,
    provenance: Option<crate::ports::types::TemplateProvenance>,
) -> Result<()> {
    let mut builder = RuntimeBuilder::new(state.home().to_path_buf(), manifest).with_id(id.clone());
    if let Some(stores) = state.stores() {
        builder = builder.with_stores(stores);
    }
    if let Some(provenance) = provenance {
        builder = builder.with_template_provenance(provenance);
    }
    let runtime = builder.build().await?;
    // The same refusal `serve` applies at boot and provisioning: a `none`-mode
    // company on a routable bind is an unauthenticated admin console. The
    // desktop app always binds loopback (`start_local` below), so this is
    // unreachable today — kept anyway so a future change to that bind cannot
    // silently reintroduce the gap on this, the third company-registration
    // path.
    if !runtime.auth_mode().has_login() && !state.config().is_local_only() {
        return Err(crate::OpenCompanyError::Config(format!(
            "company `{}` is configured with `[users].mode = \"none\"`, which has no sign-in, \
             but this host binds `{}` and would serve it to anyone who can reach that address.",
            runtime.id().as_ref(),
            state.config().bind,
        )));
    }
    state.registry().insert(id, std::sync::Arc::new(runtime));
    Ok(())
}

/// Starts an offline, loopback-only runtime from a bundled preset.
pub async fn start_local(home: impl Into<PathBuf>, preset_id: &str) -> Result<DesktopRuntime> {
    let manifest = first_run_manifest(preset_id)?;

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address: SocketAddr = listener.local_addr()?;
    let company_id = company_id_from_name(&manifest.company.name);
    let state = AppState::new(AppConfig {
        bind: address.to_string(),
        ..AppConfig::default()
    })
    .with_home(home.into())
    .with_cors(CorsConfig {
        allowed_origins: vec![TAURI_WEBVIEW_ORIGIN.to_string()],
    });
    let runtime = RuntimeBuilder::new(state.home().to_path_buf(), manifest)
        .with_id(company_id.clone())
        .build()
        .await?;
    state
        .registry()
        .insert(company_id.clone(), std::sync::Arc::new(runtime));

    let app = crate::server::router(state);
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(DesktopRuntime {
        config: DesktopConfig {
            api_url: format!("http://{address}"),
            company: company_id.as_ref().to_string(),
            operator_email: DESKTOP_OPERATOR_EMAIL,
        },
        server,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ports::CompanyStore;

    #[test]
    fn ships_the_full_company_template_catalog() {
        assert_eq!(PRESETS.len(), 19);
        assert_eq!(
            preset(DEFAULT_PRESET_ID).unwrap().name,
            "Agentic Marketing Agency"
        );
        assert!(PRESETS.iter().all(|preset| !preset.manifest.is_empty()));
    }

    /// No shipped template narrows Composio to a hand-written toolkit list.
    ///
    /// Absent means open mode: the host answers with the backend's live catalog,
    /// which is every toolkit it permits. A non-empty list is authoritative and
    /// offered verbatim — the catalog is not consulted and nothing may widen it —
    /// so declaring one here caps both the agent belt and the Connections tab at
    /// whatever was typed, for every operator who starts from that template.
    ///
    /// `agentic_software_company` briefly carried such a list, added to work
    /// around a console that rendered zero provider rows for an empty one
    /// (#397). That root cause is fixed, so a list added here now buys nothing
    /// and silently restores the cap. Narrowing a template is a legitimate
    /// choice — it is just one that has to be made deliberately, which is what
    /// this test forces (#550).
    #[test]
    fn no_shipped_template_caps_the_composio_toolkit_list() {
        for preset in PRESETS {
            let manifest: toml::Value = toml::from_str(preset.manifest)
                .unwrap_or_else(|e| panic!("{} manifest does not parse: {e}", preset.id));
            let declared = manifest
                .get("tools")
                .and_then(|tools| tools.get("composio"))
                .and_then(|composio| composio.get("toolkits"));
            assert!(
                declared.is_none(),
                "{} declares [tools.composio].toolkits = {:?}, which pins its \
                 Connections tab to that list instead of the backend's catalog. \
                 Remove it, or narrow this template on purpose and say why here.",
                preset.id,
                declared.unwrap(),
            );
        }
    }

    #[tokio::test]
    async fn starts_a_loopback_runtime_from_the_default_preset() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = start_local(directory.path(), DEFAULT_PRESET_ID)
            .await
            .unwrap();
        assert!(runtime.config().api_url.starts_with("http://127.0.0.1:"));
        assert_eq!(runtime.config().operator_email, DESKTOP_OPERATOR_EMAIL);
    }

    /// A state over `home`, as an embedded host builds one.
    fn state_over(home: &std::path::Path) -> AppState {
        AppState::new(AppConfig::default()).with_home(home.to_path_buf())
    }

    fn store_over(home: &std::path::Path) -> crate::store::FsCompanyStore {
        crate::store::FsCompanyStore::new(home.to_path_buf())
    }

    /// Issue #632: a fresh install must end up somewhere a person can sign in.
    ///
    /// Both halves matter. A company nobody is eligible for is the same dead end
    /// as no company at all — the login form renders, the 202 comes back, and no
    /// code is ever minted.
    #[tokio::test]
    async fn a_first_run_seeds_a_company_the_local_operator_can_enter() {
        let directory = tempfile::tempdir().unwrap();
        let state = state_over(directory.path());

        let ids = bootstrap_companies(&state, DEFAULT_PRESET_ID)
            .await
            .expect("a fresh root bootstraps");

        assert_eq!(ids.len(), 1, "one starter company, not a fleet");
        let runtime = state
            .registry()
            .sole()
            .expect("the seeded company is registered, not merely written");
        let record = runtime
            .store()
            .load(runtime.id())
            .await
            .unwrap()
            .expect("the seeded company persists");
        assert!(
            record
                .manifest
                .users
                .admins
                .iter()
                .any(|email| email == DESKTOP_OPERATOR_EMAIL),
            "the seeded company must name somebody eligible: {:?}",
            record.manifest.users.admins
        );
        assert_eq!(
            record.template_provenance.map(|p| p.source_id),
            Some(DEFAULT_PRESET_ID.to_string()),
            "the install records which template it started from"
        );
    }

    /// The second launch is the one that would go wrong quietly: seeding again
    /// hands the operator a duplicate starter company, and every launch after
    /// that another.
    #[tokio::test]
    async fn a_later_launch_adopts_what_the_root_already_holds() {
        let directory = tempfile::tempdir().unwrap();
        let first = bootstrap_companies(&state_over(directory.path()), DEFAULT_PRESET_ID)
            .await
            .unwrap();

        // A wholly fresh state, as a relaunched application has.
        let relaunched = state_over(directory.path());
        let second = bootstrap_companies(&relaunched, DEFAULT_PRESET_ID)
            .await
            .unwrap();

        assert_eq!(first, second, "the same company comes back");
        assert_eq!(
            std::fs::read_dir(directory.path().join("companies"))
                .unwrap()
                .count(),
            1,
            "and no second bundle was written"
        );
        assert!(relaunched.registry().sole().is_some());
    }

    /// A damaged bundle costs its own company and nothing else.
    ///
    /// The failure this rules out is the whole-desktop one: a boot that gave up
    /// on the first unreadable bundle would leave an operator with no local host
    /// at all — not even the companies that are perfectly intact — and the
    /// console renders that as "no embedded host", which reads like the app is
    /// broken rather than like one company is.
    #[tokio::test]
    async fn a_damaged_bundle_does_not_take_the_healthy_one_with_it() {
        let directory = tempfile::tempdir().unwrap();
        let state = state_over(directory.path());
        let healthy = first_run_manifest(DEFAULT_PRESET_ID).unwrap();
        let healthy_id = company_id_from_name(&healthy.company.name);
        register(&state, healthy_id.clone(), healthy, None)
            .await
            .unwrap();

        let damaged = directory.path().join("companies").join("broken");
        std::fs::create_dir_all(&damaged).unwrap();
        std::fs::write(
            damaged.join("company.toml"),
            "this is not manifest = toml {{",
        )
        .unwrap();

        let relaunched = state_over(directory.path());
        let ids = bootstrap_companies(&relaunched, DEFAULT_PRESET_ID)
            .await
            .expect("a damaged bundle must not fail the boot");

        assert_eq!(ids, vec![healthy_id], "the intact company still comes up");
        assert!(
            relaunched.registry().sole().is_some(),
            "and it is the only one, rather than joined by a second starter"
        );
    }

    /// Archiving removes a company from the registry deliberately. A boot that
    /// re-registered every bundle on disk would undo that at the next launch,
    /// silently, and the operator would find it back in the picker.
    #[tokio::test]
    async fn an_archived_company_is_not_brought_back() {
        let directory = tempfile::tempdir().unwrap();
        let state = state_over(directory.path());
        bootstrap_companies(&state, DEFAULT_PRESET_ID)
            .await
            .unwrap();
        // A second company, so the archived one is skipped rather than merely
        // replaced by the first-run seed.
        let kept = first_run_manifest("agentic_law_firm").unwrap();
        let kept_id = company_id_from_name(&kept.company.name);
        register(&state, kept_id.clone(), kept, None).await.unwrap();

        let store = store_over(directory.path());
        let archived_id =
            company_id_from_name(&first_run_manifest(DEFAULT_PRESET_ID).unwrap().company.name);
        let mut record = store.load(&archived_id).await.unwrap().unwrap();
        record.lifecycle = "archived".to_string();
        store.save(&record).await.unwrap();

        let relaunched = state_over(directory.path());
        let ids = bootstrap_companies(&relaunched, DEFAULT_PRESET_ID)
            .await
            .unwrap();

        assert_eq!(ids, vec![kept_id]);
        assert!(
            relaunched.registry().get(&archived_id).is_none(),
            "an archived company must stay unaddressable"
        );
    }
}
