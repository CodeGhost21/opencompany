//! Approvals, in both directions — which are not the same shape.
//!
//! ## Outbound: this host does **not** implement `session/request_permission`
//!
//! ACP's permission model assumes a turn that *suspends*: the agent asks, the
//! client answers, the same tool call proceeds. This host's approvals do not
//! work that way, and the difference is structural rather than a matter of
//! plumbing.
//!
//! Read `harness/policy.rs`. OpenHuman resolves `RequireApproval` **fail-closed
//! and inline**: it blocks the call and feeds the model a refusal string. *The
//! call is gone.* There is no suspended continuation to resume. The projected
//! effect is parked after the turn, the turn completes with a reply, and a
//! human later resolves it — which mints a single-use grant and **re-dispatches
//! the agent**. That is a new turn.
//!
//! So by the time an approval exists to ask about, `session/prompt` has already
//! returned its `stopReason`. There is no point at which a blocking
//! `session/request_permission` could be issued, and implementing one would
//! mean it either never fires or fires against a turn that has ended.
//!
//! Instead a park is surfaced as `_meta` on the tool call (see
//! [`map`](super::map)) plus a session-level notification carrying the approval
//! id, and the client resolves it over the existing REST surface — which the
//! desktop already has, because it runs the same console.
//!
//! ## Inbound: this direction genuinely *is* synchronous
//!
//! When this host is the ACP **client** — driving a runner, or a local harness
//! that really does suspend, as `claude-agent-acp` does — a
//! `session/request_permission` arrives as a request and must be answered.
//! [`PendingPermissions`] holds it while a human decides.
//!
//! Stated plainly, because it reads as an inconsistency until you see why:
//! **this host is a synchronous permission client and an asynchronous
//! permission server.** That is not a design preference; it is what the two
//! engines actually do.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// How long an inbound permission request is held before it is answered for the
/// person who never came back.
///
/// A bound is not optional. The request is a live JSON-RPC call holding a
/// socket, and a harness blocked on it is blocked forever — so an unanswered
/// prompt would pin a runner and its worktree indefinitely. Long enough for
/// someone to notice a notification and decide; short enough that a laptop
/// closed at the wrong moment frees the runner the same day.
pub const PERMISSION_TTL_MILLIS: u64 = 30 * 60 * 1000;

/// A session-level notification telling a client an effect was parked.
///
/// Carries the approval id, because that is what the client resolves against
/// over REST. Sent as `_meta` on a `session/update` rather than as a new
/// protocol method: a conforming client that has never heard of OpenCompany
/// ignores it, which is exactly what `_meta` is for.
pub fn parked_notification(session_id: &str, approval_id: &str, summary: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                // No ACP variant means "a human must decide something". The
                // closest honest carrier is a session-info update whose `_meta`
                // says what actually happened.
                "sessionUpdate": "session_info_update",
                "_meta": {
                    "opencompany/approval": {
                        "id": approval_id,
                        "summary": summary,
                        // Where to resolve it. Told rather than assumed: a
                        // third-party ACP client has no reason to know this
                        // host's REST shape.
                        "resolve": "POST /api/v1/companies/{company}/approvals/{id}",
                    }
                },
            },
        },
    })
}

/// How a held permission request was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionOutcome {
    /// A person chose an option.
    Selected,
    /// Nobody answered in time. ACP's own `cancelled` outcome, which a harness
    /// already knows how to handle — unlike a synthetic refusal, which it would
    /// report to the model as a decision someone made.
    TimedOut,
}

/// One inbound `session/request_permission` awaiting a human.
#[derive(Clone, Debug)]
pub struct HeldPermission {
    pub request_id: String,
    pub session_id: String,
    pub asked_at_millis: u64,
    /// The option ids the agent offered. An answer must be one of these.
    pub options: Vec<String>,
}

impl HeldPermission {
    fn expired(&self, now_millis: u64) -> bool {
        now_millis.saturating_sub(self.asked_at_millis) >= PERMISSION_TTL_MILLIS
    }
}

/// Inbound permission requests this host is holding open.
#[derive(Debug, Default)]
pub struct PendingPermissions {
    held: Mutex<HashMap<String, HeldPermission>>,
}

impl PendingPermissions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hold(&self, permission: HeldPermission) {
        self.held
            .lock()
            .expect("pending permissions poisoned")
            .insert(permission.request_id.clone(), permission);
    }

    pub fn get(&self, request_id: &str) -> Option<HeldPermission> {
        self.held
            .lock()
            .expect("pending permissions poisoned")
            .get(request_id)
            .cloned()
    }

    /// Answers a held request with an option the agent offered.
    ///
    /// Refuses an option that was never offered. Echoing back an arbitrary id
    /// would be answering a question the agent did not ask, and a harness that
    /// cannot match it may do anything from erroring to picking a default.
    pub fn resolve(&self, request_id: &str, option_id: &str) -> Option<Value> {
        let mut held = self.held.lock().expect("pending permissions poisoned");
        let permission = held.get(request_id)?;
        if !permission.options.iter().any(|o| o == option_id) {
            return None;
        }
        held.remove(request_id);
        Some(json!({
            "outcome": { "outcome": "selected", "optionId": option_id }
        }))
    }

    /// Answers everything nobody came back for, freeing whatever was blocked.
    ///
    /// Returns `(request_id, response)` pairs for the caller to send.
    pub fn expire(&self, now_millis: u64) -> Vec<(String, Value)> {
        let mut held = self.held.lock().expect("pending permissions poisoned");
        let stale: Vec<String> = held
            .iter()
            .filter(|(_, p)| p.expired(now_millis))
            .map(|(id, _)| id.clone())
            .collect();
        stale
            .into_iter()
            .map(|id| {
                held.remove(&id);
                // ACP's own cancellation outcome, not a synthetic rejection: a
                // harness told "rejected" reports a decision to the model that
                // nobody made.
                (id, json!({ "outcome": { "outcome": "cancelled" } }))
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.held
            .lock()
            .expect("pending permissions poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Shared with the ACP client lane.
pub type SharedPermissions = Arc<PendingPermissions>;

#[cfg(test)]
mod test {
    use super::*;

    fn held(id: &str, at: u64) -> HeldPermission {
        HeldPermission {
            request_id: id.to_string(),
            session_id: "sess-1".to_string(),
            asked_at_millis: at,
            options: vec!["allow".to_string(), "reject".to_string()],
        }
    }

    #[test]
    fn a_parked_effect_tells_the_client_how_to_resolve_it() {
        let note = parked_notification("sess-1", "appr-7", "Send mail to 4 people");
        assert_eq!(note["method"], "session/update");
        // A notification: an `id` would have a conforming client try to reply.
        assert!(note.get("id").is_none());

        let approval = &note["params"]["update"]["_meta"]["opencompany/approval"];
        assert_eq!(approval["id"], "appr-7");
        assert_eq!(approval["summary"], "Send mail to 4 people");
        // A third-party client has no reason to know this host's REST shape, so
        // it is told rather than assumed.
        assert!(
            approval["resolve"]
                .as_str()
                .unwrap()
                .contains("/approvals/")
        );
    }

    #[test]
    fn a_held_request_is_answered_with_an_offered_option() {
        let pending = PendingPermissions::new();
        pending.hold(held("req-1", 0));

        let answer = pending
            .resolve("req-1", "allow")
            .expect("an offered option");
        assert_eq!(answer["outcome"]["outcome"], "selected");
        assert_eq!(answer["outcome"]["optionId"], "allow");
        // Answered once: a second resolve has nothing to answer, so a duplicate
        // click cannot send two replies to one JSON-RPC id.
        assert!(pending.resolve("req-1", "allow").is_none());
        assert!(pending.is_empty());
    }

    #[test]
    fn an_option_the_agent_never_offered_is_refused() {
        // Echoing back an arbitrary id answers a question the agent did not
        // ask; a harness that cannot match it may do anything.
        let pending = PendingPermissions::new();
        pending.hold(held("req-1", 0));

        assert!(pending.resolve("req-1", "delete-everything").is_none());
        assert!(!pending.is_empty(), "the request is still open");
    }

    #[test]
    fn an_unanswered_request_is_cancelled_rather_than_held_forever() {
        // The request is a live JSON-RPC call holding a socket, and the harness
        // on the other end is blocked on it. Without a bound, a laptop closed
        // at the wrong moment pins a runner and its worktree indefinitely.
        let pending = PendingPermissions::new();
        pending.hold(held("req-1", 0));

        assert!(
            pending.expire(PERMISSION_TTL_MILLIS - 1).is_empty(),
            "not yet"
        );

        let expired = pending.expire(PERMISSION_TTL_MILLIS);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, "req-1");
        // ACP's own cancellation, not a synthetic rejection: "rejected" would
        // report a decision to the model that nobody made.
        assert_eq!(expired[0].1["outcome"]["outcome"], "cancelled");
        assert!(pending.is_empty());
    }

    #[test]
    fn expiry_only_takes_the_requests_that_are_actually_stale() {
        let pending = PendingPermissions::new();
        pending.hold(held("old", 0));
        pending.hold(held("fresh", PERMISSION_TTL_MILLIS));

        let expired = pending.expire(PERMISSION_TTL_MILLIS);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].0, "old");
        assert!(pending.get("fresh").is_some());
    }
}
