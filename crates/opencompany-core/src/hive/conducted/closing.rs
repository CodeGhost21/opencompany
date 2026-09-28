//! The closing turn a settled episode routes to one seat.
//!
//! Split out of `conducted.rs` to keep that file under the 750-line cap: this
//! is one coherent step of the episode lifecycle -- everything between the last
//! seat settling and the closing row being journaled. What the step is *for*,
//! and the six live runs that measured the gap it fills, is documented on
//! [`crate::hive::conclude`]; this module is only the part that needs
//! `HiveDispatcher`'s own `run`, parking and event log.

use std::sync::Arc;

use super::{Episode, HiveDispatcher, run};
use crate::hive::episode_store;
use crate::hive::graph::DeskHive;
use crate::hive::routing::EffectiveRouting;
use crate::ports::types::{CompanyEvent, EventSeq};

impl HiveDispatcher {
    /// Run a settled episode's closing turn, and say where its message landed.
    ///
    /// Reached only through [`crate::hive::conclude::eligible`]. Failure here
    /// is deliberately not fatal: the episode *has* settled, every seat's work
    /// is already journaled, and refusing to write the closing row because the
    /// summary turn stalled would lose a finished episode over an extra. A
    /// warning names the seat and the episode instead, and the row goes down
    /// with `completed_by: None` exactly as it did before this existed.
    pub(super) async fn conclusion(
        &self,
        desk: &DeskHive,
        routing: &EffectiveRouting,
        episode_id: &str,
        thread_root: Option<EventSeq>,
        opened_at: EventSeq,
        request: &str,
    ) -> Option<crate::hive::conclude::Conclusion> {
        // What the episode produced, for the router to choose against. A read
        // that fails is not a reason to skip the round -- an uninformed choice
        // still beats no conclusion -- so an error here degrades to no context.
        let settled_rows = episode_store::episode_rows(
            self.events.as_ref(),
            &self.record.id,
            episode_id,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "[hive] the closing turn routes without the episode's findings");
            Vec::new()
        });
        let lead = match desk.lead() {
            Some(lead) => lead,
            None => {
                tracing::warn!(desk = %desk.desk_id, "[hive] no lead, so no closing turn");
                return None;
            }
        };
        // One call, two questions -- whether this still needs assembling and who
        // should do it -- when an oracle resolves. Without one, `route_desk`
        // answers the second and the episode always concludes, which is the
        // behaviour before the decision existed.
        let seat = match self.oracle.as_deref() {
            Some(oracle) => {
                let seats: Vec<String> = desk.hive.members().map(str::to_owned).collect();
                let findings = crate::hive::conclude::findings(&settled_rows, &desk.desk_id);
                match crate::hive::conclude::decide(oracle, request, &findings, &seats, &lead).await
                {
                    crate::hive::conclude::Decision::Conclude(seat) => seat,
                    crate::hive::conclude::Decision::NotNeeded => {
                        tracing::info!(
                            desk = %desk.desk_id,
                            episode = %episode_id,
                            "[hive] the desk already answered the request, so no closing turn"
                        );
                        return None;
                    }
                }
            }
            None => match crate::hive::conclude::pick_concluder(
                desk,
                routing,
                self.router.as_deref(),
                request,
                thread_root,
                &settled_rows,
            )
            .await
            {
                Ok(seat) => seat,
                Err(error) => {
                    tracing::warn!(%error, desk = %desk.desk_id, "[hive] the closing turn routed nowhere");
                    return None;
                }
            },
        };
        // **The line the closing turn must write above.**
        //
        // Taken before the round runs, because the read-back below cannot
        // otherwise tell the closing message from anything this seat said during
        // the episode proper. The concluder is usually a seat that already spoke
        // -- the lead, most often -- so "its last reply on the desk" is an
        // ordinary deliberation row until the closing turn adds one. Without
        // this, a closing turn that recorded nothing would hand
        // `EpisodeCompleted.summary_seq` a mid-episode message and the console
        // would label it the episode's summary.
        let before = settled_rows
            .iter()
            .map(|stored| stored.seq.value())
            .max()
            .unwrap_or(0);
        let outcome = run(Episode {
            record: Arc::clone(&self.record),
            deps: Arc::clone(&self.deps),
            pool: Arc::clone(&self.pool),
            events: Arc::clone(&self.events),
            desk,
            routing,
            // Nothing to route: the seat is already chosen, and it holds no
            // `broadcast` for a router to govern.
            router: None,
            episode_id: episode_id.to_owned(),
            thread_root,
            opened_at,
            starters: vec![seat.clone()],
            concluding: true,
            parking: self.seat_parking(&desk.desk_id, thread_root, episode_id),
            mentions: self.mentions.clone(),
        })
        .await;
        let closing = match outcome {
            Ok(closing) => closing,
            Err(error) => {
                tracing::warn!(%error, seat = %seat, episode = %episode_id, "[hive] the closing turn failed");
                return None;
            }
        };
        // Where the message landed, read back rather than tracked: the seat
        // journals through the host like any other turn, so its answer is a row
        // the closing turn added -- which is what `before` distinguishes. A turn
        // that recorded nothing leaves `None`, as `Conclusion::summary_seq`
        // documents, rather than pointing at something the seat said earlier.
        let summary_seq =
            match episode_store::episode_rows(self.events.as_ref(), &self.record.id, episode_id)
                .await
            {
                Ok(rows) => rows
                    .iter()
                    .rev()
                    .take_while(|stored| stored.seq.value() > before)
                    .find(|stored| match &stored.event {
                        CompanyEvent::AgentReply {
                            chat_id, agent_id, ..
                        } => chat_id == &desk.desk_id && agent_id == &seat,
                        _ => false,
                    })
                    .map(|stored| stored.seq.value()),
                Err(error) => {
                    tracing::warn!(%error, "[hive] could not read back the closing message");
                    None
                }
            };
        tracing::info!(
            desk = %desk.desk_id,
            episode = %episode_id,
            seat = %seat,
            summary_seq = ?summary_seq,
            "[hive] the episode was concluded"
        );
        Some(crate::hive::conclude::Conclusion {
            seat,
            summary_seq,
            turns: closing.turns,
            waves: closing.waves,
        })
    }
}
