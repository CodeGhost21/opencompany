//! Whether a company's teammates can think, and — when they cannot — which of
//! the two reasons it is (issue #1735).
//!
//! The host already knows this and never said it. `/setup` reports
//! `acp_in_build`, `harness_in_build`, `mcp_in_build` and `oauth_in_build`;
//! `…/capabilities` reports `media_in_build`, `search_in_build`,
//! `publish_in_build` and `mcp_in_build`. None of them describes cognition, so
//! the console had no way to tell a considered reply from
//! [`EchoBrain`](crate::brain::EchoBrain)'s `"You said: …"` — and chat rendered
//! the echo under the teammate's own avatar and name (issue #1734).
//!
//! This is deliberately **not** a fifth `*_in_build` boolean. Cognition is two
//! facts at once — whether the harness is compiled in, and whether a model is
//! configured at runtime — and only the second is something an operator can act
//! on without a rebuild. A single flag collapses them, which sends the operator
//! who needs one settings page off looking for a new binary.

use serde::Serialize;

/// Whether this company's teammates can think, and why not when they cannot.
///
/// **Derived, never stored.** [`cognition_state`] reads the brain the runtime is
/// actually holding and the feature set this binary was compiled with, so there
/// is no second copy of the answer to drift from the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CognitionState {
    /// A cognition path that runs a real model is live — the harness, the
    /// hosted Medulla brain, a sidecar, or a brain the embedder injected.
    ///
    /// Says nothing about whether that provider will answer: reachability is
    /// what `POST …/inference/test` probes. This is the narrower question of
    /// whether anything but the echo brain is in the socket.
    Configured,
    /// This build carries the agent harness, but the company resolved no
    /// inference source at boot, so it is running the offline echo brain and
    /// answering every message with a canned line.
    ///
    /// Fixable in the app, at Settings → Inference. This is the state a fresh
    /// instance starts in, and the one the operator is most likely to mistake
    /// for the product being stupid.
    Unconfigured,
    /// The agent harness is not compiled into this binary, so no model
    /// configuration reaches one. Only a rebuild changes this.
    ///
    /// The console must say so plainly rather than offering a settings link
    /// that cannot help — the same rule `api/setup.ts` states for the
    /// `*_in_build` flags, which exist "so the flow can say 'not in this build'
    /// instead of offering a switch that does nothing".
    NotInBuild,
}

impl CognitionState {
    /// The stable wire label, for tests and diagnostics that would otherwise
    /// re-spell the serde renaming by hand.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Unconfigured => "unconfigured",
            Self::NotInBuild => "not-in-build",
        }
    }
}

/// Derives [`CognitionState`] from the brain a runtime actually holds and the
/// feature set this binary carries.
///
/// `path` is [`Cognition::path`](crate::ports::brain::Cognition::path) — read
/// off the live brain, not off the stored config, because a config that
/// resolves is not the same fact as a brain that was built from it. A company
/// configured after boot keeps the brain it started with until its runtime is
/// rebuilt, and reporting the config would tell that operator their teammates
/// can think while they are still being echoed at.
///
/// `harness_in_build` is `cfg!(feature = "openhuman")` at the call site. Passed
/// in rather than read here so the three-way matrix is testable without a
/// runtime per arm — in particular the `not-in-build` arm, which a lane that
/// enables the feature could otherwise never reach, and which would then be the
/// one arm with no coverage in the lane that ships.
///
/// The echo brain is the only path that runs no model
/// ([`ECHO_PATH`](crate::ports::brain::ECHO_PATH)), so every other label —
/// `harness`, `hosted`, `sidecar`, `custom` — is cognition of some kind and
/// reports [`CognitionState::Configured`]. Matching on the one degraded path
/// rather than allow-listing the working ones is what keeps a brain added later
/// from defaulting to "cannot think".
pub fn cognition_state(path: &str, harness_in_build: bool) -> CognitionState {
    if path != crate::ports::brain::ECHO_PATH {
        return CognitionState::Configured;
    }
    if harness_in_build {
        CognitionState::Unconfigured
    } else {
        CognitionState::NotInBuild
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ports::brain::{ECHO_PATH, HARNESS_PATH};

    /// The whole matrix, both feature settings, in one place — including the
    /// `not-in-build` arm that the `openhuman` lane cannot otherwise reach.
    #[test]
    fn the_three_states_are_derived_from_the_path_and_the_build() {
        assert_eq!(
            cognition_state(HARNESS_PATH, true),
            CognitionState::Configured,
        );
        assert_eq!(cognition_state("hosted", true), CognitionState::Configured);
        assert_eq!(cognition_state("sidecar", true), CognitionState::Configured);
        assert_eq!(cognition_state("custom", true), CognitionState::Configured);
        assert_eq!(
            cognition_state(ECHO_PATH, true),
            CognitionState::Unconfigured,
            "the harness is compiled in, so a provider is one settings page away",
        );
        assert_eq!(
            cognition_state(ECHO_PATH, false),
            CognitionState::NotInBuild,
            "no harness in this binary: no configuration reaches a model",
        );
    }

    /// The regression this exists for: a build with no harness, sitting on the
    /// echo brain, must never report itself as able to think. That is the state
    /// the console renders `"You said: …"` under a teammate's name in.
    #[test]
    fn a_build_with_no_harness_never_reports_itself_configured() {
        assert_ne!(
            cognition_state(ECHO_PATH, false),
            CognitionState::Configured,
        );
        assert_ne!(cognition_state(ECHO_PATH, true), CognitionState::Configured,);
    }

    /// A path this module has never heard of is cognition until proven
    /// otherwise. The alternative — allow-listing the working paths — would
    /// report "cannot think" for the next brain someone adds, and the symptom
    /// (a banner on a company that is thinking perfectly well) points nowhere
    /// near this function.
    #[test]
    fn an_unknown_path_is_treated_as_cognition() {
        assert_eq!(
            cognition_state("some-brain-added-later", false),
            CognitionState::Configured,
        );
    }

    #[test]
    fn the_wire_labels_match_the_serde_renaming() {
        for state in [
            CognitionState::Configured,
            CognitionState::Unconfigured,
            CognitionState::NotInBuild,
        ] {
            assert_eq!(
                serde_json::to_value(state).expect("serialize"),
                serde_json::Value::String(state.as_str().to_string()),
            );
        }
    }
}
