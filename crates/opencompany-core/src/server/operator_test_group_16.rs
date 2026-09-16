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

/// The revalidation half of the same finding: a review reply parked on
/// `task_writes` while a second verdict already settled the card must see
/// the now-current column once it resumes, not the stale `in_review`
/// snapshot it would have clone from before it blocked — so it 404s
/// instead of silently re-applying on top of the settled card.
#[cfg(feature = "openhuman")]
#[tokio::test]
async fn review_card_404s_when_the_card_left_review_while_the_reply_was_in_flight() {
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
    let _ = tokio::time::timeout(Duration::from_millis(200), &mut task).await;

    let card = runtime
        .review_card_in_review("t-1", "strategy")
        .await
        .expect("task store lookup")
        .expect("card is still in_review before the lock is released");
    runtime
        .apply_review_decision(
            &card,
            crate::harness::built_in::lifecycle::ReviewDecision::Revise,
            Some("send it back"),
            None,
        )
        .await
        .unwrap();

    drop(guard);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("review_card never resumed after task_writes was released")
        .expect("review_card task panicked");
    assert!(
        result.is_err(),
        "a review reply that had already resolved the card must not \
         silently re-apply its verdict once the card is no longer \
         in_review"
    );

    let after = runtime
        .tasks()
        .list(runtime.id())
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.id == "t-1")
        .unwrap();
    assert_eq!(after.column, crate::ports::tasks::COLUMN_IN_PROGRESS);
    let note = after.note.expect("note");
    assert!(note.contains("send it back"), "{note}");
}

/// The sharpest case in this file. `may_read_approval_contents` already
/// refuses a member the payload and the amount an approval carries, so
/// before this guard a member could approve a payment they were forbidden
/// to look at.
///
/// The approval id is deliberately one that does not exist: authority is
/// settled before the approval is resolved, so the answer must be `403` and
/// not the `404` a permitted caller would get.
#[tokio::test]
async fn a_member_may_not_resolve_an_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        let denied = app
            .clone()
            .oneshot(resolve_as(scope, "appr-nobody-parked", Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::FORBIDDEN,
            "{scope} let a member decide an approval"
        );
    }
}

/// Extending is the deadline's other side: an approval nobody decides
/// default-denies when its window runs out, so being able to push that
/// window out indefinitely is a decision about the effect, made for the
/// company. It is held to the same authority as deciding it outright.
#[tokio::test]
async fn a_member_may_not_extend_an_approval_deadline() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = park_for_extend(&runtime, "appr-member-ext", 1_000).await;
    let cookie = crate::server::test_support::member_cookie("acme");
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        let denied = app
            .clone()
            .oneshot(extend_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::FORBIDDEN,
            "{scope} let a member extend an approval deadline"
        );
    }
}

/// The other half of the guard: refusing a member must not also refuse the
/// admin the routes exist for, under either address form.
#[tokio::test]
async fn an_admin_may_still_resolve_an_approval() {
    for scope in APPROVAL_SCOPES {
        let home_dir = home();
        let state = state_with_company(home_dir.path(), "running").await;
        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let id = park_for_extend(&runtime, "appr-admin-resolve", 1_000).await;
        let cookie = crate::server::test_support::fixed_cookie("acme");
        let app = router(state);

        let allowed = app
            .oneshot(resolve_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            allowed.status(),
            StatusCode::OK,
            "{scope} refused an admin the decision"
        );
    }
}

#[tokio::test]
async fn an_admin_may_still_extend_an_approval_deadline() {
    for scope in APPROVAL_SCOPES {
        let home_dir = home();
        let state = state_with_company(home_dir.path(), "running").await;
        let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
        let id = park_for_extend(&runtime, "appr-admin-ext", 1_000).await;
        let cookie = crate::server::test_support::fixed_cookie("acme");
        let app = router(state);

        let allowed = app
            .oneshot(extend_as(scope, id.as_ref(), Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(
            allowed.status(),
            StatusCode::OK,
            "{scope} refused an admin the extension"
        );
    }
}

/// No credential at all is `401`, not `403` — the authority guard must not
/// turn an anonymous request into a role decision.
#[tokio::test]
async fn an_unauthenticated_caller_cannot_decide_or_extend_an_approval() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        for request in [
            resolve_as(scope, "appr-anon", None),
            extend_as(scope, "appr-anon", None),
        ] {
            let uri = request.uri().to_string();
            let denied = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                denied.status(),
                StatusCode::UNAUTHORIZED,
                "{uri} answered an anonymous caller with {}",
                denied.status()
            );
        }
    }
}

/// The second defect these routes carried: the temporary-password boundary
/// lived only on the single-company alias, so an admin who had never set a
/// password could decide and extend every approval through the `{id}` form.
///
/// An admin is the right principal to prove it with — the role check passes,
/// so a refusal here can only be the password boundary.
#[tokio::test]
async fn an_admin_on_a_temporary_password_may_not_decide_or_extend() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let cookie = crate::server::test_support::seed_temp_password_admin(&state, "acme").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = park_for_extend(&runtime, "appr-temp-pass", 1_000).await;
    let app = router(state);

    for scope in APPROVAL_SCOPES {
        for request in [
            resolve_as(scope, id.as_ref(), Some(&cookie)),
            extend_as(scope, id.as_ref(), Some(&cookie)),
        ] {
            let uri = request.uri().to_string();
            let denied = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                denied.status(),
                StatusCode::FORBIDDEN,
                "{uri} served an admin who has not set a password"
            );
            assert_eq!(
                body_json(denied).await["code"],
                "password_change_required",
                "{uri} refused for the wrong reason"
            );
        }
    }
}

/// STATE. `run_resolve` asks `ensure_running` before it touches the gate,
/// and the ordering is the guarantee: a company that has stopped accepting
/// work must refuse the decision *and leave the approval parked*, so the
/// operator still has a card to decide once it is running again.
///
/// A refusal that consumed the park would be worse than no refusal at all —
/// the effect would be neither approved nor decidable.
#[tokio::test]
async fn resolving_on_a_paused_company_is_refused_and_leaves_the_approval_parked() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let approval = park_for_extend(&runtime, "appr-paused", crate::ports::now_millis()).await;
    let cookie = crate::server::test_support::fixed_cookie("acme");
    let app = router(state);

    let lifecycle = |verb: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/v1/companies/acme/{verb}"))
            .header("cookie", crate::server::test_support::fixed_cookie("acme"))
            .body(Body::empty())
            .unwrap()
    };

    let paused = app.clone().oneshot(lifecycle("pause")).await.unwrap();
    assert_eq!(paused.status(), StatusCode::OK, "the company is now paused");

    for verdict in ["approve", "deny"] {
        let refused = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/company/approvals/{approval}"))
                    .header("cookie", &cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "verdict": verdict }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            refused.status(),
            StatusCode::CONFLICT,
            "a paused company answered a {verdict} instead of refusing it"
        );
    }

    assert!(
        runtime.pending_approvals().iter().any(|p| p.id == approval),
        "the refusal must leave the approval decidable, not spend it"
    );
    assert_eq!(
        runtime.grants.live_count(),
        0,
        "and it must mint nothing on the way out"
    );

    let resumed = app.clone().oneshot(lifecycle("resume")).await.unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);
    let allowed = app
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(
        allowed.status(),
        StatusCode::OK,
        "the same decision lands once the company is running again"
    );
}

/// FAIL. On the synchronous shape the operator waits for the follow-up
/// cycle, so a cycle that falls over is theirs to hear about: the request
/// answers an error rather than a success over nothing.
///
/// And the verdict is durable regardless — it is settled inline, before the
/// cycle is ever spawned. The pairing is the point. An error that also lost
/// the decision would leave the operator re-approving something already
/// approved; an error swallowed into a `200` would leave them believing work
/// resumed that never did.
#[tokio::test]
async fn a_synchronous_resolve_reports_a_failed_follow_up_and_keeps_the_verdict() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 1, Some("sales"), true).await;
    let approval = c.approvals[0].clone();

    let response = c
        .app
        .clone()
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a continuation that fell over must reach the operator waiting on it"
    );

    assert!(
        !c.runtime
            .pending_approvals()
            .iter()
            .any(|p| p.id == approval),
        "the verdict is settled before the cycle runs, so a failed cycle cannot un-decide it"
    );
    assert_eq!(c.runtime.grants.live_count(), 1);

    let again = c
        .app
        .clone()
        .oneshot(resolve_request(
            &approval,
            serde_json::json!({ "verdict": "approve", "detach": true }),
        ))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        body_json(again).await["alreadyResolved"],
        true,
        "re-deciding after the failure must say it was already decided"
    );
    assert_eq!(
        c.runtime.grants.live_count(),
        1,
        "and must not buy a second permission"
    );
}

/// AUTH (IDOR). The client sends a `node_id` only; the host re-resolves it
/// within the *addressed* company's own tree rather than trusting the
/// caller. A real node minted under the exact same id in a different
/// company must not resolve through this one — proving the lookup is
/// scoped per company, not a global id space a guessable ULID could walk.
#[tokio::test]
async fn a_chat_attachment_cannot_cross_a_company_boundary() {
    let home_dir = home();
    let state = state_with_two_companies(home_dir.path()).await;
    let acme = state.registry().get(&CompanyId::new("acme")).unwrap();
    let globex = state.registry().get(&CompanyId::new("globex")).unwrap();

    // The exact same node id, minted for real, but only in globex.
    let shared_id = "n-cross-company";
    globex
        .workspace()
        .create_binary(
            &CompanyId::new("globex"),
            &attachment_binary_node(shared_id, "globex-only.png", "image/png"),
            b"globex bytes",
        )
        .await
        .unwrap();

    let app = router(state);
    let response = app
        .oneshot(chat_with_attachments("acme", vec![shared_id.to_string()]))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a node real in another company must not resolve through this one's chat"
    );
    assert!(
        acme.events()
            .read_from(&CompanyId::new("acme"), EventSeq::new(0), usize::MAX)
            .await
            .unwrap()
            .iter()
            .all(|stored| !matches!(stored.event, CompanyEvent::OperatorMessage { .. })),
        "a refused attachment must not journal a message with the wrong list"
    );
}

/// INPUT. An id naming nothing in this company's tree, and an id naming a
/// folder rather than a file, are both `400`s — but a genuine non-binary
/// **file** (a text note) is not: only the shape actually rejected is
/// rejected.
#[tokio::test]
async fn a_chat_attachment_refuses_an_unknown_id_and_a_folder_but_admits_a_note() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    runtime
        .workspace()
        .create(&id, &attachment_folder_node("f-1", "reports"), None)
        .await
        .unwrap();
    runtime
        .workspace()
        .create(
            &id,
            &attachment_note_node("n-1", "notes.md"),
            Some("just a note"),
        )
        .await
        .unwrap();

    let app = router(state);

    let unknown = app
        .clone()
        .oneshot(chat_with_attachments("acme", vec!["does-not-exist".into()]))
        .await
        .unwrap();
    assert_eq!(
        unknown.status(),
        StatusCode::BAD_REQUEST,
        "an id naming nothing in the tree must be refused"
    );

    let folder = app
        .clone()
        .oneshot(chat_with_attachments("acme", vec!["f-1".into()]))
        .await
        .unwrap();
    assert_eq!(
        folder.status(),
        StatusCode::BAD_REQUEST,
        "a folder id must be refused — it is not a file"
    );

    let admitted = app
        .oneshot(chat_with_attachments("acme", vec!["n-1".into()]))
        .await
        .unwrap();
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "a genuine non-binary file (a note) must still resolve"
    );
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].node_id, "n-1");
}

/// LIMIT. `MAX_CHAT_ATTACHMENTS` (20) is a hard cap on one message: one
/// over is refused before any tree scan or extraction runs, and exactly
/// at the cap is still ordinary, successful traffic.
#[tokio::test]
async fn a_chat_message_may_carry_at_most_twenty_attachments() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    let mut ids = Vec::new();
    for n in 0..21 {
        let node_id = format!("n-{n}");
        runtime
            .workspace()
            .create_binary(
                &id,
                &attachment_binary_node(
                    &node_id,
                    &format!("f{n}.bin"),
                    "application/octet-stream",
                ),
                b"x",
            )
            .await
            .unwrap();
        ids.push(node_id);
    }

    let app = router(state);

    let over_cap = app
        .clone()
        .oneshot(chat_with_attachments("acme", ids.clone()))
        .await
        .unwrap();
    assert_eq!(
        over_cap.status(),
        StatusCode::BAD_REQUEST,
        "21 attachments must be refused before any of them are resolved"
    );

    let at_cap = ids[..20].to_vec();
    let ok = app
        .oneshot(chat_with_attachments("acme", at_cap))
        .await
        .unwrap();
    assert_eq!(
        ok.status(),
        StatusCode::OK,
        "exactly 20 attachments is still ordinary traffic, not the refused shape"
    );
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(attachments.len(), 20);
}

/// BOUND. A repeated id collapses to exactly one resolved attachment — and
/// the cap is measured against the *raw* list the client sent, before
/// dedup, so a client cannot smuggle an over-cap request by repeating one
/// id past the limit and relying on dedup to shrink it back down.
#[tokio::test]
async fn a_chat_attachment_id_repeated_resolves_once_and_the_cap_counts_raw_entries() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let runtime = state.registry().get(&CompanyId::new("acme")).unwrap();
    let id = CompanyId::new("acme");
    runtime
        .workspace()
        .create_binary(
            &id,
            &attachment_binary_node("n-dup", "one.png", "image/png"),
            b"one",
        )
        .await
        .unwrap();

    let app = router(state);

    // 21 copies of the same id: one unique attachment after dedup, but the
    // raw count is still over MAX_CHAT_ATTACHMENTS.
    let over_cap_by_repetition = vec!["n-dup".to_string(); 21];
    let refused = app
        .clone()
        .oneshot(chat_with_attachments("acme", over_cap_by_repetition))
        .await
        .unwrap();
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "the cap must count the raw list the client sent, not the deduplicated one"
    );

    // Comfortably under the cap, repeated three times: dedup must collapse
    // it to exactly one resolved attachment.
    let repeated = vec!["n-dup".to_string(); 3];
    let ok = app
        .oneshot(chat_with_attachments("acme", repeated))
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let attachments = last_operator_message_attachments(&runtime, &id).await;
    assert_eq!(
        attachments.len(),
        1,
        "a repeated id must resolve to exactly one attachment, not one per repetition"
    );
    assert_eq!(attachments[0].node_id, "n-dup");
}
