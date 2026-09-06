//! Hive-mind desks: a desk with two or more members answers as a room.
//!
//! A message addressed to a desk used to select exactly one responder off a
//! deterministic ladder — the desk lead, or the channel's per-message pick —
//! and that agent's single turn was the whole of the desk's answer. This module
//! is the alternative for a desk that has somebody to deliberate *with*: the
//! operator's message opens an **episode**, and the episode runs a sequence of
//! single turns until the room converges on an option, deadlocks between two,
//! spends its budget, or finds it has nothing to say.
//!
//! # One message still means one turn at a time
//!
//! An episode is not a fan-out. [`tinyhivemind_hive::step`] authorizes exactly
//! one speaker per step, so the number of turns an operator message can start
//! is bounded by the desk's turn budget and by nothing else. What the room buys
//! over a single responder is not parallelism, it is *independence* and a
//! reason to stop: the opening round is blind, so a member forms its own
//! position before it reads its peers', and the episode ends on a quorum it can
//! name rather than when one agent decides it is finished.
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
//! - [`episode`] — the host loop, and the one-function turn seam.
//! - [`evidential`] — whether a `!support` reaches a fact, and the correction
//!   it gets when it does not.
//! - [`log`] — the company journal read as a `tinyhivemind` session log.
//! - [`memory`] — what the desk remembers between episodes, and the seam.
//! - [`moves`] — the per-member move grammar, and how a barred move is handled.
//! - [`prompt`] — what one authorized turn is shown, and how its answer is read.
//! - [`referral`] — the one mechanism here that leaves the room: asking another
//!   desk a question, and carrying its answer back without carrying its vote.
//! - [`types`] — the manifest knob, the desk snapshot, and the outcome.
//!
//! See `docs/spec/runtime/hivemind.md`.

pub mod episode;
pub mod evidential;
pub mod log;
pub mod memory;
pub mod moves;
pub mod prompt;
pub mod referral;
pub mod types;

#[cfg(test)]
mod deliberation_test;
#[cfg(test)]
mod moves_test;
#[cfg(test)]
mod referral_test;
#[cfg(test)]
mod test;

pub use episode::{EpisodeDriver, HiveTurnRunner};
pub use log::EventLogSessionLog;
pub use memory::{
    HIVE_MEMORY_LABEL_PREFIX, HiveMemory, HiveMemoryHit, HiveMemoryNote, NullHiveMemory,
    desk_prefix, note_label,
};
pub use moves::{MOVE_KINDS, MoveViolation, UNGATED_KINDS};
pub use prompt::{EpisodePrompt, canonical_topic, marker_line};
pub use referral::{
    AskedQuestion, EpisodeReferrals, FederationDesk, HiveFederation, HiveReferralRunner,
    REACH_WORDS, ReferralConfig, ReferralLedger,
};
pub use types::{
    EpisodeEnding, EpisodeOutcome, HiveConfig, HiveDesk, HiveMember, HivePolicy, desk_episode,
    desk_federation,
};

/// The `agent_id` an episode's closing outcome row is journaled under.
///
/// Hyphenated on purpose, exactly as
/// [`WORKFLOW_REPLY_AUTHOR`](crate::runtime::channel::WORKFLOW_REPLY_AUTHOR)
/// is: `agent_slug` (console-minted teammates) and `is_snake_case`
/// (manifest-declared ones) both reject a hyphen, so no roster id — minted
/// before this constant existed or after — can ever equal it. A company that
/// happened to name a teammate "Hive" therefore cannot have the room's own
/// summary misattributed to it, and the read path can tell an unauthored
/// outcome row from a teammate's line without consulting a roster.
pub const HIVE_REPORT_AUTHOR: &str = "hive-report";
