//! Hive-mind desks: a desk with two or more members answers as a room.
//!
//! A message addressed to a desk used to select exactly one responder off a
//! deterministic ladder — the desk lead, or the channel's per-message pick —
//! and that agent's single turn was the whole of the desk's answer. This module
//! is the alternative for a desk that has somebody to deliberate *with*: the
//! operator's message opens an **episode**, and the episode runs a sequence of
//! **rounds** until the room converges on an option, deadlocks between two,
//! spends its budget, or finds it has nothing to say.
//!
//! # One message is still a bounded number of turns
//!
//! An episode is not a fan-out. [`tinyhivemind_hive::step`] authorizes a
//! *round* — at most `round_width` speakers while the room is blind, at most
//! `revealed_width` once it can see itself — so the number of turns an operator
//! message can start is bounded by the desk's turn budget and by nothing else,
//! exactly as it was when a round was always one. What the room buys over a
//! single responder is still not parallelism for its own sake, it is
//! *independence* and a reason to stop: members authorized together cannot read
//! one another, so a member forms its own position before it reads its peers',
//! and the episode ends on a quorum it can name rather than when one agent
//! decides it is finished.
//!
//! A width is a bound on what the episode *authorizes*, not an instruction on
//! how to run it: this host takes a round's turns in series.
//!
//! # Nothing here is a second kind of turn
//!
//! Each turn the episode authorizes runs through the ordinary turn path — the
//! same tools, the same memory retrieve/inject/store loop, the same approval
//! gate. The episode supplies the prompt and decides who is asked; everything
//! that makes a teammate a teammate is unchanged. That is why the driver takes
//! its turn runner as a trait ([`HiveTurnRunner`]) rather than reaching for the
//! harness directly: the seam is one function wide, and a test can drive a
//! whole episode without a model.
//!
//! # What lands in the journal
//!
//! Every turn is journaled as an ordinary [`AgentReply`] on the desk, authored
//! by the teammate that spoke, carrying the one line the room counts. The
//! episode then writes one closing row under [`HIVE_REPORT_AUTHOR`] saying how
//! it ended. There is no second store and no episode record: the transcript is
//! the episode, which is what makes the standings impossible to disagree with
//! the conversation they were folded from.
//!
//! [`AgentReply`]: crate::ports::types::CompanyEvent::AgentReply
//!
//! # Modules
//!
//! - [`aside`] — two members of a desk comparing notes without the room.
//! - [`episode`] — the host loop, and the one-function turn seam.
//! - [`evidential`] — whether a `!support` reaches a fact, and the correction
//!   it gets when it does not.
//! - [`log`] — the company journal read as a `tinyhivemind` session log.
//! - [`memory`] — what the desk remembers between episodes, and the seam.
//! - [`moves`] — the per-member move grammar, and how a barred move is handled.
//! - [`prompt`] — what one authorized turn is shown, and how its answer is read.
//! - [`referral`] — the one mechanism here that leaves the room: asking another
//!   desk a question, and carrying its answer back without carrying its vote.
//! - [`scope`] — the upper bound a second, concurrent episode in the same
//!   thread needs and the shared watermark alone does not give it.
//! - [`types`] — the manifest knob, the desk snapshot, and the outcome.
//!
//! See `docs/spec/runtime/hivemind.md`.

pub mod aside;
pub mod episode;
pub mod evidential;
pub mod log;
pub mod memory;
pub mod moves;
pub mod prompt;
pub mod referral;
pub mod scope;
pub mod types;

#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod aside_test;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod concurrency_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod deliberation_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod moves_fixtures_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod moves_grammar_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod moves_memory_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod moves_misc_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod referral_crossing_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod referral_fixtures_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod referral_prompt_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
mod round_tests;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
#[cfg(test)]
mod aside_tests;
pub(crate) mod test;
#[cfg(test)]
mod aside_tests;

#[cfg(test)]
mod aside_tests;
pub use aside::{ASIDE_MARKER, AsideConfig, SURFACE_MARKER};
#[cfg(test)]
mod aside_tests;
pub use episode::{EpisodeDriver, HiveTurnRunner};
#[cfg(test)]
mod aside_tests;
pub use log::EventLogSessionLog;
#[cfg(test)]
mod aside_tests;
pub use memory::{
#[cfg(test)]
mod aside_tests;
    HIVE_MEMORY_LABEL_PREFIX, HiveMemory, HiveMemoryHit, HiveMemoryNote, NullHiveMemory,
#[cfg(test)]
mod aside_tests;
    desk_prefix, note_label,
#[cfg(test)]
mod aside_tests;
};
#[cfg(test)]
mod aside_tests;
pub use moves::{MOVE_KINDS, MoveViolation, UNGATED_KINDS, line_kind, readable};
#[cfg(test)]
mod aside_tests;
pub use prompt::{EpisodePrompt, canonical_topic, marker_line};
#[cfg(test)]
mod aside_tests;
pub use referral::{
#[cfg(test)]
mod aside_tests;
    AskedQuestion, EpisodeReferrals, FederationDesk, HiveFederation, HiveReferralRunner,
#[cfg(test)]
mod aside_tests;
    REACH_WORDS, ReferralConfig, ReferralLedger,
#[cfg(test)]
mod aside_tests;
};
#[cfg(test)]
mod aside_tests;
pub use scope::EpisodeScope;
#[cfg(test)]
mod aside_tests;
pub use types::{
#[cfg(test)]
mod aside_tests;
    EpisodeEnding, EpisodeOutcome, HiveConfig, HiveDesk, HiveMember, HivePolicy, company_desks,
#[cfg(test)]
mod aside_tests;
    desk_episode, desk_federation, effective_hive_config,
#[cfg(test)]
mod aside_tests;
};
#[cfg(test)]
mod aside_tests;

#[cfg(test)]
mod aside_tests;
/// The `agent_id` an episode's closing outcome row is journaled under.
#[cfg(test)]
mod aside_tests;
///
#[cfg(test)]
mod aside_tests;
/// Hyphenated on purpose, exactly as
#[cfg(test)]
mod aside_tests;
/// [`WORKFLOW_REPLY_AUTHOR`](crate::runtime::channel::WORKFLOW_REPLY_AUTHOR)
#[cfg(test)]
mod aside_tests;
/// is: `agent_slug` (console-minted teammates) and `is_snake_case`
#[cfg(test)]
mod aside_tests;
/// (manifest-declared ones) both reject a hyphen, so no roster id — minted
#[cfg(test)]
mod aside_tests;
/// before this constant existed or after — can ever equal it. A company that
#[cfg(test)]
mod aside_tests;
/// happened to name a teammate "Hive" therefore cannot have the room's own
#[cfg(test)]
mod aside_tests;
/// summary misattributed to it, and the read path can tell an unauthored
#[cfg(test)]
mod aside_tests;
/// outcome row from a teammate's line without consulting a roster.
#[cfg(test)]
mod aside_tests;
pub const HIVE_REPORT_AUTHOR: &str = "hive-report";
#[cfg(test)]
mod aside_tests;

#[cfg(test)]
mod aside_tests;
/// The `agent_id` a failed turn's notice is journaled under.
#[cfg(test)]
mod aside_tests;
///
#[cfg(test)]
mod aside_tests;
/// Hyphenated for the same reason [`HIVE_REPORT_AUTHOR`] is, and read back as
#[cfg(test)]
mod aside_tests;
/// a system row on the same terms — but a DIFFERENT id, because the two rows
#[cfg(test)]
mod aside_tests;
/// answer to different readers.
#[cfg(test)]
mod aside_tests;
///
#[cfg(test)]
mod aside_tests;
/// The closing report restates a tally whose inputs are already on screen as
#[cfg(test)]
mod aside_tests;
/// the turns that produced them, so a console may reasonably decline to draw
#[cfg(test)]
mod aside_tests;
/// it. A failure notice is the opposite: the turn it describes does not exist,
#[cfg(test)]
mod aside_tests;
/// so there is no gap for a reader to notice and nothing else records that a
#[cfg(test)]
mod aside_tests;
/// seat was asked and could not answer. Sharing one id forced the two to be
#[cfg(test)]
mod aside_tests;
/// shown or hidden together, and hiding this one leaves "a transcript with a
#[cfg(test)]
mod aside_tests;
/// hole in it that nothing accounts for".
#[cfg(test)]
mod aside_tests;
pub const HIVE_FAILURE_AUTHOR: &str = "hive-failure";
#[cfg(test)]
mod aside_tests;

#[cfg(test)]
mod aside_tests;
/// The `agent_id` an answer carried back from another desk is journaled under.
#[cfg(test)]
mod aside_tests;
///
#[cfg(test)]
mod aside_tests;
/// Hyphenated for exactly the reason [`HIVE_REPORT_AUTHOR`] is — no roster id
#[cfg(test)]
mod aside_tests;
/// can spell it — but a **second** reserved id rather than a reuse of that one,
#[cfg(test)]
mod aside_tests;
/// because the two rows say different things and a reader that cannot tell them
#[cfg(test)]
mod aside_tests;
/// apart is a reader that has been told a peer desk's answer is this room's own
#[cfg(test)]
mod aside_tests;
/// summary. Both fold as system rows, so neither can ever be counted as a
#[cfg(test)]
mod aside_tests;
/// supporter; only this one may appear more than once in an episode.
#[cfg(test)]
mod aside_tests;
pub const HIVE_REFERRAL_AUTHOR: &str = "hive-referral";
#[cfg(test)]
mod aside_tests;

#[cfg(test)]
mod aside_tests;
/// Whether an `agent_id` is one of this module's reserved system authors rather
#[cfg(test)]
mod aside_tests;
/// than a teammate.
#[cfg(test)]
mod aside_tests;
///
#[cfg(test)]
mod aside_tests;
/// Three ids now say "the room, not a member" — the closing report, a failed
#[cfg(test)]
mod aside_tests;
/// turn's notice, and an answer carried back from another desk — and a caller
#[cfg(test)]
mod aside_tests;
/// that wants "the lines members actually said" has to exclude all three. Every
#[cfg(test)]
mod aside_tests;
/// one is hyphenated so no roster id can equal it, which is what makes this a
#[cfg(test)]
mod aside_tests;
/// safe test rather than a guess.
#[cfg(test)]
mod aside_tests;
#[must_use]
#[cfg(test)]
mod aside_tests;
pub fn is_hive_author(agent_id: &str) -> bool {
#[cfg(test)]
mod aside_tests;
    matches!(
#[cfg(test)]
mod aside_tests;
        agent_id,
#[cfg(test)]
mod aside_tests;
        HIVE_REPORT_AUTHOR | HIVE_FAILURE_AUTHOR | HIVE_REFERRAL_AUTHOR
#[cfg(test)]
mod aside_tests;
    )
#[cfg(test)]
mod aside_tests;
}
#[cfg(test)]
mod aside_tests;
