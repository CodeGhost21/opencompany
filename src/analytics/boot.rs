//! Wiring analytics into a booting host: one function, called once.
//!
//! Kept out of `src/bin/opencompany.rs` so it is testable. The binary's `serve`
//! arm is ~200 lines of sequencing that nothing else runs, and a decision this
//! consequential — whether a GPL-3.0 install reports — should not only be
//! exercised by starting a real server.

use std::sync::Arc;

use crate::analytics::config::{Decision, resolve};
use crate::analytics::types::{Envelope, OpaqueId};
use crate::analytics::{DeferredTracker, Event, Tracker, mixpanel};
use crate::app::AppState;
use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// Chooses this process's tracker, installs it behind `handle`, and reports
/// `instance_started`.
///
/// Called **after** the host's companies are registered, for two reasons that
/// both come down to reporting what is true rather than what was configured:
///
/// * the context envelope names the cognition path, and that is a property of
///   the brain a runtime *builds* — see [`DeferredTracker`] for why the handle
///   is handed out before the tracker exists;
/// * `instance_started` carries the company count, which is not known until
///   they have been registered or adopted.
///
/// Returns the decision so the caller can say, in one line, why a host that an
/// operator expected to report is not reporting.
pub fn install(state: &AppState, handle: &DeferredTracker, env: &dyn EnvSource) -> Decision {
    let deployment = Deployment::from_env(env);
    let decision = resolve(deployment, env);

    // Identity, in the order #1739's decision 2 sets out: the tenant slug
    // (hashed) when the platform named one, else this host's own random
    // instance id. Never the company name, and never anything derived from the
    // hostname or the bind address — `crate::app::instance` argues that at
    // length for the same id on the same grounds.
    let id = match state.config().tenant_namespace.as_deref() {
        Some(tenant) => OpaqueId::tenant(crate::app::canonical_tenant(tenant)),
        None => OpaqueId::instance(state.instance_id()),
    };

    // The cognition seam the rest of the tree already uses, rather than a second
    // derivation of "which brain is this host on?" beside the code that picks
    // one. Every company on a host shares its brain mode, so the first
    // registered runtime answers for all of them; a host with no companies yet
    // reports the default descriptor, which honestly says `custom`/`unknown`.
    let cognition = state
        .registry()
        .list()
        .first()
        .and_then(|id| state.registry().get(id))
        .map(|runtime| runtime.cognition())
        .unwrap_or_default();

    let envelope = Envelope::new(id, deployment, cognition);
    let tracker: Arc<dyn Tracker> = mixpanel::build(&decision, envelope);
    handle.install(tracker);

    handle.track(Event::InstanceStarted {
        companies: state.registry().list().len() as u64,
        storage: state.storage_kind().as_str(),
        setup_complete: state.setup_complete() || !state.registry().is_empty(),
    });

    decision
}

/// The one line a boot log carries about analytics.
///
/// Said out loud on purpose. Silence is the correct default, but a *silent*
/// default is how an operator spends an afternoon on a tenant that was never
/// going to report — and, in the other direction, a hosted tenant's operator is
/// entitled to see in their own logs that reporting is on.
pub fn describe(decision: &Decision) -> String {
    match decision {
        Decision::Silent(reason) => {
            format!("analytics: off ({})", reason.as_str())
        }
        Decision::Report { endpoint, .. } => {
            // The endpoint, never the token.
            format!("analytics: reporting to {endpoint}")
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::analytics::config::{ENABLE_ENV, Silence, TOKEN_ENV};
    use crate::app::config::MapEnv;
    use crate::app::deployment::DEPLOYMENT_ENV;
    use crate::{AppConfig, AppState};

    fn state() -> (AppState, tempfile::TempDir) {
        let home = tempfile::tempdir().expect("tempdir");
        let state = AppState::new(AppConfig::default()).with_home(home.path());
        (state, home)
    }

    /// **The default posture, asserted end to end at the boot seam.** A host
    /// that says nothing installs a tracker that sends nothing, even with a
    /// token sitting in its environment.
    #[test]
    fn an_undeclared_host_installs_silence() {
        let (state, _home) = state();
        let handle = DeferredTracker::new();
        let decision = install(
            &state,
            &handle,
            &MapEnv::new([(TOKEN_ENV, "not-a-real-token")]),
        );
        assert_eq!(decision, Decision::Silent(Silence::NotHosted));
        assert_eq!(
            describe(&decision),
            "analytics: off (not a hosted tenant and no explicit opt-in)"
        );
    }

    /// A hosted tenant resolves to reporting. In a build without the `analytics`
    /// feature the installed tracker is still a no-op — the decision is the same
    /// either way, which is what this pins.
    #[test]
    fn a_hosted_tenant_resolves_to_reporting() {
        let (state, _home) = state();
        let handle = DeferredTracker::new();
        let decision = install(
            &state,
            &handle,
            &MapEnv::new([
                (DEPLOYMENT_ENV, "hosted-tenant"),
                (TOKEN_ENV, "not-a-real-token"),
            ]),
        );
        assert!(decision.reports(), "{decision:?}");
        assert!(
            !describe(&decision).contains("not-a-real-token"),
            "the boot line must not carry the token: {}",
            describe(&decision)
        );
    }

    /// The instance id is what a host with no tenant namespace is known by, and
    /// it is prefixed so it can never be confused with a tenant digest.
    #[test]
    fn an_untenanted_host_is_known_by_its_instance_id() {
        let (state, _home) = state();
        let expected = OpaqueId::instance(state.instance_id());
        assert!(expected.as_str().starts_with("i_"));
        assert!(expected.as_str().contains(state.instance_id()));
    }

    #[test]
    fn an_opted_out_host_says_why() {
        let (state, _home) = state();
        let handle = DeferredTracker::new();
        let decision = install(
            &state,
            &handle,
            &MapEnv::new([
                (DEPLOYMENT_ENV, "hosted-tenant"),
                (ENABLE_ENV, "off"),
                (TOKEN_ENV, "not-a-real-token"),
            ]),
        );
        assert_eq!(decision, Decision::Silent(Silence::OptedOut));
        assert!(describe(&decision).contains("operator opted out"));
    }
}
