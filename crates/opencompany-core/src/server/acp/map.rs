//! Turning what this host streams into ACP `session/update` notifications.
//!
//! ## Why this reads the bus and not `AgentProgress`
//!
//! The obvious source is OpenHuman's own `AgentProgress`, which carries far
//! more. Two reasons it is the wrong one:
//!
//! - It only exists under `feature = "openhuman"`, and this surface has to
//!   compile in the default build. A host without the vendored runtime still
//!   serves ACP; it simply has less to say.
//! - [`steps::stream_event_from`](crate::harness::steps) is the **scrubbing
//!   boundary**. It is where tool arguments are redacted, where a remote body
//!   is reduced to a shape summary, and where a failure becomes a typed cause.
//!   Reading `AgentProgress` directly to get richer updates would route around
//!   all of that, and the richness would be exactly the raw material it exists
//!   to withhold.
//!
//! So the input here is [`TurnStreamEvent`] — already scrubbed — plus the
//! durable [`CompanyEvent`] journal.
//!
//! ## What is lost, said plainly
//!
//! `stream_event_from` returns `None` for `TextDelta`, so **there is no
//! incremental assistant text on this host**. An ACP client receives one
//! `agent_message_chunk` at the end of the turn, from the durable `AgentReply`.
//! Clients that render token-by-token will look like they have stalled and then
//! finished at once.
//!
//! That is a deliberate posture, not an oversight: `turn_stream`'s own module
//! documentation argues that the bus carries scrubbed projections only. Adding
//! deltas is a change to what this host is willing to stream, and belongs
//! behind a per-company decision rather than in a mapping layer.

use serde_json::{Value, json};

use crate::ports::types::CompanyEvent;
use crate::turn_stream::TurnStreamEvent;

/// One ACP `SessionUpdate`, ready to be wrapped in a `session/update`.
pub type SessionUpdate = Value;

/// Wraps an update in the notification envelope ACP expects.
pub fn notification(session_id: &str, update: SessionUpdate) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session_id, "update": update },
    })
}

/// Maps a live turn frame, or `None` when it has no ACP equivalent.
pub fn from_turn_stream(event: &TurnStreamEvent) -> Option<SessionUpdate> {
    match event.kind {
        "tool_call" => Some(json!({
            "sessionUpdate": "tool_call",
            "toolCallId": event.tool_call_id.clone().unwrap_or_default(),
            "title": event.label.clone().unwrap_or_else(|| "Working".to_string()),
            "status": "pending",
            // `rawInput` is deliberately absent. `detail` is a *redacted*
            // one-liner about the call, not its arguments, and putting it in
            // ACP's raw-arguments field would tell a client it had the real
            // ones — which is how a scrubbed value ends up rendered as truth.
            "_meta": meta(event),
        })),
        "tool_result" => {
            let status = match event.status {
                Some("ok") => "completed",
                Some("error") => "failed",
                // A parked call is neither: the turn stopped, and nothing
                // broke. ACP has no such status, so it maps to `failed` with
                // the truth in `_meta` — see below.
                Some("awaiting_approval") => "failed",
                _ => "in_progress",
            };
            let mut update = json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": event.tool_call_id.clone().unwrap_or_default(),
                "status": status,
                "_meta": meta(event),
            });
            if let Some(result) = &event.result {
                update["content"] = json!([{
                    "type": "content",
                    "content": { "type": "text", "text": result },
                }]);
            }
            Some(update)
        }
        // A marker, with no content: this host never streams reasoning text.
        // Emitting an empty `agent_thought_chunk` would have a client render a
        // blank bubble on every turn, so it is dropped instead.
        "thinking" => None,
        _ => None,
    }
}

/// The OpenCompany-specific facts ACP has no field for.
///
/// `_meta` is the protocol's own escape hatch, and using it is how a
/// conforming client stays unaffected while ours can render the truth — most
/// importantly that a call is **parked on an approval** rather than failed.
fn meta(event: &TurnStreamEvent) -> Value {
    let mut meta = json!({ "opencompany/seq": event.seq });
    if let Some(detail) = &event.detail {
        meta["opencompany/detail"] = json!(detail);
    }
    if event.status == Some("awaiting_approval") {
        // The distinction ACP's four statuses cannot carry, and the one an
        // operator can act on. Reporting it as a plain failure would render the
        // single actionable state in the timeline as a crash.
        meta["opencompany/awaitingApproval"] = json!(true);
    }
    if event.truncated {
        meta["opencompany/truncated"] = json!(true);
    }
    meta
}

/// Maps a durable journal event, or `None` when it is not part of a session.
pub fn from_company_event(event: &CompanyEvent, chat: &str) -> Option<SessionUpdate> {
    match event {
        // The only place a reply's *text* appears — see the module docs.
        CompanyEvent::AgentReply { chat_id, text, .. } if chat_id == chat => Some(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text },
        })),
        // Echoed so a second client on the same desk sees what the first said.
        // Genuinely useful rather than incidental: two consoles on one thread is
        // the normal shape once a desktop and a browser are both connected.
        // `chat` is optional on an operator message: a send with no desk names
        // the default one, which is the same thread a session opened without a
        // desk is bound to.
        CompanyEvent::OperatorMessage { chat: c, text, .. }
            if c.as_deref()
                .unwrap_or(crate::server::ops::language::DEFAULT_DESK)
                == chat =>
        {
            Some(json!({
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": text },
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn call(id: &str, label: &str) -> TurnStreamEvent {
        TurnStreamEvent {
            kind: "tool_call",
            seq: 1,
            tool_call_id: Some(id.to_string()),
            label: Some(label.to_string()),
            ..TurnStreamEvent::default()
        }
    }

    fn result(id: &str, status: &'static str) -> TurnStreamEvent {
        TurnStreamEvent {
            kind: "tool_result",
            seq: 2,
            tool_call_id: Some(id.to_string()),
            status: Some(status),
            ..TurnStreamEvent::default()
        }
    }

    #[test]
    fn a_started_call_becomes_a_pending_tool_call() {
        let update = from_turn_stream(&call("t1", "Read the roster")).unwrap();
        assert_eq!(update["sessionUpdate"], "tool_call");
        assert_eq!(update["toolCallId"], "t1");
        assert_eq!(update["title"], "Read the roster");
        assert_eq!(update["status"], "pending");
    }

    #[test]
    fn arguments_never_reach_the_raw_input_field() {
        // `detail` is a redacted one-liner. In `rawInput` it would claim to be
        // the real arguments, which is how a scrubbed value gets rendered as
        // truth — and the scrubbing exists precisely because the real ones must
        // not leave the host.
        let mut event = call("t1", "Send mail");
        event.detail = Some("to: <redacted>".to_string());
        let update = from_turn_stream(&event).unwrap();

        assert!(update.get("rawInput").is_none());
        assert_eq!(update["_meta"]["opencompany/detail"], "to: <redacted>");
    }

    #[test]
    fn a_finished_call_amends_by_id_with_its_summary() {
        let mut event = result("t1", "ok");
        event.result = Some("12 items".to_string());
        let update = from_turn_stream(&event).unwrap();

        assert_eq!(update["sessionUpdate"], "tool_call_update");
        assert_eq!(update["toolCallId"], "t1");
        assert_eq!(update["status"], "completed");
        assert_eq!(update["content"][0]["content"]["text"], "12 items");
    }

    #[test]
    fn a_failed_call_is_reported_as_failed() {
        let update = from_turn_stream(&result("t1", "error")).unwrap();
        assert_eq!(update["status"], "failed");
        assert!(
            update["_meta"]
                .get("opencompany/awaitingApproval")
                .is_none()
        );
    }

    #[test]
    fn a_parked_call_is_distinguishable_from_a_failed_one() {
        // ACP has four statuses and none of them mean "waiting on a person".
        // Collapsing a park into a plain failure would render the one state an
        // operator can act on as a crash — the exact mistake #411 fixed in the
        // console's own timeline.
        let update = from_turn_stream(&result("t1", "awaiting_approval")).unwrap();
        assert_eq!(update["status"], "failed", "ACP has nothing better");
        assert_eq!(update["_meta"]["opencompany/awaitingApproval"], true);
    }

    #[test]
    fn a_truncated_result_says_so() {
        let mut event = result("t1", "ok");
        event.truncated = true;
        let update = from_turn_stream(&event).unwrap();
        assert_eq!(update["_meta"]["opencompany/truncated"], true);
    }

    #[test]
    fn a_thinking_marker_produces_no_empty_bubble() {
        // This host streams no reasoning text, so an `agent_thought_chunk`
        // would carry nothing and render as a blank bubble every turn.
        let event = TurnStreamEvent {
            kind: "thinking",
            ..TurnStreamEvent::default()
        };
        assert!(from_turn_stream(&event).is_none());
    }

    #[test]
    fn the_notification_envelope_is_well_formed() {
        let wrapped = notification("sess-1", json!({ "sessionUpdate": "tool_call" }));
        assert_eq!(wrapped["jsonrpc"], "2.0");
        assert_eq!(wrapped["method"], "session/update");
        // A notification carries no id, or a conforming client tries to reply.
        assert!(wrapped.get("id").is_none());
        assert_eq!(wrapped["params"]["sessionId"], "sess-1");
    }
}
