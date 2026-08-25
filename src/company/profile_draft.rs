//! Issue #1776 — drafting **one** teammate's mandate or persona, for an
//! operator who then keeps it or throws it away.
//!
//! A teammate's `description` (the mandate: one line on what it owns) and
//! `instructions` (the persona appended to its system prompt) are the two
//! fields that decide how it behaves, and the two an operator has the least
//! help writing. This module holds the shape of a draft; the model call that
//! produces one lives in [`crate::harness::profile_draft`], behind the
//! `openhuman` feature.
//!
//! ## Why this is not the roster designer's rule being relaxed
//!
//! [`crate::company::setup`] deliberately keeps the model out of a teammate's
//! standing instructions: it names a work *shape* from a closed enum and the
//! host owns every word. That rule is untouched, and it must stay that way —
//! it governs teammates that are **created** from a design pass, where the text
//! reaches a system prompt with nobody having read it, through a route any
//! member can call.
//!
//! A draft is the opposite case in the one way that matters. It is returned to
//! the operator and stored by **nothing**: the route that produces it never
//! writes, the console shows it beside the field rather than in it, and the
//! text only becomes a persona if a person takes it and then saves. That is the
//! same stance the workflow copilot's proposal protocol takes — the model's
//! output is data in a reply, and the operator's own action is what writes.
//!
//! So the boundary this module holds is narrow and specific: **a draft is
//! bounded like the field it is for, and it is never applied here.**

use crate::company::prompt::cap_persona_instructions;
use crate::company::setup::clamp_description;

/// Which authored field a draft is for.
///
/// Only the two prose fields. `name` and `role` are short identity values an
/// operator picks in seconds — and `role` is what delegation grounds on, so a
/// drafted one would change who the company routes work to, which is not a
/// thing to hand a model on a screen whose whole promise is "this changes
/// nothing until you save".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    /// The one-line mandate, shown on the roster card.
    Description,
    /// The persona appended to this teammate's system prompt.
    Instructions,
}

impl ProfileField {
    /// The wire spelling, which is also the `PATCH` field name — the console
    /// asks for a draft of the field it is about to fill.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Instructions => "instructions",
        }
    }

    /// Reads a field off the wire. Anything else is `None`, so a request naming
    /// a field this pass does not draft is refused rather than silently
    /// answered about a different one.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "description" => Some(Self::Description),
            "instructions" => Some(Self::Instructions),
            _ => None,
        }
    }

    /// Brings a draft inside the bound the field itself obeys.
    ///
    /// Applied **host-side**, on the way out, so the console is not the only
    /// thing holding the limit — the same reason the roster pass clamps its own
    /// mandates rather than trusting the review screen to.
    ///
    /// The two bounds are different in kind, and each field gets its own:
    /// a mandate is clamped to [`MAX_DESCRIPTION`](crate::company::setup::MAX_DESCRIPTION)
    /// because the roster card has one line for it, while a persona is capped
    /// by prompt weight because it is read on every turn of that teammate.
    pub fn clamp(self, text: &str) -> String {
        match self {
            Self::Description => clamp_description(text),
            Self::Instructions => cap_persona_instructions(text.trim()),
        }
    }
}

/// Why no draft came back.
///
/// Three reasons rather than one, because **the operator's next move differs**
/// — the same split [`FallbackReason`](crate::company::setup::FallbackReason)
/// makes for the roster pass, and for the same reason: one sentence covering
/// all three can only be vague enough to be useless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftRefusal {
    /// Nothing was reachable, so no call ran. Wire up a model.
    NoModel,
    /// A model is wired and the call did not land — a timeout, an unreachable
    /// provider. Retry, or check the provider; adding a key would fix nothing.
    ModelUnreachable,
    /// A model answered and the answer could not be used. Say more in the hint,
    /// or write the field by hand.
    Unreadable,
}

impl DraftRefusal {
    /// The wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoModel => "no_model",
            Self::ModelUnreachable => "model_unreachable",
            Self::Unreadable => "unreadable",
        }
    }
}

/// One teammate a draft is about, plus everything the pass is allowed to see.
///
/// A closed set, assembled host-side from the company record. The console does
/// not compose it and cannot add to it: a draft is grounded in this teammate,
/// its siblings' roles, and what the operator typed into the hint — never in
/// the rest of the company.
#[derive(Debug, Clone, Default)]
pub struct ProfileSubject {
    /// The company's name, so a mandate reads as one of *this* company's.
    pub company_name: String,
    /// What the company produces (`[company].output`), when it declares it.
    pub company_output: Option<String>,
    /// The teammate's roster id.
    pub agent_id: String,
    /// Its name, when an operator has given it one.
    pub name: Option<String>,
    /// Its role — the one field a draft can always lean on.
    pub role: String,
    /// The mandate in force, so a redraft improves on it rather than ignoring
    /// it, and so a persona can be written to fit the job the card claims.
    pub description: Option<String>,
    /// The persona in force, for the same reason.
    pub instructions: Option<String>,
    /// The rest of the roster — **id and role only**.
    ///
    /// Named so a drafted mandate does not restate a sibling's. The delegation
    /// surface renders id and role and nothing else, so two teammates whose
    /// mandates overlap are two the company cannot tell apart when it comes to
    /// hand out work (issue #1162).
    pub siblings: Vec<Sibling>,
    /// What the operator typed into the hint box, when they typed anything.
    pub hint: Option<String>,
}

/// One other teammate on the roster, as a draft is told about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sibling {
    pub id: String,
    pub role: String,
}

/// What one drafting pass produced.
///
/// Either a draft or a reason there is none — never both, and never neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileDraft {
    /// Text for the operator to keep or throw away. Already clamped.
    Drafted(String),
    /// No draft, and why.
    Refused(DraftRefusal),
}

impl ProfileDraft {
    /// The drafted text, or `None` when the pass refused.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Drafted(text) => Some(text),
            Self::Refused(_) => None,
        }
    }

    /// The refusal, or `None` when a draft came back.
    pub fn refusal(&self) -> Option<DraftRefusal> {
        match self {
            Self::Drafted(_) => None,
            Self::Refused(reason) => Some(*reason),
        }
    }

    /// Builds the outcome for a model answer, clamping it for the field and
    /// treating a blank answer as unreadable.
    ///
    /// A model that answers with whitespace has technically replied, and
    /// handing that to the console as a draft would put an empty suggestion
    /// card on screen — which reads as "the copilot thinks your teammate needs
    /// no instructions" rather than as the failure it is.
    pub fn from_answer(field: ProfileField, answer: &str) -> Self {
        let clamped = field.clamp(answer);
        if clamped.trim().is_empty() {
            return Self::Refused(DraftRefusal::Unreadable);
        }
        Self::Drafted(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::company::setup::MAX_DESCRIPTION;

    #[test]
    fn only_the_two_prose_fields_are_draftable() {
        assert_eq!(
            ProfileField::parse("description"),
            Some(ProfileField::Description)
        );
        assert_eq!(
            ProfileField::parse(" instructions "),
            Some(ProfileField::Instructions)
        );
        for other in ["name", "role", "tools", "model", "", "Description"] {
            assert_eq!(ProfileField::parse(other), None, "{other}");
        }
    }

    /// The bound is the field's own, applied here rather than trusted to the
    /// console — a caller that is not our console gets the same clamp.
    #[test]
    fn a_long_mandate_is_clamped_to_the_card() {
        let long = "x ".repeat(MAX_DESCRIPTION);
        let draft = ProfileDraft::from_answer(ProfileField::Description, &long);
        let text = draft.text().expect("a long answer still drafts");
        assert!(
            text.chars().count() <= MAX_DESCRIPTION + 1,
            "clamped to the card: {} chars",
            text.chars().count()
        );
    }

    /// A persona is bounded by prompt weight, not by the card — the two limits
    /// are different in kind, so a persona well over the mandate bound survives.
    #[test]
    fn a_persona_is_not_clamped_to_the_mandate_bound() {
        let persona = "Confirm the budget before launching. ".repeat(20);
        let draft = ProfileDraft::from_answer(ProfileField::Instructions, &persona);
        let text = draft.text().expect("a persona drafts");
        assert!(
            text.chars().count() > MAX_DESCRIPTION,
            "a persona is not held to the card's one line: {} chars",
            text.chars().count()
        );
    }

    /// A blank answer is a failure, not an empty suggestion.
    #[test]
    fn a_blank_answer_is_unreadable_rather_than_an_empty_draft() {
        for blank in ["", "   ", "\n\t "] {
            let draft = ProfileDraft::from_answer(ProfileField::Instructions, blank);
            assert_eq!(draft.refusal(), Some(DraftRefusal::Unreadable), "{blank:?}");
            assert_eq!(draft.text(), None);
        }
    }

    #[test]
    fn a_refusal_names_the_operators_next_move() {
        assert_eq!(DraftRefusal::NoModel.as_str(), "no_model");
        assert_eq!(DraftRefusal::ModelUnreachable.as_str(), "model_unreachable");
        assert_eq!(DraftRefusal::Unreadable.as_str(), "unreadable");
    }
}
