//! The headline regression for `EpisodeScope`: two hive episodes
//! deliberating in the same thread must never fold each other's turns as
//! their own votes.
//!
//! # Why `tokio::join!`, and what it actually proves
//!
//! The bug this guards needs two things to be true at once: both episodes'
//! triggering messages are durable in the journal *before either episode's
//! first turn lands*, and episode B's very first `project_session` read
//! happens only after episode A has already deposited turns above B's own
//! trigger. `YieldFirstTurn` below is the seam that forces the second half —
//! episode B's runner will not answer its first turn until episode A's whole
//! run has notified it — while both episodes are driven from the same
//! `tokio::join!`, i.e. as two live futures the executor is actually holding
//! concurrently, not two sequential function calls the test happens to write
//! next to each other. That is the exact shape the finding describes: "two
//! follow-ups in the same existing thread are accepted before the first turn
//! finishes."
