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
mod aside_test;
#[cfg(test)]
mod aside_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod deliberation_tests;
#[cfg(test)]
mod moves_fixtures_tests;
#[cfg(test)]
mod moves_grammar_tests;
#[cfg(test)]
mod moves_memory_tests;
#[cfg(test)]
mod moves_misc_tests;
#[cfg(test)]
mod referral_crossing_tests;
#[cfg(test)]
mod referral_fixtures_tests;
#[cfg(test)]
mod referral_prompt_tests;
#[cfg(test)]
mod round_tests;
#[cfg(test)]
pub(crate) mod test;


pub use aside::{ASIDE_MARKER, AsideConfig, SURFACE_MARKER};
pub use episode::{EpisodeDriver, HiveTurnRunner};
pub use log::EventLogSessionLog;
pub use memory::{
pub use moves::{MOVE_KINDS, MoveViolation, UNGATED_KINDS, line_kind, readable};
pub use prompt::{EpisodePrompt, canonical_topic, marker_line};
pub use referral::{
pub use scope::EpisodeScope;
pub use types::{
