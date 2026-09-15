//! Issue #983: settle chat turns a previous host process left open.
//!
//! The tenant-side half of "a turn that was accepted must be readable". A chat
//! turn journals a [`CompanyEvent::TurnStarted`] the moment the request is
//! accepted and a [`CompanyEvent::TurnFailed`] if it errors, so the pair is a
//! bracket the way `WorkflowRunStarted` / `WorkflowRunFinished` is — and a start
//! with no terminal at boot is a turn that died with the last host.
//!
//! Kept beside the workflow sweep it mirrors rather than folded into it, because
//! the two read different brackets and neither wants the other's shape.

use std::collections::HashMap;
use std::sync::Arc;

use crate::ports::EventLog;
use crate::ports::types::{CompanyEvent, CompanyId, EventSeq};

/// The reason stamped on a turn the host never got to finish.
///
/// Phrased as a host fact, like [`INTERRUPTED_BY_RESTART`][super::INTERRUPTED_BY_RESTART]:
/// nothing about the question went wrong, the process holding the answer went
/// away. Without this line the transcript holds the operator's question, no
/// answer after it, and no explanation — which is indistinguishable from a
/// message that never warranted a reply.
pub const TURN_INTERRUPTED_BY_RESTART: &str = concat!(
    "this turn was interrupted by a host restart and never answered; ",
    "ask again — nothing it did before stopping is lost, but no reply was produced"
);

/// Terminates chat turns a previous host process left open.
///
/// # Why an unterminated start is provably dead
///
/// The same three invariants
/// [`reap_orphaned_runs`](crate::ports::runs::reap_orphaned_runs) rests on, and
/// they hold for a chat turn verbatim: a turn is a process-local `tokio::spawn`
/// so it cannot outlive the process that spawned it; exactly one process owns a
/// company's journal (it is a documented single-writer log); and every turn
/// serialises on the per-company cycle mutex, so no turn from *this* process is
/// in flight before boot completes. So at boot an unmatched `TurnStarted` cannot
/// belong to a live turn: there are no live turns. No timeout heuristic is
/// needed, for exactly the reason none is needed there.
///
/// # It must NOT run on a rebuild
///
/// The argument above holds at boot and is false the moment a company has been
/// serving — and here the consequence is worse than it is for a workflow run. A
/// chat turn survives a live runtime swap ([`rebuild_company`](super::rebuild_company)):
/// `rebuild_company` quiesces and drains the *cycle* lock, but the spawned turn
/// task owns the reply journaling and the row settle **after** its cycle
/// returns, and the successor adopts the same mutex. Sweeping mid-life would
/// therefore stamp "interrupted by a host restart" on a turn that is still
/// working, and its real answer would land afterwards — leaving the operator a
/// transcript that says the turn failed and then answers it. The caller gates on
/// the handover being absent; see the call site in the runtime builder. Same
/// lesson as #290.
///
/// Best-effort throughout: a read or append failure is logged and swallowed,
/// because record-keeping must never stop a company from booting.
pub async fn sweep_interrupted_turns(events: &Arc<dyn EventLog>, company: &CompanyId) {
    let stored = match events
        .read_from(company, EventSeq::new(0), usize::MAX)
        .await
    {
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(
                %company,
                %err,
                "could not read the journal to sweep interrupted turns"
            );
            return;
        }
    };

    // One pass keyed on turn id: a start inserts, a failure removes. Whatever is
    // left was accepted and never settled. `HashMap` rather than a set because
    // the log line names the desk, which lives only on the start.
    let mut open: HashMap<String, String> = HashMap::new();
    for stored in stored {
        match stored.event {
            CompanyEvent::TurnStarted {
                turn_id, chat_id, ..
            } => {
                open.insert(turn_id, chat_id);
            }
            CompanyEvent::TurnFailed { turn_id, .. } => {
                open.remove(&turn_id);
            }
            _ => {}
        }
    }

    if open.is_empty() {
        return;
    }

    // Sorted so the appended order is deterministic — a `HashMap` iteration
    // order would make the journal's tail differ run to run for no reason.
    let mut interrupted: Vec<(String, String)> = open.into_iter().collect();
    interrupted.sort_by(|a, b| a.0.cmp(&b.0));

    for (turn_id, chat_id) in interrupted {
        tracing::info!(
            %company,
            turn = %turn_id,
            chat = %chat_id,
            "settling a chat turn left open by a previous host process"
        );
        if let Err(err) = events
            .append(
                company,
                CompanyEvent::TurnFailed {
                    turn_id: turn_id.clone(),
                    error: TURN_INTERRUPTED_BY_RESTART.to_string(),
                },
            )
            .await
        {
            tracing::warn!(
                %company,
                turn = %turn_id,
                %err,
                "could not settle an interrupted turn; the next boot sweeps it again"
            );
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::store::FsEventLog;

    fn log() -> (tempfile::TempDir, Arc<dyn EventLog>) {
        let dir = tempfile::Builder::new()
            .prefix("oc-turn-sweep-")
            .tempdir()
            .expect("tempdir");
        let events: Arc<dyn EventLog> = Arc::new(FsEventLog::new(dir.path()));
        (dir, events)
    }

    async fn started(events: &Arc<dyn EventLog>, company: &CompanyId, turn_id: &str) {
        events
            .append(
                company,
                CompanyEvent::TurnStarted {
                    turn_id: turn_id.to_string(),
                    chat_id: "general".to_string(),
                    parent: None,
                    by: None,
                },
            )
            .await
            .expect("append");
    }

    async fn failures(events: &Arc<dyn EventLog>, company: &CompanyId) -> Vec<(String, String)> {
        events
            .read_from(company, EventSeq::new(0), usize::MAX)
            .await
            .expect("read")
            .into_iter()
            .filter_map(|s| match s.event {
                CompanyEvent::TurnFailed { turn_id, error } => Some((turn_id, error)),
                _ => None,
            })
            .collect()
    }

    /// The case the sweep exists for: the host died holding a turn, so the
    /// operator's question sits in the transcript with nothing after it.
    #[tokio::test]
    async fn an_unterminated_turn_is_settled_at_boot() {
        let (_home, events) = log();
        let company = CompanyId::new("acme");
        started(&events, &company, "turn-dead").await;

        sweep_interrupted_turns(&events, &company).await;

        assert_eq!(
            failures(&events, &company).await,
            vec![(
                "turn-dead".to_string(),
                TURN_INTERRUPTED_BY_RESTART.to_string()
            )]
        );
    }

    /// A turn that settled itself is left alone, and the sweep is idempotent —
    /// its own synthetic failure closes the bracket, so a second boot after an
    /// unclean one does not stack a second line onto the same turn.
    #[tokio::test]
    async fn a_settled_turn_is_never_swept_twice() {
        let (_home, events) = log();
        let company = CompanyId::new("acme");
        started(&events, &company, "turn-ok").await;
        events
            .append(
                &company,
                CompanyEvent::TurnFailed {
                    turn_id: "turn-ok".to_string(),
                    error: "the brain refused".to_string(),
                },
            )
            .await
            .expect("append");
        started(&events, &company, "turn-dead").await;

        sweep_interrupted_turns(&events, &company).await;
        sweep_interrupted_turns(&events, &company).await;

        assert_eq!(
            failures(&events, &company).await,
            vec![
                ("turn-ok".to_string(), "the brain refused".to_string()),
                (
                    "turn-dead".to_string(),
                    TURN_INTERRUPTED_BY_RESTART.to_string()
                ),
            ],
            "the sweep re-settled a turn it had already settled"
        );
    }

    /// A company with nothing open appends nothing at all — the sweep must not
    /// leave a trace of having run on every boot of every company.
    #[tokio::test]
    async fn a_quiet_company_is_untouched() {
        let (_home, events) = log();
        let company = CompanyId::new("acme");
        sweep_interrupted_turns(&events, &company).await;
        assert!(
            events
                .read_from(&company, EventSeq::new(0), usize::MAX)
                .await
                .expect("read")
                .is_empty()
        );
    }

    /// One company's dead turn is not another's.
    #[tokio::test]
    async fn the_sweep_is_scoped_to_one_company() {
        let (_home, events) = log();
        let acme = CompanyId::new("acme");
        let other = CompanyId::new("other");
        started(&events, &acme, "turn-a").await;
        started(&events, &other, "turn-b").await;

        sweep_interrupted_turns(&events, &acme).await;

        assert_eq!(failures(&events, &acme).await.len(), 1);
        assert!(
            failures(&events, &other).await.is_empty(),
            "another company's live turn was settled"
        );
    }
}
