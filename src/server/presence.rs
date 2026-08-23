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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::ports::types::CompanyId;

/// How often a live console announces itself.
pub const PRESENCE_HEARTBEAT_MILLIS: u64 = 60_000;

/// How long an announcement stays good for.
///
/// Three times [`PRESENCE_HEARTBEAT_MILLIS`], so a single missed beat — a
/// slow request, a moment of packet loss, a tab that was throttled while
/// backgrounded — does not flap somebody offline and back.
pub const PRESENCE_TTL_MILLIS: u64 = 3 * PRESENCE_HEARTBEAT_MILLIS;

/// The most consoles one person may hold a lease on at once, per company.
///
/// Bounds a real memory-growth vector: a `consoleId` is client-supplied and
/// otherwise unbounded, so a buggy console minting a fresh one on every
/// reconnect — or a member deliberately hammering the route — would otherwise
/// grow this map forever, since an expired lease is only ever *hidden* from
/// reads (`list`, `aggregate`) rather than removed until [`PresenceRegistry::sweep`]
/// next runs. `beat` enforces this cap directly rather than relying on the
/// sweep's cadence, which bounds the *worst case* between sweeps rather than
/// the sweep interval itself. Comfortably above any real browser's tab count
/// (issue: "Bound client-supplied console leases").
const MAX_CONSOLES_PER_PERSON: usize = 16;

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

/// The most present of two statuses — the aggregation rule for a person with
/// more than one open console.
///
/// `Online` beats `Away` beats nothing at all, so a person reading in one tab
/// while idle in another still shows as present: the honest answer to "is this
/// person here" is "yes, in at least one of their consoles," never the
/// gloomiest tab's guess.
fn most_present(a: PresenceStatus, b: PresenceStatus) -> PresenceStatus {
    use PresenceStatus::*;
    match (a, b) {
        (Online, _) | (_, Online) => Online,
        (Away, _) | (_, Away) => Away,
        _ => Offline,
    }
}

/// Who is currently present, per company.
///
/// Peer state is deliberately thin — a status and a timestamp, nothing else.
/// No cursor, no current room, no last-read: which channel somebody is looking
/// at is not a fact their colleagues need, and carrying it would turn a
/// presence dot into activity tracking.
///
/// # A lease is per console, not per person
///
/// The same signed-in human commonly has more than one tab open. Keying solely
/// on `(company, user)` made closing *any one* of them delete the lease every
/// other tab was still renewing, so the person flapped offline until their next
/// heartbeat healed it (up to a full [`PRESENCE_HEARTBEAT_MILLIS`]). The inner
/// map keys on `(company, user)` as before; the value is now every console that
/// user currently has open, so a departure only ever removes the console that
/// actually left.
#[derive(Debug, Default)]
pub struct PresenceRegistry {
    people: Mutex<HashMap<(CompanyId, String), HashMap<String, Peer>>>,
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

    /// Records one console's heartbeat, and says whether it changed anything a
    /// watcher would care about.
    ///
    /// Returning "did this change" is what keeps the bus quiet: a console beats
    /// every minute whether or not anything moved, and republishing an
    /// unchanged `online` to every other console once a minute per person is
    /// pure noise. A frame goes out when the person's *aggregate* status
    /// — [`most_present`] across every console they have open — arrives or
    /// changes, which is exactly when a dot would move.
    ///
    /// An expired lease counts as an arrival, so somebody who was away long
    /// enough to lapse is re-announced rather than silently reappearing on the
    /// next reader's poll.
    pub fn beat(
        &self,
        company: &CompanyId,
        user: &str,
        console: &str,
        status: PresenceStatus,
        now_millis: u64,
    ) -> bool {
        let mut people = self.people.lock().expect("presence registry poisoned");
        let key = (company.clone(), user.to_string());
        let consoles = people.entry(key).or_default();
        let before = aggregate(consoles, now_millis);
        // A genuinely new console (not a renewal of one already tracked) past
        // the cap evicts the stalest entry first — expired ones before live
        // ones, then oldest `last_beat_millis` — so an unbounded stream of
        // fresh `consoleId`s cannot grow this map without limit. See
        // `MAX_CONSOLES_PER_PERSON`.
        if !consoles.contains_key(console)
            && consoles.len() >= MAX_CONSOLES_PER_PERSON
            && let Some(stalest) = consoles
                .iter()
                .min_by_key(|(_, peer)| peer.last_beat_millis)
                .map(|(id, _)| id.clone())
        {
            consoles.remove(&stalest);
        }
        consoles.insert(
            console.to_string(),
            Peer {
                status,
                last_beat_millis: now_millis,
            },
        );
        let after = aggregate(consoles, now_millis);
        before != after
    }

    /// Drops one console's lease immediately — a clean disconnect.
    ///
    /// Worth having even though the TTL would get there eventually: a person
    /// who closes a tab should not linger as online for three minutes, and the
    /// browser can say so on the way out. Only removes the departing console —
    /// a colleague's other open tabs keep their own leases, so closing one does
    /// not drop the others.
    ///
    /// Returns the person's **new aggregate status**, and only when it
    /// actually changed: `None` for a duplicate teardown, or for a departure
    /// that leaves another console whose status already matched the
    /// aggregate (an away tab closing while an online one is still live
    /// changes nothing an observer could see). When the departing console was
    /// carrying the aggregate up — an online tab closing while only an away
    /// one remains — this reports the *downgraded* status (`Away`), not
    /// `Offline`, so the caller publishes what the person now looks like
    /// rather than a false "gone" a moment before their away tab's next
    /// heartbeat corrects it. `Offline` is reported only when nobody is left.
    pub fn detach(
        &self,
        company: &CompanyId,
        user: &str,
        console: &str,
        now_millis: u64,
    ) -> Option<PresenceStatus> {
        let mut people = self.people.lock().expect("presence registry poisoned");
        let key = (company.clone(), user.to_string());
        let consoles = people.get_mut(&key)?;
        let before = aggregate(consoles, now_millis);
        consoles.remove(console)?;
        let after = aggregate(consoles, now_millis);
        if consoles.is_empty() {
            people.remove(&key);
        }
        if after == before {
            None
        } else {
            Some(after.unwrap_or(PresenceStatus::Offline))
        }
    }

    /// Everyone whose lease is still good, newest first.
    ///
    /// Expired entries are filtered here rather than swept on a timer: reads
    /// are the only thing that cares, so a lapsed lease costs a comparison
    /// instead of a background task. [`Self::sweep`] exists for the memory,
    /// not for the correctness. A person with several open consoles is one row
    /// here — [`most_present`] across them, timestamped by whichever renewed
    /// most recently.
    pub fn list(&self, company: &CompanyId, now_millis: u64) -> Vec<PresenceView> {
        let people = self.people.lock().expect("presence registry poisoned");
        let mut out: Vec<PresenceView> = people
            .iter()
            .filter(|((id, _), _)| id == company)
            .filter_map(|((_, user), consoles)| {
                let live: Vec<&Peer> = consoles
                    .values()
                    .filter(|peer| !expired(peer.last_beat_millis, now_millis))
                    .collect();
                if live.is_empty() {
                    return None;
                }
                let status = live
                    .iter()
                    .map(|peer| peer.status)
                    .reduce(most_present)
                    .expect("checked non-empty above");
                let at_millis = live
                    .iter()
                    .map(|peer| peer.last_beat_millis)
                    .max()
                    .expect("checked non-empty above");
                Some(PresenceView {
                    user_id: user.clone(),
                    status,
                    at_millis,
                })
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
    /// many console leases went, for a log line.
    pub fn sweep(&self, now_millis: u64) -> usize {
        let mut people = self.people.lock().expect("presence registry poisoned");
        let mut removed = 0;
        people.retain(|_, consoles| {
            let before = consoles.len();
            consoles.retain(|_, peer| !expired(peer.last_beat_millis, now_millis));
            removed += before - consoles.len();
            !consoles.is_empty()
        });
        removed
    }
}

/// The aggregate status across every live console a person currently has open,
/// or `None` when none of them are (an empty map, or every lease expired).
fn aggregate(consoles: &HashMap<String, Peer>, now_millis: u64) -> Option<PresenceStatus> {
    consoles
        .values()
        .filter(|peer| !expired(peer.last_beat_millis, now_millis))
        .map(|peer| peer.status)
        .reduce(most_present)
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

/// Periodically forgets lapsed leases across the whole process (issue: "Bound
/// client-supplied console leases").
///
/// [`PresenceRegistry::sweep`] existed from the start but had no production
/// caller — its only caller was a unit test. That was not silently unsafe
/// (`list` already filters an expired lease out of every read, so nothing
/// downstream ever saw a stale one), but the backing map itself only ever
/// grew: a crashed tab that never sent `DELETE`, or a client minting fresh
/// `consoleId`s, both left dead entries nothing ever removed. Deliberately not
/// folded into [`crate::runtime::maintenance::MaintenanceTicker`] — that ticker
/// is scoped to registered *companies* and drives per-company retirement
/// through [`crate::CompanyRuntime`]; this registry is host-global and keyed by
/// neither, so it gets its own always-on task, spawned once at boot the same
/// way. [`MAX_CONSOLES_PER_PERSON`] bounds the worst case *per person* between
/// sweeps; this is what reclaims the memory for everyone once a lease expires.
pub struct PresenceSweeper {
    registry: Arc<PresenceRegistry>,
}

impl PresenceSweeper {
    pub fn new(registry: Arc<PresenceRegistry>) -> Self {
        Self { registry }
    }

    /// Runs until `shutdown` is notified, sweeping once per [`PRESENCE_TTL_MILLIS`]
    /// — lapsed-but-unswept memory is bounded by one TTL's worth of leases, not
    /// by how long the process has been up.
    pub fn spawn(self, shutdown: Arc<Notify>) -> JoinHandle<()> {
        tokio::spawn(async move {
            // Built once and pinned, not rebuilt inside the loop — see
            // `MaintenanceTicker::spawn`'s identical comment for why a
            // freshly-built `Notified` each iteration can miss a shutdown that
            // arrives mid-sweep.
            let notified = shutdown.notified();
            tokio::pin!(notified);
            loop {
                tokio::select! {
                    _ = &mut notified => break,
                    _ = tokio::time::sleep(Duration::from_millis(PRESENCE_TTL_MILLIS)) => {
                        self.registry.sweep(crate::ports::now_millis());
                    }
                }
            }
        })
    }
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
        assert!(reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0));
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
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        assert_eq!(
            reg.list(&acme(), PRESENCE_HEARTBEAT_MILLIS * 2).len(),
            1,
            "two beats' worth of silence is still within the lease"
        );
    }

    #[test]
    fn a_lapsed_lease_stops_being_present() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        assert!(reg.list(&acme(), PRESENCE_TTL_MILLIS + 1).is_empty());
    }

    /// Crash recovery costs nothing: nobody has to notice the browser died.
    #[test]
    fn a_crashed_console_needs_no_cleanup() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        // No detach — the tab is simply gone.
        assert!(reg.list(&acme(), PRESENCE_TTL_MILLIS + 1).is_empty());
    }

    #[test]
    fn a_clean_disconnect_drops_the_lease_at_once() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        assert_eq!(
            reg.detach(&acme(), "u1", "tab-1", 0),
            Some(PresenceStatus::Offline)
        );
        assert!(reg.list(&acme(), 0).is_empty());
        assert_eq!(
            reg.detach(&acme(), "u1", "tab-1", 0),
            None,
            "a second teardown changes nothing"
        );
    }

    /// The bug this module's header now calls out by name: the same person
    /// with two tabs open must not have one tab's departure log the other one
    /// out. Closing tab 1 must not touch tab 2's lease, and only closing the
    /// last open tab is a real departure worth a frame.
    #[test]
    fn closing_one_tab_does_not_disconnect_another() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        reg.beat(&acme(), "u1", "tab-2", PresenceStatus::Online, 0);

        assert_eq!(
            reg.detach(&acme(), "u1", "tab-1", 0),
            None,
            "tab 2 is still open at the same status, so this is not a real departure"
        );
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1, "still present through tab 2");
        assert_eq!(live[0].status, PresenceStatus::Online);

        assert_eq!(
            reg.detach(&acme(), "u1", "tab-2", 0),
            Some(PresenceStatus::Offline),
            "the last open tab leaving is a real departure"
        );
        assert!(reg.list(&acme(), 0).is_empty());
    }

    /// Two open tabs disagreeing about status — one idle, one active — must
    /// read as the more present of the two: a person reading in one tab while
    /// away in another is still here.
    #[test]
    fn the_most_present_tab_wins_the_aggregate_status() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Away, 0);
        reg.beat(&acme(), "u1", "tab-2", PresenceStatus::Online, 0);
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status, PresenceStatus::Online);
    }

    /// The downgrade case: closing the console that was carrying the
    /// aggregate must report the *new* aggregate (`Away`), not `Offline` —
    /// the person is still here, just not at their most-present console
    /// anymore, and every other viewer's dot should say so at once rather
    /// than waiting for the away tab's next heartbeat to correct a false
    /// "gone".
    #[test]
    fn closing_the_more_present_tab_reports_the_downgraded_status() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-online", PresenceStatus::Online, 0);
        reg.beat(&acme(), "u1", "tab-away", PresenceStatus::Away, 0);

        assert_eq!(
            reg.detach(&acme(), "u1", "tab-online", 0),
            Some(PresenceStatus::Away),
            "the away tab is still open — the aggregate degrades, it does not vanish"
        );
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status, PresenceStatus::Away);
    }

    /// The memory-growth finding: a client that keeps minting fresh
    /// `consoleId`s (a bug in some other console, or a member hammering the
    /// route) must not grow one person's lease set without bound. Past the
    /// cap, a new console evicts the stalest one rather than being appended.
    #[test]
    fn a_flood_of_new_consoles_is_capped_rather_than_grown_forever() {
        let reg = PresenceRegistry::new();
        for i in 0..(MAX_CONSOLES_PER_PERSON * 3) {
            reg.beat(
                &acme(),
                "u1",
                &format!("console-{i}"),
                PresenceStatus::Online,
                i as u64,
            );
        }
        let count = reg
            .people
            .lock()
            .unwrap()
            .get(&(acme(), "u1".to_string()))
            .map(|consoles| consoles.len())
            .unwrap_or(0);
        assert!(
            count <= MAX_CONSOLES_PER_PERSON,
            "expected at most {MAX_CONSOLES_PER_PERSON} tracked consoles, found {count}"
        );
        // And the person is still (correctly) present — capping evicts the
        // stalest lease, it does not stop tracking the person altogether.
        assert_eq!(
            reg.list(&acme(), (MAX_CONSOLES_PER_PERSON * 3) as u64)
                .len(),
            1
        );
    }

    /// A console renewing its own existing lease must never be evicted by its
    /// own renewal, even sitting exactly at the cap — the cap only ever makes
    /// room for a *new* console id.
    #[test]
    fn renewing_an_existing_console_at_the_cap_never_evicts_itself() {
        let reg = PresenceRegistry::new();
        for i in 0..MAX_CONSOLES_PER_PERSON {
            reg.beat(
                &acme(),
                "u1",
                &format!("console-{i}"),
                PresenceStatus::Online,
                0,
            );
        }
        // Renew the very first console many times — it must still be present
        // afterward, not evicted as "stalest" by its own renewals.
        for tick in 1..10 {
            reg.beat(&acme(), "u1", "console-0", PresenceStatus::Online, tick);
        }
        let people = reg.people.lock().unwrap();
        let consoles = people.get(&(acme(), "u1".to_string())).unwrap();
        assert!(consoles.contains_key("console-0"));
        assert_eq!(consoles.len(), MAX_CONSOLES_PER_PERSON);
    }

    /// The bus stays quiet while nothing moves: a console beats every minute
    /// whether or not anything changed, and republishing that to everyone would
    /// be one frame per person per minute for no visible difference.
    #[test]
    fn an_unchanged_beat_reports_no_change() {
        let reg = PresenceRegistry::new();
        assert!(
            reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0),
            "arrival"
        );
        assert!(
            !reg.beat(
                &acme(),
                "u1",
                "tab-1",
                PresenceStatus::Online,
                PRESENCE_HEARTBEAT_MILLIS
            ),
            "a routine renewal is not news"
        );
        assert!(
            reg.beat(
                &acme(),
                "u1",
                "tab-1",
                PresenceStatus::Away,
                PRESENCE_HEARTBEAT_MILLIS
            ),
            "a status change is"
        );
    }

    #[test]
    fn a_beat_after_the_lease_lapsed_is_an_arrival_again() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        assert!(
            reg.beat(
                &acme(),
                "u1",
                "tab-1",
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
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        reg.beat(
            &CompanyId::new("other"),
            "u2",
            "tab-1",
            PresenceStatus::Online,
            0,
        );
        let live = reg.list(&acme(), 0);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].user_id, "u1");
    }

    #[test]
    fn the_sweep_frees_only_what_list_already_ignores() {
        let reg = PresenceRegistry::new();
        reg.beat(&acme(), "u1", "tab-1", PresenceStatus::Online, 0);
        reg.beat(
            &acme(),
            "u2",
            "tab-1",
            PresenceStatus::Online,
            PRESENCE_TTL_MILLIS,
        );
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
