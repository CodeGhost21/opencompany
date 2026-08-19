//! Harnesses: the execution engines a company's agents run their turns on.
//!
//! A **harness** is one answer to "what actually runs this agent's turn". A
//! company declares a named set of them in `company.toml` and binds each agent
//! to one, so a single company can put its researcher on a deep reasoning model,
//! its bulk workers on a cheap one, and its coding agent on the operator's own
//! Claude Code — the last of which needs no credential from us at all.
//!
//! Two kinds ship:
//!
//! * [`built_in`] — the embedded OpenHuman/tinyagents loop, in this process,
//!   against an inference provider the harness itself declares. Everything
//!   under `built_in/` is *one harness implementation*, not "the harness".
//! * [`acp`] — an external agent driven over the Agent Client Protocol, either
//!   a subprocess on the operator's machine or a runner that dialed in.
//!
//! Both are [`RunTurn`](crate::runtime::delegation::RunTurn) implementations,
//! which is the whole point: the company cycle asks for a turn and does not
//! learn which engine served it.
//!
//! ## Transitional re-exports
//!
//! `built_in`'s contents were previously declared directly here, so the glob
//! below keeps every `crate::harness::X` path resolving while callers migrate to
//! `crate::harness::built_in::X`. It is deliberately a re-export rather than a
//! rename in place: the move commit that created this file changed no content,
//! and the paths are updated separately.

pub mod acp;
pub mod built_in;
pub mod lanes;
pub mod router;

pub use built_in::*;

/// Issue #989 (Part 2a of #926): end-to-end proof that a **chat** turn which
/// pauses at its tool-iteration cap runs the same #244 unpublished-work scan
/// and nudge the task-dispatch path (`run_task`) already gets — and that a
/// capped turn which wrote nothing is not nudged on top of it. Test-only.
#[cfg(test)]
mod cap_publish_test;
/// Issue #926: end-to-end proof that a turn which exhausts its tool-iteration
/// budget pauses **visibly** — the flag is read, the operator gets a second
/// bubble saying so, and the notice never reaches memory. Test-only.
#[cfg(test)]
mod cap_turn_test;
/// End-to-end proof of issue #988: a turn really does get
/// [`MAX_TOOL_ITERATIONS`](build::MAX_TOOL_ITERATIONS) tool rounds instead of the
/// vendored ten, and a budget-armed turn's in-turn
/// [`BudgetStopHook`](oh::agent::stop_hooks::BudgetStopHook) halts it when it
/// outruns its money — distinguishably from an iteration-cap pause. Test-only.
#[cfg(test)]
mod iteration_cap_turn_test;

/// The ACP `RunTurn`, under the path it had before the split.
///
/// Aliased rather than glob-re-exported because [`built_in`] has its own
/// `run_turn` module — the two would collide under a bare re-export.
#[cfg(feature = "acp")]
pub use acp::run_turn as acp_run_turn;
