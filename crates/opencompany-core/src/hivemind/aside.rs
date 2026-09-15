//! Private asides: two members of a desk comparing notes without the room.
//!
//! This is the one mechanism in this module that narrows *who reads a line*
//! rather than who speaks it. [`referral`](super::referral) lets a room ask
//! another desk a question; an aside lets two members of the **same** desk say
//! something the rest of that desk cannot read.
//!
//! # It is auditable, not confidential
//!
//! A row an agent may not read is **elided, never dropped**. Its sequence, its
//! author and its audience stay in every projection, and only its content is
//! withheld — so a peer can see that an exchange happened and who was in it,
//! and a `^N` citation naming it still resolves. Three reasons that shape was
//! chosen over an absent row are in the library's own spec
//! (`vendor/tinyhivemind/docs/specs/private-asides.md`): auditability, latent
//! asymmetry (an agent that cannot see that a peer knows something has no
//! reason to ask), and citations.
//!
//! Operators and people read every aside in full. The privacy here is between
//! agents and is a deliberation device — **never a security boundary**, and
//! nothing in this module should ever be relied on as one.
//!
//! # An aside carries information, never support
//!
//! The rule that keeps the room's counting single-valued: a trace deposited in
//! a row whose audience is not `Desk` contributes nothing — no supporter, no
//! movement toward a decision. This host does not enforce it, and does not need
//! to: [`step`](tinyhivemind_hive::step) folds the whole transcript and drops
//! non-`Desk` traces uniformly, for every reader. To make an aside count, a
//! member spends a desk-visible turn saying so in the open — that is what
//! `!surface` is for, and a `!surface` line is an ordinary desk row with no
//! special handling anywhere in this module.
//!
//! This is the same rule cross-desk referral already accepts at a channel
//! boundary, applied inside one desk.
//!
//! # Why it is off by default
//!
//! Because it was measured and it lost. The library's own experiment
//! (`docs/experiments/2026-09-07-do-asides-help.md`) found a pairwise private
//! check costs 2.5 points at a tuned turn budget, loses 15 points on a hidden
//! profile — averaging inside one correlated desk imports the shared bias — and
//! is indistinguishable from the same exchange held in the open. So this claims
//! bounded independence and an auditable record, and claims **nothing** about
//! answer quality. [`AsideConfig`] defaults to disabled, and a desk that wants
//! asides says so.

use serde::{Deserialize, Serialize};
use tinyhivemind_hive::aside::{AsidePolicy, Audience};

/// The marker a member opens a private line with.
pub const ASIDE_MARKER: &str = "!aside";

/// The marker that pays an aside back to the room.
pub const SURFACE_MARKER: &str = "!surface";

/// The manifest knob: `hive = { aside = { enabled = true, ... } }`.
///
/// Every field is `Option` so an absent block and an explicitly-default block
/// are distinguishable, and so a desk may set one bound without restating the
/// others. [`Self::policy`] is the only place the defaults live.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AsideConfig {
    /// Whether members of this desk may open a private line at all.
    ///
    /// Off unless a desk says otherwise — see the module docs for why the
    /// mechanism does not get the benefit of the doubt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Largest addressed audience, **excluding** the author.
    ///
    /// Defaults to 1: a direct pair. A caucus of three inside a desk of four is
    /// not an aside, it is a second desk with no quorum and no record, and a
    /// host that wants one should declare the desk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_members: Option<usize>,
    /// How many rows one aside may carry before it must settle.
    ///
    /// Defaults to 2 — a question and an answer. The bound is small because
    /// every row spent privately is a row of the desk's turn budget spent where
    /// the room cannot read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_messages: Option<usize>,
    /// Whether an aside must deposit one desk-visible message before the same
    /// members may open another.
    ///
    /// Defaults to `true`. Without it a pair can hold the whole episode
    /// privately and the room's record is a run of stubs.
    ///
    /// Enforced at the *next* aside rather than at the last turn: nothing
    /// compels a settlement before an episode ends, so a room can still close
    /// with one unsettled. Making the episode refuse to converge on an
    /// unsettled aside was considered upstream and rejected as too blunt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_surface: Option<bool>,
    /// Whether an aside must be a thread rather than a run of channel rows.
    ///
    /// Defaults to **false** here, unlike the library's own suggestion, because
    /// a hive episode runs on the desk channel: every turn is parented to the
    /// operator message's own parent so the library's channel projection can
    /// promote it (`docs/spec/runtime/hivemind.md`). A desk that required
    /// threads would authorize no asides at all, which is a confusing way to
    /// spell `enabled = false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_thread: Option<bool>,
}

impl AsideConfig {
    /// Whether this desk opted in.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    /// Whether the block says nothing, so it can be skipped on the wire.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The library policy this config means.
    ///
    /// A disabled config returns [`AsidePolicy::DEFAULT`] — every bound at zero
    /// — rather than a disabled policy carrying this desk's bounds, so a desk
    /// that is off cannot be read as a desk that is on with limits.
    #[must_use]
    pub fn policy(&self) -> AsidePolicy {
        if !self.enabled() {
            return AsidePolicy::DEFAULT;
        }
        AsidePolicy {
            enabled: true,
            max_members: self.max_members.unwrap_or(1).max(1),
            max_messages: self.max_messages.unwrap_or(2).max(1),
            must_surface: self.must_surface.unwrap_or(true),
            require_thread: self.require_thread.unwrap_or(false),
        }
    }
}

/// Whether this line opens a private aside.
///
/// Matched at the start of the line and on a word boundary, exactly as the
/// trace grammar matches its own markers: `!asides` is not `!aside`, and a line
/// that merely mentions the word is an ordinary line.
#[must_use]
pub fn opens_aside(line: &str) -> bool {
    marker_is(line, ASIDE_MARKER)
}

/// Whether this line pays an aside back to the room.
#[must_use]
pub fn surfaces(line: &str) -> bool {
    marker_is(line, SURFACE_MARKER)
}

fn marker_is(line: &str, marker: &str) -> bool {
    let line = line.trim_start();
    let Some(rest) = line.strip_prefix(marker) else {
        return false;
    };
    rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace())
}

/// The aside a row belongs to, as a canonical, order-independent key.
///
/// The author is included alongside the addressees because an audience is
/// "author plus the named ids" — two rows with the same addressee but different
/// authors are two different asides, and folding them into one would let one
/// member's budget be spent by another.
#[must_use]
pub fn party(author: &str, audience: &Audience) -> Option<Vec<String>> {
    let members = audience.members();
    if members.is_empty() {
        return None;
    }
    let mut party: Vec<String> = members.to_vec();
    party.push(author.to_owned());
    party.sort();
    party.dedup();
    Some(party)
}

/// How many rows this party has already spent, and whether its last aside is
/// still unsettled.
///
/// Both are folded from the transcript the caller already holds rather than
/// tracked in memory, for the reason the episode re-reads its transcript every
/// iteration: what the fold counts is exactly what a person reading the desk
/// would see, so the two can never disagree.
///
/// "Unsettled" means the party opened an aside and no member of it has written
/// a desk-visible `!surface` since. A `!surface` by any member settles it —
/// the room is owed one settlement, not one per participant.
#[must_use]
pub fn spent_and_unsettled(
    transcript: &[tinyhivemind_hive::SessionMessage],
    party: &[String],
) -> (usize, bool) {
    use tinyhivemind_hive::SessionAuthor;

    let author_of = |message: &tinyhivemind_hive::SessionMessage| match &message.author {
        SessionAuthor::Agent { id, .. } => Some(id.clone()),
        _ => None,
    };

    let mut spent = 0usize;
    let mut unsettled = false;
    for message in transcript {
        let Some(author) = author_of(message) else {
            continue;
        };
        if let Some(other) = self::party(&author, &message.audience) {
            if other == party {
                spent = spent.saturating_add(1);
                unsettled = true;
            }
            continue;
        }
        // A desk-visible row. Only a `!surface` from somebody in the party
        // settles it — an ordinary desk turn by a member does not, or the
        // requirement would be paid by the next thing anybody happened to say.
        //
        // `readable()` rather than `content`: a row this viewer may not read
        // must never be quoted, and here that also means it must never be
        // *classified*. In practice the transcript folded here is the operator
        // projection, so nothing is elided — going through the accessor is what
        // keeps that true if a caller ever passes a narrowed one.
        if party.contains(&author) && message.readable().is_some_and(surfaces) {
            unsettled = false;
        }
    }
    (spent, unsettled)
}

#[cfg(test)]
mod test {
    use super::*;
    use tinyhivemind_hive::{Sequence, SessionAuthor, SessionMessage};

    fn row(seq: u64, author: &str, content: &str, audience: Audience) -> SessionMessage {
        SessionMessage {
            sequence: Sequence(seq),
            author: SessionAuthor::Agent {
                id: author.to_owned(),
                label: author.to_owned(),
            },
            content: content.to_owned(),
            audience,
            elided: None,
        }
    }

    fn aside_to(who: &[&str]) -> Audience {
        Audience::Aside {
            members: who.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn a_default_config_authorizes_nothing() {
        let config = AsideConfig::default();
        assert!(!config.enabled());
        assert_eq!(config.policy(), AsidePolicy::DEFAULT);
        assert!(config.is_default());
    }

    /// A disabled desk that nevertheless wrote bounds must not read as a desk
    /// that is on: every bound goes to zero with the switch.
    #[test]
    fn a_disabled_config_discards_its_own_bounds() {
        let config = AsideConfig {
            enabled: Some(false),
            max_members: Some(3),
            max_messages: Some(9),
            ..AsideConfig::default()
        };
        assert_eq!(config.policy(), AsidePolicy::DEFAULT);
    }

    #[test]
    fn an_enabled_config_defaults_to_a_settled_pair() {
        let policy = AsideConfig {
            enabled: Some(true),
            ..AsideConfig::default()
        }
        .policy();
        assert!(policy.enabled);
        assert_eq!(policy.max_members, 1, "a pair, not a caucus");
        assert_eq!(policy.max_messages, 2, "a question and an answer");
        assert!(policy.must_surface, "the room is owed a settlement");
        assert!(
            !policy.require_thread,
            "a hive episode runs on the desk channel; requiring a thread would \
             authorize nothing"
        );
    }

    #[test]
    fn markers_match_on_a_word_boundary_only() {
        assert!(opens_aside("!aside @auditor is this figure right?"));
        assert!(opens_aside("  !aside @auditor"));
        assert!(opens_aside("!aside"));
        assert!(!opens_aside("!asides @auditor"));
        assert!(!opens_aside("I would !aside but cannot"));
        assert!(!opens_aside("!propose #stage"));

        assert!(surfaces(
            "!surface the auditor and I agree the figure holds"
        ));
        assert!(!surfaces("!surfaced it already"));
    }

    /// The party is the author plus the addressees, canonically ordered, so the
    /// same pair folds to one key whichever of them wrote the row.
    #[test]
    fn a_party_is_order_independent_and_includes_its_author() {
        let one = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let other = party("auditor", &aside_to(&["planner"])).expect("an aside");
        assert_eq!(one, other);
        assert_eq!(one, vec!["auditor".to_owned(), "planner".to_owned()]);
        assert!(
            party("planner", &Audience::Desk).is_none(),
            "a desk row is in no party"
        );
    }

    #[test]
    fn spending_counts_only_this_partys_rows() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let transcript = vec![
            row(
                1,
                "planner",
                "!aside @auditor check this",
                aside_to(&["auditor"]),
            ),
            row(
                2,
                "auditor",
                "!aside @planner it holds",
                aside_to(&["planner"]),
            ),
            // A different pair entirely: must not spend this party's budget.
            row(
                3,
                "planner",
                "!aside @scout and this?",
                aside_to(&["scout"]),
            ),
            row(4, "critic", "!propose #stage", Audience::Desk),
        ];
        let (spent, unsettled) = spent_and_unsettled(&transcript, &party_key);
        assert_eq!(spent, 2);
        assert!(unsettled, "nobody surfaced it");
    }

    #[test]
    fn a_surface_by_either_member_settles_the_party() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let mut transcript = vec![row(
            1,
            "planner",
            "!aside @auditor check this",
            aside_to(&["auditor"]),
        )];
        assert!(spent_and_unsettled(&transcript, &party_key).1);

        // The *other* member pays it back, which is enough: the room is owed
        // one settlement, not one per participant.
        transcript.push(row(
            2,
            "auditor",
            "!surface the figure holds",
            Audience::Desk,
        ));
        let (spent, unsettled) = spent_and_unsettled(&transcript, &party_key);
        assert_eq!(spent, 1, "a settlement is a desk row, not an aside row");
        assert!(!unsettled);
    }

    /// An ordinary desk turn by a member is not a settlement. Otherwise the
    /// requirement is paid by whatever anybody happened to say next, and the
    /// room never learns what the aside produced.
    #[test]
    fn an_ordinary_desk_turn_does_not_settle_an_aside() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let transcript = vec![
            row(
                1,
                "planner",
                "!aside @auditor check this",
                aside_to(&["auditor"]),
            ),
            row(2, "planner", "!propose #stage Stage it.", Audience::Desk),
        ];
        assert!(
            spent_and_unsettled(&transcript, &party_key).1,
            "a proposal is not a settlement"
        );
    }

    /// A `!surface` from somebody who was never in the aside settles nothing —
    /// they have nothing to report and cannot discharge a debt they do not owe.
    #[test]
    fn a_surface_by_a_non_member_settles_nothing() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let transcript = vec![
            row(
                1,
                "planner",
                "!aside @auditor check this",
                aside_to(&["auditor"]),
            ),
            row(2, "critic", "!surface they seemed to agree", Audience::Desk),
        ];
        assert!(spent_and_unsettled(&transcript, &party_key).1);
    }

    /// Reopening after a settlement starts the debt again, and the spend count
    /// keeps rising: the budget is per party per episode, not per aside.
    #[test]
    fn reopening_after_a_settlement_owes_the_room_again() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let transcript = vec![
            row(
                1,
                "planner",
                "!aside @auditor check this",
                aside_to(&["auditor"]),
            ),
            row(2, "auditor", "!surface the figure holds", Audience::Desk),
            row(
                3,
                "planner",
                "!aside @auditor and the other one?",
                aside_to(&["auditor"]),
            ),
        ];
        let (spent, unsettled) = spent_and_unsettled(&transcript, &party_key);
        assert_eq!(spent, 2);
        assert!(unsettled);
    }

    /// The operator's own rows carry no agent author and must never be folded
    /// into anybody's party or counted as anybody's settlement.
    #[test]
    fn operator_rows_are_in_no_party() {
        let party_key = party("planner", &aside_to(&["auditor"])).expect("an aside");
        let transcript = vec![
            SessionMessage {
                sequence: Sequence(1),
                author: SessionAuthor::Operator,
                content: "!surface".to_owned(),
                audience: Audience::Desk,
                elided: None,
            },
            row(
                2,
                "planner",
                "!aside @auditor check this",
                aside_to(&["auditor"]),
            ),
        ];
        let (spent, unsettled) = spent_and_unsettled(&transcript, &party_key);
        assert_eq!(spent, 1);
        assert!(unsettled, "an operator cannot settle an agents' aside");
    }
}
