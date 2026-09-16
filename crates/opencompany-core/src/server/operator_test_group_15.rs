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

/// Issue #1781 review, Codex P1 follow-up: even with the periodic refresh
/// the test above covers, `company_events` still only re-checked on its
/// own `LABEL_REFRESH_EVERY` (60s) tick — a demotion landing right after
/// one tick left an open SSE stream projecting an owner-fallback report
/// under a stale cached `true` for up to another 60s. `is_admin_for_item`
/// is the fix: it revalidates fresh for that one content class instead of
/// trusting `cached`, no matter how long ago the last periodic tick was —
/// proven here by feeding it a `cached: true` that is already wrong the
/// instant this call happens, with no `sleep` at all.
///
/// The second half is the other side of the same fix: an *ordinary* event
/// must keep using `cached` untouched, or every SSE item would pay a
/// store read regardless of content — the whole reason the fix is scoped
/// to the owner-fallback content class rather than revalidating every
/// item.
#[tokio::test]
async fn is_admin_for_item_revalidates_only_the_owner_fallback_report() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    let mut user = crate::ports::users::UserRecord {
        id: "u1".to_string(),
        email: "admin@acme.test".to_string(),
        display_name: None,
        avatar: None,
        role: crate::ports::users::UserRole::Admin,
        status: crate::ports::users::UserStatus::Active,
        password_hash: None,
        must_change_password: false,
        created_at_millis: crate::ports::now_millis(),
        last_seen_at_millis: None,
        updated_at_millis: crate::ports::now_millis(),
    };
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();
    let actor = Actor {
        kind: ActorKind::User,
        id: user.id.clone(),
    };

    // The demotion: no wait, no periodic tick — the very next item must
    // already see it for the gated content class.
    user.role = crate::ports::users::UserRole::Member;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    let owner_fallback_item = EventStreamItem::Event(stored(CompanyEvent::AgentReply {
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
    }));
    assert!(
        !super::is_admin_for_item(&owner_fallback_item, &runtime, Some(&actor), true).await,
        "an owner-fallback report must revalidate fresh and see the demotion \
         immediately — a stale cached `true` must never leak this content, \
         regardless of when the last periodic refresh ran"
    );

    let ordinary_item = EventStreamItem::Event(stored(CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: "General".into(),
        agent_id: "ceo".into(),
        text: "ordinary reply".into(),
        steps: Vec::new(),
    }));
    assert!(
        super::is_admin_for_item(&ordinary_item, &runtime, Some(&actor), true).await,
        "an ordinary event must keep using the cached snapshot untouched — \
         revalidating every item, not just the gated content class, would \
         add a store read to the hot path for no reason"
    );
}

/// The machine principal has no user record to look up — `actor: None` —
/// and [`ScopedCompany::is_admin`]'s own doc says it is unrestricted by
/// construction, so the refresh must leave it alone rather than treating
/// a missing actor as "look up nothing, therefore not admin".
#[tokio::test]
async fn refreshed_is_admin_leaves_the_machine_principal_unchanged() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    assert!(refreshed_is_admin(&runtime, None, true).await);
    assert!(!refreshed_is_admin(&runtime, None, false).await);
}

/// Two cards can be `in_review` on the same desk at once. Approving the
/// pill the operator actually clicked must move that card and leave the
/// other alone — resolving the desk's most-recently-updated card instead
/// (Codex #3903031183) moves the wrong one whenever the older pill is
/// clicked after a newer card has settled.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_settles_the_clicked_task_not_the_desks_latest() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    for (task_id, updated_at_millis) in [("t-old", 1u64), ("t-new", 2u64)] {
        runtime
            .tasks()
            .upsert(
                runtime.id(),
                &crate::ports::tasks::TaskRecord {
                    id: task_id.to_string(),
                    title: TaskTitle::authored("Ship it"),
                    note: None,
                    column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                    priority: "medium".to_string(),
                    assignee: "ceo".to_string(),
                    updated_at_millis,
                    origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                    parent_task_id: None,
                    output: None,
                    plan: None,
                    planning_attempts: Vec::new(),
                    deliverable: crate::ports::tasks::TaskDeliverable::Once,
                    workflow_proposal: None,
                    origin_run_id: None,
                    origin_workflow_id: None,
                    origin_message_seq: None,
                    bounced: None,
                },
            )
            .await
            .unwrap();
    }

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let receipt = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-old".to_string(),
            decision: "approve".to_string(),
            note: None,
        }),
    )
    .await
    .expect("the clicked card is settled")
    .0;
    assert_eq!(receipt.task_id, "t-old");
    assert_eq!(receipt.column, crate::ports::tasks::COLUMN_DONE);

    let cards = runtime.tasks().list(runtime.id()).await.unwrap();
    let old = cards.iter().find(|t| t.id == "t-old").unwrap();
    let new = cards.iter().find(|t| t.id == "t-new").unwrap();
    assert_eq!(
        old.column,
        crate::ports::tasks::COLUMN_DONE,
        "the clicked pill's card must settle"
    );
    assert_eq!(
        new.column,
        crate::ports::tasks::COLUMN_IN_REVIEW,
        "the desk's newer card must be untouched by a verdict on the older pill"
    );
}

/// A `task_id` naming a card outside the reviewed desk (or one that has
/// already left `in_review`) must not resolve to some other card in the
/// conversation — the request is rejected rather than silently falling
/// back to "whatever is in review here".
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_rejects_a_task_id_not_in_review_on_this_desk() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-review".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .unwrap();

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let err = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "does-not-exist".to_string(),
            decision: "approve".to_string(),
            note: None,
        }),
    )
    .await
    .expect_err("an unknown task id must not fall back to the desk's own card");
    assert_eq!(
        axum::response::IntoResponse::into_response(err).status(),
        StatusCode::NOT_FOUND
    );
}

/// `apply_review_decision`'s `Revise` arm through the HTTP handler: the
/// card re-enters `in_progress` with the operator's note appended, rather
/// than settling to `done` the way `Approve` does.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_revise_re_enters_in_progress_with_the_note() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: Some("[writer] first draft".to_string()),
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .unwrap();

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let receipt = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-1".to_string(),
            decision: "revise".to_string(),
            note: Some("tighten the intro".to_string()),
        }),
    )
    .await
    .expect("revise applies")
    .0;
    assert_eq!(receipt.task_id, "t-1");
    assert_eq!(receipt.column, crate::ports::tasks::COLUMN_IN_PROGRESS);

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    let note = after.note.expect("note");
    assert!(note.contains("tighten the intro"), "{note}");
}

/// A thread reply intercepted as review feedback re-dispatches its card
/// instead of answering with `responses` here. Codex #3903907771:
/// `ChatView.send` reads an empty `responses` as "the turn produced
/// nothing" and renders a synthetic "(no reply)" bubble underneath the
/// operator's own feedback, even though the card was re-dispatched and
/// will answer through its later relay. `reviewFeedbackApplied` is what
/// tells the console this empty `responses` is expected.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn thread_reply_review_feedback_marks_the_response_not_empty_handed() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .unwrap();

    runtime
        .events()
        .append(
            runtime.id(),
            crate::ports::types::CompanyEvent::DeskTaskCompleted {
                task_id: "t-1".to_string(),
                desk: "ceo".to_string(),
                output: "done".to_string(),
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                artifact_ids: Vec::new(),
                origin_chat_id: Some("strategy".to_string()),
                origin_parent: None,
            },
        )
        .await
        .unwrap();
    let relay_seq = runtime
        .events()
        .append(
            runtime.id(),
            crate::ports::types::CompanyEvent::AgentReply {
                audience: Vec::new(),
                chat_id: "strategy".to_string(),
                agent_id: "ceo".to_string(),
                text: "Here is the draft.".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
            },
        )
        .await
        .unwrap();

    let message = ChatMessage {
        text: "needs another pass".to_string(),
        chat: Some("strategy".to_string()),
        parent: Some(relay_seq.value().to_string()),
        deliverable: None,
        detach: false,
        mentions: None,
        attachments: Vec::new(),
    };

    let outcome = chat_and_emit(&state, &id, runtime.clone(), message, None)
        .await
        .expect("review feedback applies");
    let ChatOk::Settled(body) = outcome else {
        panic!("a synchronous review-feedback intercept must not detach");
    };
    assert!(body.responses.is_empty());
    assert_eq!(
        body.review_feedback_applied,
        Some(true),
        "an empty `responses` here must be marked expected, not read as \
         a silent turn"
    );

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    assert_eq!(
        after.column,
        crate::ports::tasks::COLUMN_IN_PROGRESS,
        "the reply still re-dispatches the card"
    );
}

/// An unrecognized `decision` string rejects with `InvalidRequest` (400)
/// rather than falling through to either verdict.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_rejects_an_unknown_decision() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .unwrap();

    let scope = ScopedCompany {
        runtime: runtime.clone(),
        actor: None,
        may_read_contents: true,
        is_admin: true,
    };
    let err = review_card(
        scope,
        Json(ChatReviewRequest {
            chat_id: "strategy".to_string(),
            task_id: "t-1".to_string(),
            decision: "yeet".to_string(),
            note: None,
        }),
    )
    .await
    .expect_err("an unknown decision string must not settle the card");
    assert_eq!(
        axum::response::IntoResponse::into_response(err).status(),
        StatusCode::BAD_REQUEST
    );
}

/// `POST {scope}/chat/review` end to end through the real router: proves
/// the route is actually mounted by [`with_review_routes`] (not just that
/// the handler function works when called directly) and that the wire
/// body deserializes and settles the card via HTTP.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn chat_review_route_is_mounted_and_settles_via_http() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(
            runtime.id(),
            &crate::ports::tasks::TaskRecord {
                id: "t-1".to_string(),
                title: TaskTitle::authored("Ship it"),
                note: None,
                column: crate::ports::tasks::COLUMN_IN_REVIEW.to_string(),
                priority: "medium".to_string(),
                assignee: "ceo".to_string(),
                updated_at_millis: 1,
                origin: crate::ports::TaskOrigin::new(Some("strategy".to_string()), None),
                parent_task_id: None,
                output: None,
                plan: None,
                planning_attempts: Vec::new(),
                deliverable: crate::ports::tasks::TaskDeliverable::Once,
                workflow_proposal: None,
                origin_run_id: None,
                origin_workflow_id: None,
                origin_message_seq: None,
                bounced: None,
            },
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat/review")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "chatId": "strategy",
                        "taskId": "t-1",
                        "decision": "approve",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["taskId"], "t-1");
    assert_eq!(value["column"], "done");
}

/// No card is `in_review` on the desk at all — as opposed to a `taskId`
/// naming the wrong card, covered above — must also 404, through the same
/// HTTP path the console calls.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn chat_review_route_404s_when_no_card_is_in_review() {
    let home_dir = home();
    let home = home_dir.path().to_path_buf();
    let state = state_with_company(&home, "running").await;

    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat/review")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "chatId": "strategy",
                        "taskId": "t-1",
                        "decision": "approve",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Two review verdicts racing the same `in_review` card (PR #1981 review
/// finding, Codex P1) must not both resolve it before either applies —
/// same `task_writes`-serialized load-modify-save shape
/// `add_desk_member_serializes_against_the_company_write_lock` proves
/// above, applied to `review_card`.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_serializes_against_the_task_writes_lock() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    runtime
        .tasks()
        .upsert(runtime.id(), &card_in_review("t-1", "strategy"))
        .await
        .unwrap();

    let guard = runtime.task_writes.lock().await;

    let runtime_for_task = runtime.clone();
    let mut task = tokio::spawn(async move {
        let scope = ScopedCompany {
            runtime: runtime_for_task,
            actor: None,
            may_read_contents: true,
            is_admin: true,
        };
        review_card(
            scope,
            Json(ChatReviewRequest {
                chat_id: "strategy".to_string(),
                task_id: "t-1".to_string(),
                decision: "approve".to_string(),
                note: None,
            }),
        )
        .await
    });

    let raced_ahead = tokio::time::timeout(Duration::from_millis(200), &mut task)
        .await
        .is_ok();
    assert!(
        !raced_ahead,
        "review_card resolved and applied a verdict while task_writes was \
         held elsewhere — it is not serializing against concurrent board \
         writers"
    );

    drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("review_card never resumed after task_writes was released")
        .expect("review_card task panicked");
    assert!(result.is_ok());
}
