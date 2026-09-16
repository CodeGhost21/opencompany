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

/// Issue #618: membership gets you the approval, role gets you its
/// contents.
///
/// Issue #561: the receipt says whether this decision actually released the
/// turn.
///
/// A turn that parked two calls is blocked on two decisions (issue #469
/// continues it once, on the last one). The console used to tell the
/// operator "the agent is completing the action" on the first click, which
/// is false — nothing runs until the second. This is the count it now words
/// that sentence from: one still owed after the first decision, none after
/// the second.
#[tokio::test]
async fn a_receipt_says_how_many_decisions_the_turn_is_still_blocked_on() {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    let effect = |memo: &str| crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(10.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "board@example.test", "memo": memo }),
        agent: Some("ceo".into()),
        run_id: None,
    };

    // One turn, two parked calls — the shape an operator meets whenever an
    // agent gates more than once in a turn.
    for (id, memo) in [("appr-561-a", "first"), ("appr-561-b", "second")] {
        runtime
            .journal
            .record_parked(
                &crate::ports::types::ApprovalId::new(id),
                &effect(memo),
                1_000,
                TaskLink::Unlinked,
                ApprovalConversation::default(),
                Some("cycle-561".to_string()),
            )
            .await
            .unwrap();
        runtime.continuations.arm("cycle-561");
    }

    let app = router(state);
    let resolve = |app: axum::Router, id: &'static str| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/company/approvals/{id}"))
                    .header("content-type", "application/json")
                    .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                    .body(Body::from(
                        serde_json::json!({ "verdict": "approve", "detach": true }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let first = resolve(app.clone(), "appr-561-a").await;
    assert_eq!(
        first["stillAwaiting"], 1,
        "the first decision releases nothing — the turn is still blocked on the second: {first}"
    );

    // The count is read per decision, at the moment the verdict lands. What
    // happens to the sibling afterwards is issue #848's business — a turn's
    // gated calls may be consolidated and settle together — and this test
    // deliberately asserts only the half the operator's confirmation is
    // worded from: this click did not release the turn.
    //
    // The other half — the last decision reporting nothing outstanding —
    // is pinned on the queue itself in
    // `runtime::continuation::test::outstanding_counts_the_decision_being_made`,
    // where it is deterministic rather than racing a spawned follow-up.
}

/// **The two-account part is the point.** The harness signs every request
/// in as an admin, so a redaction verified only as an admin passes
/// identically against no redaction at all — the test would prove nothing
/// while looking like coverage. This seeds a second, Member-role account
/// and drives the same route with both.
#[tokio::test]
async fn a_member_sees_the_approval_but_not_its_payload_or_amount() {
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();

    let effect = crate::ports::types::Effect {
        kind: "payment.send".into(),
        group: crate::ports::types::EffectGroup::Spend,
        amount_usd: Some(2400.0),
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::json!({ "to": "board@example.test", "memo": "Q3 retainer" }),
        agent: Some("ceo".into()),
        run_id: None,
    };
    runtime
        .journal
        .record_parked(
            &crate::ports::types::ApprovalId::new("appr-618"),
            &effect,
            1_000,
            TaskLink::Unlinked,
            ApprovalConversation::default(),
            None,
        )
        .await
        .unwrap();

    let app = router(state);

    async fn approvals_as(app: &axum::Router, cookie: String) -> serde_json::Value {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/company/approvals")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // The admin decides the sign-off, so the admin sees what it will do.
    let as_admin = approvals_as(&app, crate::server::test_support::fixed_cookie("acme")).await;
    let admin_row = &as_admin.as_array().unwrap()[0];
    assert_eq!(admin_row["amount_usd"].as_f64(), Some(2400.0));
    assert_eq!(admin_row["payload"]["to"], "board@example.test");
    assert!(
        admin_row.get("contents_hidden").is_none(),
        "an admin is not told anything was hidden: {admin_row}"
    );

    let as_member =
        approvals_as(&app, crate::server::test_support::member_cookie("acme")).await;
    let member_row = &as_member.as_array().unwrap()[0];

    // Still visible: everything that makes stalled work legible. This half
    // is what #468 depends on — a member must keep seeing that work is
    // waiting and what kind of call it is.
    assert_eq!(member_row["id"], "appr-618");
    assert_eq!(member_row["kind"], "payment.send");
    assert_eq!(member_row["agent"], "ceo");
    assert_eq!(member_row["at_millis"].as_u64(), Some(1_000));

    // Withheld: the recipient and the money.
    assert!(
        member_row.get("payload").is_none(),
        "the recipient must not reach a member: {member_row}"
    );
    // `null`, not absent: unlike `payload`, `amount_usd` carries no
    // `skip_serializing_if`, so it stays on the wire as an explicit null.
    // Both read as "no value" to the console (`a.amount_usd != null`
    // covers either), and changing the wire shape as a side effect of a
    // redaction would be a worse trade than asserting the shape that is
    // actually there.
    assert!(
        member_row["amount_usd"].is_null(),
        "nor the amount: {member_row}"
    );
    assert_eq!(
        member_row["contents_hidden"], true,
        "and the console must be able to say so rather than render an empty card: {member_row}"
    );

    // Belt and braces: the recipient string must appear nowhere in the
    // member's response, however the shape changes later.
    let raw = serde_json::to_string(&as_member).unwrap();
    assert!(
        !raw.contains("board@example.test") && !raw.contains("Q3 retainer"),
        "payload content leaked to a member: {raw}"
    );
}

/// **Issue #2028 — the bug.** An Approvals click that says `skip` banks a
/// skip. Before the route arm existed the same request banked a `retry`,
/// because `verdict: approve` was the only thing the host read.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_skip_from_the_approvals_page_banks_a_skip() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1, "one answer, one banked resolution");
    assert_eq!(
        banked[0]["resolution"]["verdict"], "skip",
        "the operator asked to skip the node, not to run it again"
    );
}

/// The amend twin: the words the operator typed reach the banked resolution
/// verbatim, which is what the re-entered step reads.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_amend_from_the_approvals_page_carries_the_answer_verbatim() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "amend",
            "blocker_answer": "use gpt-4o-mini instead",
            "detach": true,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(banked[0]["resolution"]["verdict"], "amend");
    assert_eq!(
        banked[0]["resolution"]["answer"], "use gpt-4o-mini instead",
        "the correction must reach the step, or the re-run repeats the failure"
    );
}

/// A cancel still denies, and is still the only verdict that does.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_cancel_from_the_approvals_page_banks_a_cancel() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "deny", "blocker_verdict": "cancel", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");

    let banked = banked_resolutions(&c.home, &c.company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(banked[0]["resolution"]["verdict"], "cancel");
}

/// Answering one member of a root-cause group answers all of them — the
/// same fan-out a DM answer performs — and the receipt names every id it
/// settled so the console can drop the siblings' cards too.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_group_settles_together_and_the_receipt_names_every_member() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let first = park_node_blocker(&runtime, "grouped-1", Some("connection:slack")).await;
    let second = park_node_blocker(&runtime, "grouped-2", Some("connection:slack")).await;

    let (status, answer) = post_resolve(
        &app,
        &first,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["settledIds"],
        serde_json::json!(["grouped-1", "grouped-2"]),
        "the receipt must name the siblings the answer settled: {answer}"
    );
    assert!(
        runtime.pending_approvals().is_empty(),
        "one answer to a root-cause group retires every member of it"
    );
    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 2, "both members banked the same verdict");
    for line in &banked {
        assert_eq!(line["resolution"]["verdict"], "skip");
    }
    let _ = second;
}

/// An ordinary resolve is unchanged: no `settledIds` key at all, so a
/// console predating the field reads the same body it always did.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_ordinary_resolve_names_no_settled_ids() {
    let home_dir = home();
    let c = blocked_company(home_dir.path()).await;

    let (status, answer) = post_resolve(
        &c.app,
        &c.approval_id,
        serde_json::json!({ "verdict": "approve", "detach": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert!(
        answer.get("settledIds").is_none(),
        "a resolve that fanned to nothing must carry no list: {answer}"
    );
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_disagreeing_verdict_pair_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "deny", "blocker_verdict": "skip" }),
        "cannot accompany verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blank_amend_is_refused_rather_than_downgraded() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "amend",
            "blocker_answer": "   \n\t ",
        }),
        "needs a non-empty blocker_answer",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_amend_with_no_answer_at_all_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "amend" }),
        "needs a non-empty blocker_answer",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_answer_with_no_verdict_is_refused() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_answer": "use gpt-4o-mini" }),
        "blocker_answer needs a blocker_verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_answer_on_a_wordless_verdict_is_refused() {
    for verdict in ["retry", "skip", "cancel"] {
        let event = if verdict == "cancel" {
            "deny"
        } else {
            "approve"
        };
        assert_refused(
            serde_json::json!({
                "verdict": event,
                "blocker_verdict": verdict,
                "blocker_answer": "words this verdict cannot carry",
            }),
            "only accompanies blocker_verdict",
        )
        .await;
    }
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn an_unknown_blocker_verdict_is_refused_by_name() {
    assert_refused(
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "ignore" }),
        "unknown blocker_verdict",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_with_an_amended_payload_is_refused() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "skip",
            "amended_payload": { "text": "edited" },
        }),
        "cannot accompany amended_payload",
    )
    .await;
}

#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_with_a_tool_scope_is_refused() {
    assert_refused(
        serde_json::json!({
            "verdict": "approve",
            "blocker_verdict": "skip",
            "scope": "tool",
            "expires_in_millis": 3_600_000,
        }),
        "cannot accompany scope",
    )
    .await;
}

/// A `blocker_verdict` on an approval that is not a parked blocker is a 400,
/// not a quiet fall-through to the two-value path — which would lose the
/// operator's verdict without telling anyone.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn a_blocker_verdict_on_an_ordinary_approval_is_refused() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);
    let ordinary = park_for_extend(&runtime, "ordinary-1", crate::ports::now_millis()).await;

    let (status, answer) = post_resolve(
        &app,
        &ordinary,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert!(
        answer["error"]
            .as_str()
            .unwrap_or_default()
            .contains("is not a parked blocker"),
        "{answer}"
    );
    assert!(
        runtime.pending_approvals().iter().any(|p| p.id == ordinary),
        "a refused request must leave the approval parked"
    );
    assert!(banked_resolutions(&home, &company).await.is_empty());
}

/// A stepless blocker uses its task link to settle the card it paused.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn skipping_an_agent_question_settles_the_card_its_approval_is_linked_to() {
    use crate::ports::blockers::{BlockerKind, BlockerPayload, BlockerSource};
    use crate::runtime::journal::{ApprovalConversation, TaskLink};

    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let company = CompanyId::new("acme");
    let runtime = state.registry().get(&company).unwrap();
    let app = router(state);

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-9".to_string(),
                title: crate::ports::tasks::TaskTitle::authored("Draft the launch note"),
                note: None,
                column: crate::ports::tasks::COLUMN_PAUSED.to_string(),
                priority: "medium".to_string(),
                assignee: "eng".to_string(),
                updated_at_millis: 1,
                origin: None,
                origin_message_seq: None,
                parent_task_id: None,
                output: Some(crate::ports::tasks::TaskOutput {
                    source: crate::ports::tasks::TaskOutputSource::Run {
                        run_id: "old-run".to_string(),
                        attempt: Some(1),
                    },
                    at_millis: 1,
                    artifacts: Vec::new(),
                    workflows: Vec::new(),
                }),
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                bounced: Some("stale failure".to_string()),
            },
        )
        .await
        .unwrap();

    let payload = BlockerPayload {
        kind: BlockerKind::Information,
        source: BlockerSource::AgentQuestion,
        step: None,
        reason: "which of the two briefs is current?".to_string(),
        needed: "an answer from you".to_string(),
        group_key: None,
    };
    let approval = ApprovalId::new("question-1");
    let effect = crate::ports::types::Effect {
        kind: payload.effect_kind(),
        group: crate::ports::types::EffectGroup::Other,
        amount_usd: None,
        established_thread: false,
        first_time_counterparty: false,
        payload: serde_json::to_value(&payload).unwrap(),
        agent: None,
        run_id: None,
    };
    let at = crate::ports::now_millis();
    runtime
        .approval_gate
        .rehydrate(approval.clone(), effect.clone(), at);
    runtime
        .journal
        .record_parked(
            &approval,
            &effect,
            at,
            TaskLink::from_task_id(Some("t-9")),
            ApprovalConversation {
                thread: Some("dm:eng".to_string()),
                parent: None,
            },
            None,
        )
        .await
        .unwrap();

    let (status, answer) = post_resolve(
        &app,
        &approval,
        serde_json::json!({ "verdict": "approve", "blocker_verdict": "skip" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer["settledIds"],
        serde_json::json!(["question-1"]),
        "the non-detached body names what it settled too: {answer}"
    );

    let banked = banked_resolutions(&home, &company).await;
    assert_eq!(banked.len(), 1);
    assert_eq!(
        banked[0]["resolution"]["verdict"], "skip",
        "the operator's verdict is banked whatever the resume can do with it"
    );
    assert!(
        banked[0]["resolution"].get("step").is_none(),
        "the durable record keeps the stepless park the blocker carried: {}",
        banked[0]
    );
    let card = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-9")
        .expect("the card still exists");
    assert_eq!(
        card.column,
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "the skipped card is ready for human review"
    );
    assert!(card.output.is_none(), "a skip produces no output");
    assert!(card.bounced.is_none(), "a skip clears the old bounce chip");
    assert_eq!(card.origin_chat_id(), Some("dm:eng"));
    assert!(
        card.note
            .as_deref()
            .is_some_and(|note| { note.contains("blocker question waived by the operator") })
    );
    assert!(
        runtime
            .runs()
            .list_runs(
                runtime.id(),
                &crate::ports::runs::RunFilter::for_task("t-9"),
            )
            .await
            .unwrap()
            .is_empty(),
        "a skip must not open another attempt"
    );
}
