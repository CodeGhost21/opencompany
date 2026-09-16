/// The wire shape the console binds to.
///
/// `fold_asides` is worthless if the field reaches the browser under a
/// different name, and `tsc` cannot catch that: the DTO is Rust, the
/// interface is hand-written TypeScript, and nothing checks one against the
/// other. This is that check.

use super::operator_test_support_1::*;
use super::operator_test_support_2::*;
use super::operator_test_support_3::*;
use super::operator_test_support_4::*;

#[test]
fn projects_agent_reply_with_chat_fields_and_steps() {
    use crate::ports::types::{TurnStep, TurnStepKind, TurnStepStatus};
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "shipped it".into(),
        steps: vec![TurnStep {
            kind: TurnStepKind::ToolCall,
            status: TurnStepStatus::Ok,
            label: "Reading messages".into(),
            detail: None,
            elapsed_ms: Some(12),
            ..TurnStep::default()
        }],
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(v["type"], "agent_reply");
    assert_eq!(v["seq"], 7);
    assert_eq!(v["atMillis"], 1_700_000_000_000_u64);
    assert_eq!(v["chatId"], "General");
    assert_eq!(v["agentId"], "ceo");
    assert_eq!(v["text"], "shipped it");
    // The scrubbed timeline rides along so a live listener sees the steps.
    assert_eq!(v["steps"][0]["label"], "Reading messages");
    assert_eq!(v["steps"][0]["status"], "ok");
    // A channel reply names no thread, so the legacy frame is unchanged.
    assert!(v.get("parentId").is_none(), "unexpected parentId: {v}");
}

/// **The live frame carries the model's own body, not only the operator's.**
///
/// `MessageView` has shipped both since it gained `cue_text`; the live frame
/// had only `text`, so anything needing the room's grammar had to scrape it
/// back out of the operator-facing body. `frontend/src/lib/hive/episode.ts`
/// does exactly that (`moveOf(m.text)`), which is why rewriting `text` here
/// costs the deliberation panel rather than merely tidying a bubble.
///
/// Pinned now, while the two are equal, so the step that rewrites `text`
/// cannot quietly take `cueText` with it.
#[test]
fn projects_the_agents_own_body_beside_the_operators() {
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "returns".into(),
        agent_id: "refunds".into(),
        text: "!support #kettle ^16 the swap is the customer's first preference".into(),
        steps: Vec::new(),
    });
    let value = super::project_event_for_viewer(
        &stored,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("agent_reply is an attention signal");

    assert_eq!(
        value["cueText"], "!support #kettle ^16 the swap is the customer's first preference",
        "the room's grammar is what the fold reads; it must survive on this frame: {value}"
    );
    assert_eq!(
        value["text"], "the swap is the customer's first preference",
        "and the operator reads prose, exactly as the reload already gives them: {value}"
    );
}

/// And on a desk that does not deliberate the two are byte-equal, so no
/// consumer has to choose between them for an ordinary reply.
#[test]
fn a_reply_with_no_move_carries_the_same_body_twice() {
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "general".into(),
        agent_id: "ceo".into(),
        text: "here is the summary you asked for".into(),
        steps: Vec::new(),
    });
    let value = super::project_event_for_viewer(
        &stored,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("agent_reply is an attention signal");

    assert_eq!(value["cueText"], value["text"]);
}

#[test]
fn projects_agent_reply_with_viewer_mention_metadata() {
    use crate::ports::types::{Mention, MentionTarget};
    let stored = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: vec![
            Mention {
                target: MentionTarget::User { id: "u-1".into() },
                text: "@Ada".into(),
                offset: 0,
                quiet: false,
            },
            Mention {
                target: MentionTarget::Everyone,
                text: "@everyone".into(),
                offset: 5,
                quiet: true,
            },
        ],
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "@Ada @everyone".into(),
        steps: Vec::new(),
    });
    let authors = std::collections::HashMap::from([(String::from("u-1"), String::from("Ada"))]);
    let value =
        super::project_event_for_viewer(&stored, &authors, &Viewer::User("u-1".into()), false)
            .expect("agent_reply is an attention signal");
    assert_eq!(
        value["mentions"],
        serde_json::json!([
            { "text": "@Ada", "offset": 0, "label": "Ada", "mine": true },
            { "text": "@everyone", "offset": 5, "label": "everyone", "mine": true, "quiet": true },
        ])
    );
}

/// Issue #1781 review, Codex P1: `history_for_desk` already hides an
/// owner-fallback report from a non-admin on reload; this proves the live
/// SSE projection agrees, rather than handing a non-admin console the full
/// admin-only text the instant it lands.
#[test]
fn drops_owner_fallback_report_from_a_non_admin_viewer() {
    let event = stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "operator".into(),
        agent_id: crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR.to_string(),
        text: "no admin has a mailbox".into(),
        steps: Vec::new(),
    });

    let non_admin = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::User("member-1".into()),
        false,
    );
    assert!(
        non_admin.is_none(),
        "a non-admin viewer must not receive the admin-only report live: {non_admin:?}"
    );

    let admin = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::User("admin-1".into()),
        true,
    )
    .expect("an admin viewer still receives the report live");
    assert_eq!(
        admin["agentId"],
        crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR
    );

    // The Operator viewer (issue #66's original, unrestricted principal)
    // must see it too — same as `project_event`'s `is_admin: true` default.
    let operator = super::project_event_for_viewer(
        &event,
        &std::collections::HashMap::new(),
        &Viewer::Operator,
        true,
    )
    .expect("the operator viewer still receives the report live");
    assert_eq!(operator["text"], "no admin has a mailbox");
}

#[test]
fn projects_agent_reply_with_its_thread_parent() {
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: Some(EventSeq::new(4)),
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "in the thread".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(v["parentId"], "4");
}

/// Issue #983: the accept frame carries the turn, the desk and the thread —
/// and **nothing else**.
///
/// The negative half is what this test is for. `TurnStarted` is the first
/// frame on this stream that brackets an operator's own message, so it is
/// the obvious place for somebody to "helpfully" add the text or the asker
/// — which is exactly the payload the deny-by-default projection exists to
/// keep off the wire, and which `OperatorMessage` is dropped to avoid.
#[test]
fn projects_turn_started_with_structural_keys_only() {
    use crate::ports::types::{Actor, ActorKind};
    let v = super::project_event(&stored(CompanyEvent::TurnStarted {
        turn_id: "turn-1".into(),
        chat_id: "General".into(),
        parent: Some(EventSeq::new(4)),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u-1".into(),
        }),
    }))
    .expect("an accepted turn is an attention signal");
    assert_eq!(v["type"], "turn_started");
    assert_eq!(v["turnId"], "turn-1");
    assert_eq!(v["chatId"], "General");
    assert_eq!(v["parentId"], "4");
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["type", "seq", "atMillis", "turnId", "chatId", "parentId"],
        "the accept frame grew a key: {v}"
    );

    // A turn answering the channel itself omits the thread rather than
    // sending null, so the console's check is a presence check.
    let v = super::project_event(&stored(CompanyEvent::TurnStarted {
        turn_id: "turn-2".into(),
        chat_id: "General".into(),
        parent: None,
        by: None,
    }))
    .expect("an accepted turn is an attention signal");
    assert!(v.get("parentId").is_none(), "unexpected parentId: {v}");
}

/// The settle frame says a turn is over and **not why**.
///
/// `TurnFailed::error` is a reason in our own words that can name
/// internals; the console learns the reason from the tenant-scoped run row.
#[test]
fn projects_turn_settled_without_the_failure_reason() {
    let v = super::project_event(&stored(CompanyEvent::TurnFailed {
        turn_id: "turn-1".into(),
        error: "connection to db-primary.internal refused".into(),
    }))
    .expect("a settled turn is an attention signal");
    assert_eq!(v["type"], "turn_settled");
    assert_eq!(v["turnId"], "turn-1");
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["type", "seq", "atMillis", "turnId"],
        "the settle frame grew a key: {v}"
    );
}

/// The operator's own message is **still** dropped (issue #983).
///
/// Pinned because #983 added the two arms above right beside it, and the
/// natural next step — "the console needs the message too, project it" —
/// would put operator-authored free text onto this stream for the first
/// time. It does not need it: the message is already in the POST's own
/// response and in `chat/history`, which is the point of journaling it at
/// accept time. If somebody later decides otherwise, they say so here.
#[test]
fn projects_nothing_for_the_operators_own_message() {
    assert!(
        super::project_event(&stored(CompanyEvent::OperatorMessage {
            mentions: Vec::new(),
            text: "the operator's own words".into(),
            by: None,
            chat: Some("General".into()),
            parent: None,
            deliverable: None,
            attachments: Vec::new(),
        }))
        .is_none(),
        "the operator's own message must not reach the console over SSE"
    );
}

/// A reaction is deliberately NOT on the attention stream (issue #364).
///
/// Pinned rather than left to the deny-by-default fall-through, because the
/// omission is a decision and not an oversight: the frame would have to
/// carry the reacting person, and this stream has no per-viewer projection
/// to turn an actor into a label. Reload-visibility is what the issue asks
/// for. If someone later decides reactions should stream, this test is
/// where they say so out loud.
#[test]
fn projects_nothing_for_a_reaction() {
    assert!(
        super::project_event(&stored(CompanyEvent::ReactionToggled {
            message_seq: EventSeq::new(4),
            emoji: "👍".into(),
            on: true,
            by: None,
        }))
        .is_none(),
        "a reaction must not reach the console over SSE"
    );
}

/// Issue #379: the park frame carries an id, a kind and the channel — and
/// **nothing else**.
///
/// The negative half is the load-bearing one. The effect's arguments are
/// redacted in exactly one place (`pending_approvals`), and if this frame
/// ever grew a `payload` key it would become a second surface that has to
/// redact and one day will not. Asserting the absence is what makes that a
/// build failure rather than a leak.
#[test]
fn projects_approval_parked_with_a_channel_and_no_payload() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalParked {
        approval_id: ApprovalId::new("appr-1"),
        effect_kind: "payment.send".into(),
        thread: Some("desk-finance".into()),
    }))
    .expect("a parked approval is an attention signal");
    assert_eq!(v["type"], "approval_parked");
    assert_eq!(v["approvalId"], "appr-1");
    assert_eq!(v["kind"], "payment.send");
    assert_eq!(v["chatId"], "desk-finance");
    for forbidden in ["payload", "agent", "amountUsd", "effect", "args"] {
        assert!(
            v.get(forbidden).is_none(),
            "the park frame must stay thin — `{forbidden}` leaked: {v}",
        );
    }
    assert_eq!(
        v.as_object().unwrap().len(),
        6,
        "type, seq, atMillis, approvalId, kind, chatId — and nothing more: {v}",
    );
}

/// A park with no conversation behind it omits the channel entirely, so a
/// console filtering by thread matches it nowhere and it stays on the
/// Approvals page (#379).
#[test]
fn projects_approval_parked_without_a_channel_when_no_thread_produced_it() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalParked {
        approval_id: ApprovalId::new("appr-cron"),
        effect_kind: "email.send".into(),
        thread: None,
    }))
    .expect("a parked approval is an attention signal");
    assert_eq!(v["type"], "approval_parked");
    assert!(
        v.get("chatId").is_none(),
        "a page-only approval must carry no channel: {v}",
    );
}

#[test]
fn projects_agent_reply_omits_empty_steps() {
    let v = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "hi".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    // A tool-less reply keeps the legacy wire shape — no `steps` key.
    assert!(v.get("steps").is_none());
    // …and an uncorrelated reply carries no `taskId` either, so the
    // pre-#185 wire shape is byte-for-byte what it was.
    assert!(v.get("taskId").is_none());
}

/// #185: the correlation key rides the SSE stream when — and only when — the
/// event carries one. Both directions matter: its presence is what lets a
/// live console route a frame to the right task, and its absence is what
/// keeps the legacy shape intact for every ordinary chat reply.
#[test]
fn projects_task_id_only_when_the_event_is_correlated() {
    let reply = super::project_event(&stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: Some("t-1".into()),
        outputs: Vec::new(),
        chat_id: "t-1".into(),
        agent_id: "ceo".into(),
        text: "on it".into(),
        steps: Vec::new(),
    }))
    .expect("agent_reply is an attention signal");
    assert_eq!(reply["taskId"], serde_json::json!("t-1"));

    let failure = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: Some("t-1".into()),
        server: "gh".into(),
        tool: "issues".into(),
        status: "credential_required".into(),
        message: "needs auth".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert_eq!(failure["taskId"], serde_json::json!("t-1"));

    let uncorrelated = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: None,
        server: "gh".into(),
        tool: "issues".into(),
        status: "credential_required".into(),
        message: "needs auth".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert!(uncorrelated.get("taskId").is_none());
}

/// #185/#377: the dispatch terminal projects the structural fields, plus
/// the conversation the card was raised from. `column` is the one that
/// matters most — it is how a console tells a clean finish from a cancelled
/// or failed run — and `chatId` is what says which channel it belongs in.
#[test]
fn projects_desk_task_completed_with_every_field() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["type"], serde_json::json!("desk_task_completed"));
    assert_eq!(v["taskId"], serde_json::json!("t-1"));
    assert_eq!(v["desk"], serde_json::json!("engineer"));
    assert_eq!(v["column"], serde_json::json!("in_review"));
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    // The envelope's own keys still ride along — the console mints the
    // marker's identity from `seq` (issue #483's mechanism), so losing it
    // here would silently disable the reload dedupe.
    assert!(v.get("seq").is_some(), "{v}");
    assert!(v.get("atMillis").is_some(), "{v}");
}

/// Issue #377: the run's prose is **not** on this frame.
///
/// The relay bubble (#151) already carries the agent's words into the same
/// channel this marker lands in. Projecting `output` here as well would put
/// one run's text into one conversation twice, and dropping it at the
/// projection is what stops any later reader from reintroducing that.
#[test]
fn desk_task_completed_does_not_project_the_runs_prose() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "the whole reply, verbatim".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert!(v.get("output").is_none(), "{v}");
    assert!(
        !v.to_string().contains("the whole reply"),
        "the prose must not reach the wire under any key: {v}"
    );
}

/// Issue #377: a card nobody raised from a conversation omits `chatId`
/// rather than sending null — so "board-created" is a presence check on the
/// console, the same shape `approval_parked` uses for a page-only approval.
#[test]
fn desk_task_completed_omits_the_chat_id_for_a_board_created_card() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: None,
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert!(v.get("chatId").is_none(), "{v}");
    assert_eq!(v["column"], serde_json::json!("in_review"));
}

/// Issue #1890 B: the thread inside the channel, on exactly the terms
/// `chatId` rides on.
///
/// Stringified, because the console keys threads by message id and a
/// message id is a string there — `chat/history` renders the same root the
/// same way, and the two must agree or the marker would render inline live
/// and jump into a thread on reload.
#[test]
fn desk_task_completed_projects_the_thread_its_card_was_raised_in() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: Some(crate::ports::types::EventSeq::new(41)),
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    assert_eq!(v["parentId"], serde_json::json!("41"));
}

/// A card raised straight into a channel omits `parentId` rather than
/// sending null — the same presence-check shape `chatId` takes, so the
/// console reads "channel level" without a null check.
#[test]
fn desk_task_completed_omits_the_parent_for_a_channel_level_card() {
    let v = super::project_event(&stored(CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".into(),
        desk: "engineer".into(),
        output: "shipped".into(),
        column: "in_review".into(),
        artifact_ids: Vec::new(),
        origin_chat_id: Some("engineering".into()),
        origin_parent: None,
    }))
    .expect("desk_task_completed is an attention signal");
    assert_eq!(v["chatId"], serde_json::json!("engineering"));
    assert!(v.get("parentId").is_none(), "{v}");
}

#[test]
fn projects_task_dispatched() {
    let v = super::project_event(&stored(CompanyEvent::TaskDispatched {
        task_id: "t-42".into(),
        run_id: None,
    }))
    .expect("task_dispatched is an attention signal");
    assert_eq!(v["type"], "task_dispatched");
    assert_eq!(v["taskId"], "t-42");
}

/// Issue #464: an opened card reaches the console as its own frame. This is
/// the half a unit test can prove — that the projection exists and carries
/// the card; that the *board* redraws off it is a browser fact.
#[test]
fn projects_task_card_changed() {
    let v = super::project_event(&stored(CompanyEvent::TaskCardChanged {
        task_id: "t-77".into(),
        change: crate::runtime::CHANGE_OPENED.into(),
        column: Some("todo".into()),
    }))
    .expect("a board write is an attention signal");
    assert_eq!(v["type"], "task_card_changed");
    assert_eq!(v["taskId"], "t-77");
    assert_eq!(v["change"], "opened");
    assert_eq!(v["column"], "todo");
}

/// A removed card is projected without a column — the console's "is it
/// gone?" check is a presence check, never a null one.
#[test]
fn projects_a_removed_card_without_a_column() {
    let v = super::project_event(&stored(CompanyEvent::TaskCardChanged {
        task_id: "t-77".into(),
        change: crate::runtime::CHANGE_REMOVED.into(),
        column: None,
    }))
    .expect("a board write is an attention signal");
    assert_eq!(v["change"], "removed");
    assert!(
        v.get("column").is_none(),
        "a removed card is in no column: {v}"
    );
}

/// Issue #327: the workspace's own frame. The stream is deny-by-default, so
/// an event with no arm is silently unprojected — this is what proves the
/// arm exists at all.
///
/// Also pins what is **not** on the wire: no node name, no body. A note's
/// text is operator- or agent-authored free text, and this frame's job is
/// to say something moved, not to carry the tree.
#[test]
fn projects_workspace_changed_without_a_name_or_a_body() {
    let v = super::project_event(&stored(CompanyEvent::WorkspaceChanged {
        node_id: "n-9".into(),
        change: crate::runtime::CHANGE_UPDATED.into(),
    }))
    .expect("a workspace write must reach the console");
    assert_eq!(v["type"], "workspace_changed");
    assert_eq!(v["nodeId"], "n-9");
    assert_eq!(v["change"], "updated");
    assert!(v.get("name").is_none(), "no node name on the wire: {v}");
    assert!(v.get("content").is_none(), "no body on the wire: {v}");
}

#[test]
fn projects_mcp_call_failed_with_scrubbed_message() {
    let v = super::project_event(&stored(CompanyEvent::McpCallFailed {
        task_id: None,
        server: "browserbase".into(),
        tool: "browse".into(),
        status: "tool_call_rejected".into(),
        message: "server rejected the call".into(),
    }))
    .expect("mcp_call_failed is an attention signal");
    assert_eq!(v["type"], "mcp_call_failed");
    assert_eq!(v["server"], "browserbase");
    assert_eq!(v["tool"], "browse");
    assert_eq!(v["status"], "tool_call_rejected");
    // The message is already scrubbed at the source; we forward exactly it.
    assert_eq!(v["message"], "server rejected the call");
}

#[test]
fn projects_approval_resolved_without_the_actor() {
    let v = super::project_event(&stored(CompanyEvent::ApprovalResolved {
        approval_id: ApprovalId::new("ap-1"),
        verdict: Verdict::Approve,
        by: Actor {
            kind: ActorKind::User,
            // A user id must never reach the wire via the attention feed.
            id: "secret-user-id".into(),
        },
    }))
    .expect("approval_resolved is an attention signal");
    assert_eq!(v["type"], "approval_resolved");
    assert_eq!(v["approvalId"], "ap-1");
    assert_eq!(v["verdict"], "approve");
    // The actor is intentionally dropped — the projection carries no `by`,
    // and the serialized bytes never mention the user id.
    assert!(v.get("by").is_none(), "actor must not be projected");
    assert!(
        !v.to_string().contains("secret-user-id"),
        "user id leaked onto the wire"
    );
    // Issue #971: and a person's decision carries no `automatic` flag, so
    // the console's "an operator decided this" reading of its absence is
    // the correct one.
    assert!(
        v.get("automatic").is_none(),
        "a user's own decision is not automatic"
    );
}
