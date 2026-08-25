//! The enable/disable decision: whether this process reports at all, and where.
//!
//! Kept apart from both the transport and the payload because it is the part
//! that has to be *provably* right. It is pure — an [`EnvSource`] and a
//! [`Deployment`] in, a [`Decision`] out — so every branch of it is tested in
//! the default build, with no network and no feature flag.

use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// Operator override: `on` forces reporting, `off` forbids it.
pub const ENABLE_ENV: &str = "OPENCOMPANY_ANALYTICS";
/// The Mixpanel project token. **Configuration, never a compiled-in constant** —
/// a token baked into a public binary is a token everyone has.
pub const TOKEN_ENV: &str = "OPENCOMPANY_ANALYTICS_TOKEN";
/// Overrides the collector URL. Exists so a test can point at a local server,
/// and so a deployment can front Mixpanel with its own proxy.
pub const ENDPOINT_ENV: &str = "OPENCOMPANY_ANALYTICS_ENDPOINT";

/// Where events go when nothing overrides it.
pub const DEFAULT_ENDPOINT: &str = "https://api.mixpanel.com/track";

/// A Mixpanel project token.
///
/// A newtype rather than a bare `String` for one reason: it must never be
/// printed, logged, or serialized by accident. It derives **neither** `Debug`
/// nor `Serialize` — the hand-written `Debug` redacts — because
/// `serde_json::to_value(&some_config)` is precisely how a credential reaches a
/// payload (issue #1741, `SecretValue`). Nothing in this module ever serializes
/// a config struct; the token is read out explicitly, once, at the moment a
/// request body is built.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectToken(String);

impl ProjectToken {
    /// Wraps a token read from configuration.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The token, for the one caller that puts it on the wire.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ProjectToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProjectToken(<redacted>)")
    }
}

/// Why a process is not reporting. Logged once at boot, so an operator who
/// *expected* analytics can tell "switched off" from "misconfigured".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    /// The operator set `OPENCOMPANY_ANALYTICS=off`.
    OptedOut,
    /// Not a hosted tenant, and nobody opted in. **The default.**
    NotHosted,
    /// Reporting was asked for, but no project token is configured.
    NoToken,
}

impl Silence {
    /// The stable reason slug, for the boot log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptedOut => "operator opted out",
            Self::NotHosted => "not a hosted tenant and no explicit opt-in",
            Self::NoToken => "no project token is configured",
        }
    }
}

/// What this process will do.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Send nothing. No client is constructed, so nothing *can* be sent.
    Silent(Silence),
    /// Report to `endpoint` under `token`.
    Report {
        /// The collector URL.
        endpoint: String,
        /// The project token.
        token: ProjectToken,
    },
}

impl Decision {
    /// Whether this decision reports.
    pub fn reports(&self) -> bool {
        matches!(self, Self::Report { .. })
    }
}

/// Resolves the decision.
///
/// The order matters and is the whole policy:
///
/// 1. `OPENCOMPANY_ANALYTICS=off` wins over everything. An operator switching
///    it off must not be overruled by a deployment kind, a token, or a future
///    default.
/// 2. Otherwise reporting is on **only** for [`Deployment::HostedTenant`], or
///    when an operator explicitly sets `OPENCOMPANY_ANALYTICS=on`. Decision 1
///    of #1739: silence is the default and reporting is the exception, so a
///    self-hosted or desktop install that has said nothing sends nothing.
/// 3. A token is required. Without one there is nowhere to report to, and
///    guessing is not an option — see [`TOKEN_ENV`].
pub fn resolve(deployment: Deployment, env: &dyn EnvSource) -> Decision {
    let switch = env.get(ENABLE_ENV).map(|v| v.trim().to_ascii_lowercase());

    match switch.as_deref() {
        Some("off" | "false" | "0" | "no") => return Decision::Silent(Silence::OptedOut),
        Some("on" | "true" | "1" | "yes") => {}
        // An unrecognised value is NOT an opt-in. The dangerous direction here
        // is "a typo turns reporting on", so anything that is not clearly `on`
        // falls through to the deployment default.
        _ => {
            if deployment != Deployment::HostedTenant {
                return Decision::Silent(Silence::NotHosted);
            }
        }
    }

    let Some(token) = env.get(TOKEN_ENV) else {
        return Decision::Silent(Silence::NoToken);
    };

    Decision::Report {
        endpoint: env
            .get(ENDPOINT_ENV)
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string()),
        token: ProjectToken::new(token),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    fn token_env(pairs: &[(&str, &str)]) -> MapEnv {
        let mut all = vec![(TOKEN_ENV, "not-a-real-token")];
        all.extend_from_slice(pairs);
        MapEnv::new(all)
    }

    /// **The decision the GPL posture rests on.** A self-hosted instance that
    /// has been handed a token — which is the easiest way to get this wrong,
    /// because a token looks like consent — still sends nothing.
    #[test]
    fn a_self_hosted_instance_is_silent_even_with_a_token() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &token_env(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_desktop_instance_is_silent_even_with_a_token() {
        assert_eq!(
            resolve(Deployment::Desktop, &token_env(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_hosted_tenant_with_a_token_reports() {
        let decision = resolve(Deployment::HostedTenant, &token_env(&[]));
        assert!(decision.reports(), "{decision:?}");
        match decision {
            Decision::Report { endpoint, token } => {
                assert_eq!(endpoint, DEFAULT_ENDPOINT);
                assert_eq!(token.expose(), "not-a-real-token");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A hosted tenant with no token is misconfigured, not reporting to
    /// nowhere — and the reason says which.
    #[test]
    fn a_hosted_tenant_without_a_token_is_silent() {
        assert_eq!(
            resolve(Deployment::HostedTenant, &MapEnv::default()),
            Decision::Silent(Silence::NoToken)
        );
    }

    /// `off` outranks the deployment kind. The platform can switch a tenant off
    /// without rebuilding it.
    #[test]
    fn off_outranks_a_hosted_deployment() {
        assert_eq!(
            resolve(Deployment::HostedTenant, &token_env(&[(ENABLE_ENV, "off")])),
            Decision::Silent(Silence::OptedOut)
        );
    }

    /// The self-hoster's opt-in, which is the only way a non-hosted install ever
    /// reports.
    #[test]
    fn a_self_hoster_can_opt_in() {
        assert!(resolve(Deployment::SelfHosted, &token_env(&[(ENABLE_ENV, "on")])).reports());
    }

    /// A typo must not opt anybody in. `on` is the only spelling of yes that
    /// this reads as yes, and everything else falls to the deployment default.
    #[test]
    fn a_misspelled_switch_does_not_opt_in() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &token_env(&[(ENABLE_ENV, "onn")])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    /// The token is a credential: it must not be printable by accident, because
    /// the accident is a `{:?}` in a log line nobody reviewed.
    #[test]
    fn a_token_is_not_printable() {
        let token = ProjectToken::new("not-a-real-token");
        let printed = format!("{token:?}");
        assert!(
            !printed.contains("not-a-real-token"),
            "the Debug impl leaked the token: {printed}"
        );

        let decision = Decision::Report {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            token,
        };
        let printed = format!("{decision:?}");
        assert!(
            !printed.contains("not-a-real-token"),
            "the Debug impl leaked the token through the decision: {printed}"
        );
    }
}
