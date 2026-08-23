//! Who is here, and who is typing.
//!
//! Two ephemeral facts about the humans in a company, kept in memory and
//! published on the live event bus. Neither is journaled and neither has a
//! store: presence is a *lease* rather than a record, and typing is not even
//! that.
//!
//! # Presence is a TTL, not a connection table
//!
//! A console heartbeats every [`PRESENCE_HEARTBEAT_MILLIS`]; an entry is live
//! for [`PRESENCE_TTL_MILLIS`], which is **three times** the beat. The multiple
//! is the point: one dropped request must not flap somebody offline and back,
//! and a browser that crashes needs no cleanup at all — its lease simply
//! expires. Tracking sockets instead would mean every disconnect path (tab
//! close, sleep, network drop, pod restart) had to be handled correctly, and
//! the failure mode of missing one is a person who looks online forever.
//!
//! This is the same shape [`RunnerRegistry`](crate::runner::registry) already
//! uses, including its rule that a heartbeat is only meaningful for a caller
//! the host has actually authenticated.
//!
//! # The subject is always the caller
//!
//! A presence write names no user; the subject is taken from the session. A
//! body that could name somebody else would let any member mark any colleague
//! online or offline, and nothing downstream could tell the difference.
//!
//! # Replica-local, deliberately
//!
//! Two hosted replicas share a tenant database but not this map, so each knows
//! only about the consoles connected to it. That is exactly as partitioned as
//! the live turn timeline ([`crate::turn_stream`]) already is, and for the same
//! reason: both ride a process-local broadcast bus. A viewer sees everyone on
//! their own replica live, and everyone else through the durable
//! `lastSeenAtMillis` floor. Making presence cross-replica means a shared bus,
//! which is a much larger change than the feature warrants.
//!
//! # This is not read receipts
//!
//! [`read_state`](crate::server::ops::read_state) stays what it is: a private
//! floor for computing *your own* unread badge. Nothing here exposes what
//! another person has read. "Who is here" and "who has read this" look adjacent
//! and are not.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::ports::types::CompanyId;

/// How often a live console announces itself.
pub const PRESENCE_HEARTBEAT_MILLIS: u64 = 60_000;

/// How long an announcement stays good for.
///
/// Three times [`PRESENCE_HEARTBEAT_MILLIS`], so a single missed beat — a
/// slow request, a moment of packet loss, a tab that was throttled while
/// backgrounded — does not flap somebody offline and back.
pub const PRESENCE_TTL_MILLIS: u64 = 3 * PRESENCE_HEARTBEAT_MILLIS;

/// Exactly three states.
///
/// Anything a browser cannot honestly distinguish is deliberately not a state.
/// In particular there is no "busy" and no "invisible": the first is a guess,
/// and the second is a promise this design cannot keep, because a peer that
/// stops beating is indistinguishable from one that closed its laptop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresenceStatus {
    /// At the machine.
    Online,
    /// Signed in, but idle — **not** "the window is unfocused". See the console
    /// half: an unfocused window is not an absent human.
    Away,
    /// Explicitly appearing offline, or gone.
    Offline,
}

impl PresenceStatus {
    /// The wire word.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Away => "away",
            Self::Offline => "offline",
        }
    }
}

/// One person's live lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Peer {
    status: PresenceStatus,
    last_beat_millis: u64,
}

/// Who is currently present, per company.
///
/// Peer state is deliberately thin — a status and a timestamp, nothing else.
/// No cursor, no current room, no last-read: which channel somebody is looking
/// at is not a fact their colleagues need, and carrying it would turn a
/// presence dot into activity tracking.
#[derive(Debug, Default)]
pub struct PresenceRegistry {
    people: Mutex<HashMap<(CompanyId, String), Peer>>,
}

/// One person's presence, as a reader sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresenceView {
    /// The user id. Already known to every signed-in member through
    /// `GET {scope}/chat/mentionables`, which is why this carries no label:
    /// the console already holds the directory that names them.
    pub user_id: String,
    pub status: PresenceStatus,
    /// When this lease was last renewed, epoch millis.
    pub at_millis: u64,
}

impl PresenceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a heartbeat, and says whether it changed anything a watcher
    /// would care about.
    ///
    /// Returning "did this change" is what keeps the bus quiet: a console beats
    /// every minute whether or not anything moved, and republishing an
    /// unchanged `online` to every other console once a minute per person is
    /// pure noise. A frame goes out when somebody *arrives* or *changes state*,
    /// which is exactly when a dot would move.
    ///
    /// An expired lease counts as an arrival, so somebody who was away long
    /// enough to lapse is re-announced rather than silently reappearing on the
    /// next reader's poll.
    pub fn beat(
        &self,
        company: &CompanyId,
        user: &str,
        status: PresenceStatus,
        now_millis: u64,
    ) -> bool {
        let mut people = self.people.lock().expect("presence registry poisoned");
        let key = (company.clone(), user.to_string());
        let changed = match people.get(&key) {
            Some(peer) => peer.status != status || expired(peer.last_beat_millis, now_millis),
            None => true,
        };
        people.insert(
            key,
            Peer {
                status,
                last_beat_millis: now_millis,
            },
        );
        changed
    }

    /// Drops a lease immediately — a clean disconnect.
    ///
    /// Worth having even though the TTL would get there eventually: a person
    /// who closes a tab should not linger as online for three minutes, and the
    /// browser can say so on the way out. Returns whether anything was actually
    /// removed, so a duplicate teardown publishes nothing.
    pub fn detach(&self, company: &CompanyId, user: &str) -> bool {
        self.people
            .lock()
            .expect("presence registry poisoned")
            .remove(&(company.clone(), user.to_string()))
            .is_some()
    }

    /// Everyone whose lease is still good, newest first.
    ///
    /// Expired entries are filtered here rather than swept on a timer: reads
    /// are the only thing that cares, so a lapsed lease costs a comparison
    /// instead of a background task. [`Self::sweep`] exists for the memory,
    /// not for the correctness.
    pub fn list(&self, company: &CompanyId, now_millis: u64) -> Vec<PresenceView> {
        let people = self.people.lock().expect("presence registry poisoned");
        let mut out: Vec<PresenceView> = people
            .iter()
            .filter(|((id, _), peer)| id == company && !expired(peer.last_beat_millis, now_millis))
            .map(|((_, user), peer)| PresenceView {
                user_id: user.clone(),
                status: peer.status,
                at_millis: peer.last_beat_millis,
            })
            .collect();
        out.sort_by(|a, b| {
            b.at_millis
                .cmp(&a.at_millis)
                .then(a.user_id.cmp(&b.user_id))
        });
        out
    }

    /// Forgets every lapsed lease, in every company.
    ///
    /// Purely to bound memory on a long-lived host — [`Self::list`] already
    /// ignores what this removes, so nothing observable changes. Returns how
    /// many went, for a log line.
    pub fn sweep(&self, now_millis: u64) -> usize {
        let mut people = self.people.lock().expect("presence registry poisoned");
        let before = people.len();
        people.retain(|_, peer| !expired(peer.last_beat_millis, now_millis));
        before - people.len()
    }
}

/// Whether a lease taken at `last_beat` has lapsed by `now`.
///
/// Clock-skew tolerant in the one direction that matters: a beat stamped in the
/// future (two hosts disagreeing by a second) is not expired, because
/// `saturating_sub` floors the age at zero rather than wrapping it to
/// eighteen quintillion milliseconds and marking a live peer as gone.
fn expired(last_beat: u64, now: u64) -> bool {
    now.saturating_sub(last_beat) > PRESENCE_TTL_MILLIS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acme() -> CompanyId {
        CompanyId::new("acme")
    }

    #[test]
    fn a_beat_makes_somebody_present() {
        let reg = PresenceRegistry::new();
        assert!(reg.beat(&acme(), "u1", PresenceStatus::Online, 0));
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].user_id, "u1");
        assert_eq!(live[0].status, PresenceStatus::Online);
    }

    /// The 3x margin doing its job: a beat missed entirely still leaves the
    /// person online, so a dot does not flicker on one slow request.
    #[test]
    fn one_missed_beat_does_not_flap_somebody_offline() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        assert_eq!(
            reg.list(&acme(), PRESENCE_HEARTBEAT_MILLIS * 2).len(),
            1,
            "two beats' worth of silence is still within the lease"
        );
    }

    #[test]
    fn a_lapsed_lease_stops_being_present() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        assert!(reg.list(&acme(), PRESENCE_TTL_MILLIS + 1).is_empty());
    }

    /// Crash recovery costs nothing: nobody has to notice the browser died.
    #[test]
    fn a_crashed_console_needs_no_cleanup() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        // No detach — the tab is simply gone.
        assert!(reg.list(&acme(), PRESENCE_TTL_MILLIS + 1).is_empty());
    }

    #[test]
    fn a_clean_disconnect_drops_the_lease_at_once() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        assert!(reg.detach(&acme(), "u1"));
        assert!(reg.list(&acme(), 0).is_empty());
        assert!(
            !reg.detach(&acme(), "u1"),
            "a second teardown changes nothing"
        );
    }

    /// The bus stays quiet while nothing moves: a console beats every minute
    /// whether or not anything changed, and republishing that to everyone would
    /// be one frame per person per minute for no visible difference.
    #[test]
    fn an_unchanged_beat_reports_no_change() {
        let reg = PresenceRegistry::new();
        assert!(
            reg.beat(&acme(), "u1", PresenceStatus::Online, 0),
            "arrival"
        );
        assert!(
            !reg.beat(
                &acme(),
                "u1",
                PresenceStatus::Online,
                PRESENCE_HEARTBEAT_MILLIS
            ),
            "a routine renewal is not news"
        );
        assert!(
            reg.beat(
                &acme(),
                "u1",
                PresenceStatus::Away,
                PRESENCE_HEARTBEAT_MILLIS
            ),
            "a status change is"
        );
    }

    #[test]
    fn a_beat_after_the_lease_lapsed_is_an_arrival_again() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        assert!(
            reg.beat(
                &acme(),
                "u1",
                PresenceStatus::Online,
                PRESENCE_TTL_MILLIS + 1
            ),
            "otherwise somebody who lapsed reappears with no frame announcing it"
        );
    }

    /// One tenant must never see another's people, and the map is shared.
    #[test]
    fn presence_does_not_leak_between_companies() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        reg.beat(&CompanyId::new("other"), "u2", PresenceStatus::Online, 0);
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].user_id, "u1");
    }

    #[test]
    fn the_sweep_frees_only_what_list_already_ignores() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", PresenceStatus::Online, 0);
        reg.beat(&acme(), "u2", PresenceStatus::Online, PRESENCE_TTL_MILLIS);
        let now = PRESENCE_TTL_MILLIS + 1;
        let visible_before = reg.list(&acme(), now);
        assert_eq!(reg.sweep(now), 1);
        assert_eq!(reg.list(&acme(), now), visible_before);
    }

    /// A beat stamped slightly in the future — two hosts whose clocks disagree
    /// — must not read as expired. Wrapping subtraction would make the age
    /// enormous and mark a live person gone.
    #[test]
    fn a_beat_from_a_slightly_fast_clock_is_not_expired() {
        assert!(!expired(1_000, 0));
    }

    #[test]
    fn every_status_has_a_stable_wire_word() {
        assert_eq!(PresenceStatus::Online.as_str(), "online");
        assert_eq!(PresenceStatus::Away.as_str(), "away");
        assert_eq!(PresenceStatus::Offline.as_str(), "offline");
        assert_eq!(
            serde_json::to_string(&PresenceStatus::Away).expect("serialize"),
            "\"away\""
        );
    }

    /// Forward-compatibility is not wanted here: an unknown status must be a
    /// refusal, not a silently-stored string a reader cannot render.
    #[test]
    fn an_unknown_status_is_refused() {
        assert!(serde_json::from_str::<PresenceStatus>("\"invisible\"").is_err());
    }
}
