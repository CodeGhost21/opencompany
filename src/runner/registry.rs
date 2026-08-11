//! Which runners are attached right now, and what they can do.
//!
//! ## Presence is the status
//!
//! There is no "are you alive?" call, because its answer is only ever as fresh
//! as the last heartbeat anyway — so the heartbeat *is* the answer. A runner
//! that stops sending one goes stale and stops being scheduled to.
//!
//! The cost is honest and worth stating: a runner that dies without closing its
//! socket keeps looking live for up to [`PRESENCE_TTL_MILLIS`]. Shortening the
//! TTL does not fix that, it only trades a stale-runner window for false
//! evictions of a runner on a slow link. The real mitigation is a per-dispatch
//! deadline, which belongs with dispatch rather than here.
//!
//! ## At most one live instance per scope
//!
//! Two copies of a desktop must not both take a company's work. On a duplicate
//! claim the **new** connection wins and the old is evicted — buzz's rule, and
//! the right way round: the newer connection is the one someone just made, and
//! the older is most often a dead socket nobody has noticed yet.
//!
//! This lives on `AppState`, not on a `CompanyRuntime`: runners are
//! process-scoped, and a company rebuild would otherwise drop every attached
//! runner for reasons that have nothing to do with them.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

/// How often a runner is expected to report in.
pub const HEARTBEAT_MILLIS: u64 = 30_000;

/// How long after its last heartbeat a runner is still considered live.
///
/// Three missed beats. Two is too tight for a laptop that briefly slept; more
/// widens the window in which work is scheduled to something already gone.
pub const PRESENCE_TTL_MILLIS: u64 = 90_000;

/// One missed beat is a slow link; three is a machine that has gone.
///
/// A `const` assertion rather than a test, because it is a fact about two
/// constants and nothing at runtime can change it — a test would only re-check
/// arithmetic the compiler already did. Tightening the TTL below three beats
/// makes the lane evict laptops that briefly slept, so this fails the build
/// rather than letting it be tuned by accident.
const _: () = assert!(PRESENCE_TTL_MILLIS >= HEARTBEAT_MILLIS * 3);

/// A harness a runner says it can drive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessOffer {
    pub id: String,
    /// Only a *ready* harness is worth scheduling to — installed-but-signed-out
    /// is reported so the host can say why nothing is being dispatched.
    pub ready: bool,
}

/// What a runner claims it can do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerCapabilities {
    pub harnesses: Vec<HarnessOffer>,
    pub max_parallel: u32,
}

/// One attached runner.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerStatus {
    pub runner_id: String,
    /// The owner whose attestation admitted it.
    pub owner: String,
    /// The `(company, agent)` scopes it claims.
    pub scopes: Vec<String>,
    pub capabilities: RunnerCapabilities,
    pub last_seen_millis: u64,
    /// The connection it arrived on, so an eviction can name what to close.
    pub connection: String,
}

impl RunnerStatus {
    pub fn is_live(&self, now_millis: u64) -> bool {
        now_millis.saturating_sub(self.last_seen_millis) < PRESENCE_TTL_MILLIS
    }

    /// Whether this runner can actually take work: live, and holding at least
    /// one ready harness. A runner whose harnesses are all signed out is
    /// attached and useless, and scheduling to it would fail every task.
    pub fn can_take_work(&self, now_millis: u64) -> bool {
        self.is_live(now_millis) && self.capabilities.harnesses.iter().any(|h| h.ready)
    }
}

/// What admitting a runner displaced, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    /// A previously-attached runner now evicted, and the connection to close.
    pub evicted: Option<String>,
}

/// Every runner attached to this host process.
#[derive(Debug, Default)]
pub struct RunnerRegistry {
    runners: Mutex<HashMap<String, RunnerStatus>>,
}

impl RunnerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attaches a runner, evicting whatever held its scopes.
    pub fn admit(&self, status: RunnerStatus) -> Admission {
        let mut runners = self.runners.lock().expect("runner registry poisoned");

        // The new connection wins. The old one is most often a socket nobody
        // has noticed is dead; the new one is a machine someone just started.
        let displaced = runners
            .values()
            .find(|existing| {
                existing.runner_id != status.runner_id
                    && existing.scopes.iter().any(|s| status.scopes.contains(s))
            })
            .map(|existing| existing.connection.clone());

        if let Some(connection) = &displaced {
            runners.retain(|_, existing| &existing.connection != connection);
        }
        runners.insert(status.runner_id.clone(), status);
        Admission { evicted: displaced }
    }

    /// Records a heartbeat. Returns whether the runner was known.
    pub fn beat(&self, runner_id: &str, now_millis: u64) -> bool {
        let mut runners = self.runners.lock().expect("runner registry poisoned");
        match runners.get_mut(runner_id) {
            Some(status) => {
                status.last_seen_millis = now_millis;
                true
            }
            // A heartbeat from something never admitted. Reported rather than
            // silently creating an entry: a runner that reconnects must go
            // through the handshake again, or presence would be a way to attach
            // without ever proving an identity.
            None => false,
        }
    }

    pub fn detach(&self, runner_id: &str) {
        self.runners
            .lock()
            .expect("runner registry poisoned")
            .remove(runner_id);
    }

    /// Runners that could take work for `scope` right now.
    pub fn available_for(&self, scope: &str, now_millis: u64) -> Vec<RunnerStatus> {
        self.runners
            .lock()
            .expect("runner registry poisoned")
            .values()
            .filter(|r| r.scopes.iter().any(|s| s == scope) && r.can_take_work(now_millis))
            .cloned()
            .collect()
    }

    /// Everything attached, live or not — the operator-facing list.
    ///
    /// Includes stale entries deliberately: "a runner was here and stopped
    /// reporting" is the single most useful thing to show someone wondering
    /// why nothing is running.
    pub fn list(&self) -> Vec<RunnerStatus> {
        self.runners
            .lock()
            .expect("runner registry poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Drops runners that have gone quiet. Returns how many.
    pub fn sweep(&self, now_millis: u64) -> usize {
        let mut runners = self.runners.lock().expect("runner registry poisoned");
        let before = runners.len();
        runners.retain(|_, status| status.is_live(now_millis));
        before - runners.len()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn runner(id: &str, scope: &str, connection: &str, seen: u64) -> RunnerStatus {
        RunnerStatus {
            runner_id: id.to_string(),
            owner: "owner-1".to_string(),
            scopes: vec![scope.to_string()],
            capabilities: RunnerCapabilities {
                harnesses: vec![HarnessOffer {
                    id: "claude".to_string(),
                    ready: true,
                }],
                max_parallel: 2,
            },
            last_seen_millis: seen,
            connection: connection.to_string(),
        }
    }

    #[test]
    fn a_fresh_runner_can_take_work() {
        let registry = RunnerRegistry::new();
        assert_eq!(
            registry.admit(runner("r1", "acme::ceo", "c1", 0)).evicted,
            None
        );
        assert_eq!(registry.available_for("acme::ceo", 0).len(), 1);
    }

    #[test]
    fn a_second_runner_on_one_scope_evicts_the_first() {
        // Two copies of a desktop must not both take a company's work. The
        // newer connection wins: it is the one someone just made, and the older
        // is most often a socket nobody has noticed is dead.
        let registry = RunnerRegistry::new();
        registry.admit(runner("r1", "acme::ceo", "conn-1", 0));

        let admission = registry.admit(runner("r2", "acme::ceo", "conn-2", 0));
        assert_eq!(admission.evicted.as_deref(), Some("conn-1"));

        let available = registry.available_for("acme::ceo", 0);
        assert_eq!(available.len(), 1, "exactly one holds the scope");
        assert_eq!(available[0].runner_id, "r2");
    }

    #[test]
    fn runners_on_different_scopes_coexist() {
        let registry = RunnerRegistry::new();
        assert_eq!(
            registry.admit(runner("r1", "acme::ceo", "c1", 0)).evicted,
            None
        );
        assert_eq!(
            registry.admit(runner("r2", "globex::cto", "c2", 0)).evicted,
            None
        );
        assert_eq!(registry.list().len(), 2);
    }

    #[test]
    fn a_runner_that_stops_reporting_stops_being_scheduled_to() {
        let registry = RunnerRegistry::new();
        registry.admit(runner("r1", "acme::ceo", "c1", 0));

        assert_eq!(
            registry
                .available_for("acme::ceo", PRESENCE_TTL_MILLIS - 1)
                .len(),
            1
        );
        assert!(
            registry
                .available_for("acme::ceo", PRESENCE_TTL_MILLIS)
                .is_empty(),
            "a stale runner must not be given work"
        );
    }

    #[test]
    fn a_heartbeat_keeps_a_runner_live() {
        let registry = RunnerRegistry::new();
        registry.admit(runner("r1", "acme::ceo", "c1", 0));
        assert!(registry.beat("r1", PRESENCE_TTL_MILLIS - 1));
        assert_eq!(
            registry
                .available_for("acme::ceo", PRESENCE_TTL_MILLIS + 1)
                .len(),
            1
        );
    }

    #[test]
    fn a_heartbeat_from_an_unknown_runner_does_not_attach_it() {
        // Otherwise presence is a way to attach without ever proving an
        // identity — a runner that reconnects must handshake again.
        let registry = RunnerRegistry::new();
        assert!(!registry.beat("never-seen", 0));
        assert!(registry.list().is_empty());
    }

    #[test]
    fn a_runner_with_no_ready_harness_is_attached_but_not_scheduled_to() {
        // Attached and useless is a real state — every harness signed out — and
        // scheduling to it would fail every task. It stays listed so an
        // operator can see *why* nothing is running.
        let registry = RunnerRegistry::new();
        let mut idle = runner("r1", "acme::ceo", "c1", 0);
        idle.capabilities.harnesses[0].ready = false;
        registry.admit(idle);

        assert!(registry.available_for("acme::ceo", 0).is_empty());
        assert_eq!(registry.list().len(), 1, "still visible to an operator");
    }

    #[test]
    fn the_list_keeps_stale_runners_so_an_operator_can_see_them() {
        let registry = RunnerRegistry::new();
        registry.admit(runner("r1", "acme::ceo", "c1", 0));

        let listed = registry.list();
        assert_eq!(listed.len(), 1);
        assert!(
            !listed[0].is_live(PRESENCE_TTL_MILLIS),
            "stale, and still shown"
        );
    }

    #[test]
    fn a_sweep_drops_only_the_quiet_ones() {
        let registry = RunnerRegistry::new();
        registry.admit(runner("stale", "acme::ceo", "c1", 0));
        registry.admit(runner("fresh", "globex::cto", "c2", PRESENCE_TTL_MILLIS));

        assert_eq!(registry.sweep(PRESENCE_TTL_MILLIS), 1);
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.list()[0].runner_id, "fresh");
    }
}
