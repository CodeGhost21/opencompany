//! Shared turn-failure and save-refusal sentences (decision D-copy / X9,
//! 2026-09-15, `docs/key-reworks/README.md`).
//!
//! Before this module, near-identical fail-closed sentences were written out
//! by hand at each call site (`resolve_choice` in `inference.rs`, the agent
//! pin check in `harness/built_in/provider.rs`, the provider save routes in
//! `server/ops/inference/providers.rs` and `server/ops/team_agent.rs`), and
//! nothing kept them in agreement — two sites saying the same thing slightly
//! differently reads, to an operator hopping between an agent's error and the
//! company default's error, as two different products. One function per
//! sentence, called from every site that needs it, is the fix; the tests
//! below are what keep it a fix rather than a fourth copy.
//!
//! D-names-in-errors (X7) is why every function here takes a **display
//! name** — an agent's `role`/label, a provider's `label` — never a raw slug
//! or agent id. The id still rides in whatever structured data the caller
//! attaches (an `AgentPin` carries both); only the sentence a person reads is
//! restricted to names.

/// Why a provider a pin or the default named cannot serve a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderGone {
    /// No row with that slug exists any more (deleted, or never added).
    Removed,
    /// The row exists but its `enabled` flag is off.
    TurnedOff,
}

impl ProviderGone {
    fn word(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::TurnedOff => "turned off",
        }
    }
}

/// A provider save (add, edit, or set-default) sent no model.
///
/// "Choose a model for {Provider} before saving."
pub fn no_model_for_provider(provider_label: &str) -> String {
    format!("Choose a model for {provider_label} before saving.")
}

/// The console path every sentence below points to: the LLM page, under the
/// "API Keys" group of Connections (`frontend/src/views/connection-pages.ts`:
/// group `keys` is labelled "API Keys"; the `inference` page in it is
/// labelled "LLM" — verified against that file, not guessed).
const SETTINGS_PATH: &str = "Connections → API Keys → LLM";

/// Nothing resolves for this agent's turn: no pin, no full company default,
/// and the legacy chain gave nothing either.
///
/// "No model is chosen. Choose a provider and model for {Agent}, or set the
/// company default in Connections → API Keys → LLM."
pub fn nothing_resolved(agent_name: &str) -> String {
    format!(
        "No model is chosen. Choose a provider and model for {agent_name}, \
         or set the company default in {SETTINGS_PATH}."
    )
}

/// No agent is in view (a company-wide boot/status read, or an internal pass
/// with no single agent to name) and nothing resolves.
///
/// "No model is chosen for this company. Choose a default provider and model
/// in Connections → API Keys → LLM."
///
/// A `const` because `inference::NO_MODEL_CHOSEN` (2b's originally-named
/// symbol, kept for callers outside this module that still match error text
/// against it) needs a `&'static str` it can re-export, not a function call.
pub const COMPANY_NO_MODEL_CHOSEN: &str = "No model is chosen for this company. Choose a default \
     provider and model in Connections → API Keys → LLM.";

/// Same text as [`COMPANY_NO_MODEL_CHOSEN`], as an owned `String` for callers
/// building an [`crate::error::OpenCompanyError`] (which takes `String`).
pub fn nothing_resolved_for_company() -> String {
    COMPANY_NO_MODEL_CHOSEN.to_string()
}

/// A provider that would otherwise resolve has no credential.
///
/// "{Agent} uses {Provider}, which has no key. Add one in Connections → API
/// Keys → LLM, or choose another provider and model for {Agent}."
pub fn provider_has_no_key(agent_name: &str, provider_label: &str) -> String {
    format!(
        "{agent_name} uses {provider_label}, which has no key. Add one in \
         {SETTINGS_PATH}, or choose another provider and model for {agent_name}."
    )
}

/// An agent's own pair names a provider that is gone or switched off (F6:
/// fails closed, never falls back to the company default on its own).
///
/// "{Agent} uses {Provider}, which is removed. Choose another provider and
/// model for {Agent}, or clear its model to use the company default."
pub fn pair_broken(agent_name: &str, provider_label: &str, why: ProviderGone) -> String {
    let word = why.word();
    format!(
        "{agent_name} uses {provider_label}, which is {word}. Choose another \
         provider and model for {agent_name}, or clear its model to use the \
         company default."
    )
}

/// The company default names a provider that is gone or switched off.
///
/// "The company default uses {Provider}, which is removed. Choose a new
/// default in Connections → API Keys → LLM."
pub fn default_broken(provider_label: &str, why: ProviderGone) -> String {
    let word = why.word();
    format!(
        "The company default uses {provider_label}, which is {word}. Choose \
         a new default in {SETTINGS_PATH}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_model_for_provider_names_the_provider() {
        assert_eq!(
            no_model_for_provider("TinyHumans"),
            "Choose a model for TinyHumans before saving."
        );
    }

    #[test]
    fn nothing_resolved_names_the_agent_and_the_settings_path() {
        assert_eq!(
            nothing_resolved("Researcher"),
            "No model is chosen. Choose a provider and model for Researcher, \
             or set the company default in Connections → API Keys → LLM."
        );
    }

    #[test]
    fn nothing_resolved_for_company_names_no_agent() {
        let text = nothing_resolved_for_company();
        assert!(text.starts_with("No model is chosen for this company."));
        assert!(text.contains("Connections → API Keys → LLM"));
    }

    #[test]
    fn provider_has_no_key_names_the_agent_provider_and_settings_path() {
        assert_eq!(
            provider_has_no_key("Researcher", "Anthropic"),
            "Researcher uses Anthropic, which has no key. Add one in \
             Connections → API Keys → LLM, or choose another provider and \
             model for Researcher."
        );
    }

    #[test]
    fn pair_broken_distinguishes_removed_from_turned_off() {
        assert_eq!(
            pair_broken("Researcher", "Anthropic", ProviderGone::Removed),
            "Researcher uses Anthropic, which is removed. Choose another \
             provider and model for Researcher, or clear its model to use \
             the company default."
        );
        assert_eq!(
            pair_broken("Researcher", "Anthropic", ProviderGone::TurnedOff),
            "Researcher uses Anthropic, which is turned off. Choose another \
             provider and model for Researcher, or clear its model to use \
             the company default."
        );
    }

    #[test]
    fn default_broken_names_the_provider_and_the_settings_path() {
        assert_eq!(
            default_broken("Anthropic", ProviderGone::Removed),
            "The company default uses Anthropic, which is removed. Choose a \
             new default in Connections → API Keys → LLM."
        );
    }
}
