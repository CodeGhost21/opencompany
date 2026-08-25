//! [`Deployment`]: which *kind* of install this process is.
//!
//! Three deployments run this same binary and they are not interchangeable:
//! a tenant container the hosting platform provisioned, an operator's own
//! self-hosted server, and the desktop app's embedded host. Behaviour that is
//! correct for one is wrong for another — most sharply for analytics
//! (`docs/spec/runtime/analytics.md`), where a hosted tenant reporting to the
//! platform that runs it is ordinary operations and a self-hosted GPL install
//! doing the same thing is a betrayal.
//!
//! ## Why this is declared, not sniffed
//!
//! It would be easy to infer the kind from something already present —
//! `harness_in_build` differs between the desktop and the server today, the
//! data dir is `/data` in a container, the bind address is `0.0.0.0`. Every one
//! of those is a coincidence that inverts the day someone changes an unrelated
//! setting: the desktop enables the harness, an operator mounts `/data`, a
//! self-hoster binds all interfaces behind their own proxy. A discriminator
//! whose meaning depends on an unrelated decision is worse than none, because
//! the failure is silent and points at the wrong file.
//!
//! So it is **declared** by whoever launches the process, through
//! `OPENCOMPANY_DEPLOYMENT`, and the default is the one that is safe to be
//! wrong about: [`Deployment::SelfHosted`], which sends nothing.
//!
//! One inference is allowed, and only one: `OPENCOMPANY_TENANT_ID` is injected
//! by the control plane and by nothing else (`CLAUDE.md`, shared-single-DB
//! mode), so its presence names a hosted tenant. It is a fallback for tenants
//! whose manager predates `OPENCOMPANY_DEPLOYMENT`, not the primary signal —
//! db-per-tenant tenants do not set it, which is exactly why it cannot be the
//! only answer.

use crate::app::config::EnvSource;

/// The environment variable that declares the deployment kind.
pub const DEPLOYMENT_ENV: &str = "OPENCOMPANY_DEPLOYMENT";

/// The variable the control plane injects in shared-single-DB mode. Read here
/// only as a fallback signal for "this is a hosted tenant".
const TENANT_ENV: &str = "OPENCOMPANY_TENANT_ID";

/// Which kind of install this process is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Deployment {
    /// The desktop app's embedded host. One human, on their own machine.
    Desktop,
    /// An operator running this GPL-3.0 crate on their own infrastructure.
    /// **The default**, because it is the kind that must never phone home by
    /// accident.
    #[default]
    SelfHosted,
    /// A per-tenant container the OpenCompany hosting platform provisioned and
    /// operates.
    HostedTenant,
}

impl Deployment {
    /// The stable slug for this kind. `&'static str` on purpose: it is a
    /// telemetry property, and every analytics property value in this crate is
    /// a compile-time constant so that no caller-supplied text can reach one.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::SelfHosted => "self-hosted",
            Self::HostedTenant => "hosted-tenant",
        }
    }

    /// Parses a declared slug. Unknown text is **not** an error and **not** a
    /// guess: it resolves to [`Self::SelfHosted`], the silent default. A typo in
    /// a launcher must not upgrade an install into one that reports.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "desktop" => Self::Desktop,
            "hosted-tenant" | "hosted_tenant" | "hosted" => Self::HostedTenant,
            _ => Self::SelfHosted,
        }
    }

    /// Resolves the deployment kind from the environment.
    ///
    /// `OPENCOMPANY_DEPLOYMENT` wins outright. Failing that, a tenant namespace
    /// names a hosted tenant (see the module docs for why that inference is the
    /// only one taken). Everything else is self-hosted.
    pub fn from_env(env: &dyn EnvSource) -> Self {
        if let Some(declared) = env.get(DEPLOYMENT_ENV) {
            return Self::parse(&declared);
        }
        if env.get(TENANT_ENV).is_some() {
            return Self::HostedTenant;
        }
        Self::SelfHosted
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    /// The load-bearing default. Everything about the analytics posture rests on
    /// an unconfigured process being self-hosted, so this is pinned rather than
    /// left to `#[derive(Default)]` being read correctly by the next person.
    #[test]
    fn an_undeclared_deployment_is_self_hosted() {
        assert_eq!(
            Deployment::from_env(&MapEnv::default()),
            Deployment::SelfHosted
        );
        assert_eq!(Deployment::default(), Deployment::SelfHosted);
    }

    #[test]
    fn a_declaration_wins_over_the_tenant_inference() {
        let env = MapEnv::new([
            ("OPENCOMPANY_DEPLOYMENT", "desktop"),
            ("OPENCOMPANY_TENANT_ID", "acme"),
        ]);
        assert_eq!(Deployment::from_env(&env), Deployment::Desktop);
    }

    #[test]
    fn a_tenant_namespace_names_a_hosted_tenant() {
        let env = MapEnv::new([("OPENCOMPANY_TENANT_ID", "acme")]);
        assert_eq!(Deployment::from_env(&env), Deployment::HostedTenant);
    }

    /// A typo must fall to silence, never to reporting. The dangerous direction
    /// is the only one worth a test.
    #[test]
    fn an_unrecognised_declaration_falls_back_to_silence() {
        let env = MapEnv::new([("OPENCOMPANY_DEPLOYMENT", "hosted-tenat")]);
        assert_eq!(Deployment::from_env(&env), Deployment::SelfHosted);
    }

    #[test]
    fn every_kind_has_a_stable_slug() {
        assert_eq!(Deployment::Desktop.as_str(), "desktop");
        assert_eq!(Deployment::SelfHosted.as_str(), "self-hosted");
        assert_eq!(Deployment::HostedTenant.as_str(), "hosted-tenant");
    }
}
