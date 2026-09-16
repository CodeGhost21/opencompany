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

/// **Issue #379's routing, re-homed (issue #469).** The continuation
/// resumes in the thread the sign-off was raised in — and in no other.
///
/// Asserted in **both directions**, because either alone would pass on a
/// mistake. A desk channel's request and a direct message to that channel's
/// lead are answered by the same teammate, so a reply keyed on the agent
/// lands a channel's continuation in a private line nobody is watching, and
/// a reply keyed on the channel does the reverse.
///
/// This used to be pinned inside the harness brain, against a hand-built
/// grant. It moved here with the journaling: the thread comes off the park
/// record now, so the strong version of the test is the one that lets a real
/// turn stamp it and a real resolve read it back.
#[tokio::test]
async fn a_continuation_resumes_in_the_thread_it_was_raised_in_and_no_other() {
    async fn threads_for(chat: &str) -> Vec<String> {
        let home_dir = home();
        let c = multi_park_company(home_dir.path(), 1, Some(chat), false).await;
        let response = c
            .app
            .clone()
            .oneshot(approve_detached(&c.approvals[0]))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        settle(&c.runtime, 1).await;
        agent_replies(&c.runtime)
            .await
            .into_iter()
            .map(|r| r.split('|').next().unwrap().to_string())
            .collect()
    }

    // Raised in a desk channel: the continuation belongs to the channel.
    let desk = threads_for("desk-finance").await;
    assert_eq!(desk, vec!["desk-finance".to_string()]);
    assert_ne!(
        desk[0], "ceo",
        "a channel's approval must not resume in the desk lead's private DM"
    );

    // Raised in a direct message with that same lead: the mirror image.
    let dm = threads_for("ceo").await;
    assert_eq!(dm, vec!["ceo".to_string()]);
    assert_ne!(
        dm[0], "desk-finance",
        "a private line's approval must not resume in the desk channel"
    );
}

/// A single-approval turn is unchanged: it continues on that one decision,
/// exactly as it did before the gate existed.
#[tokio::test]
async fn a_lone_sign_off_still_continues_on_its_own_decision() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 1, None, false).await;
    let before = c.cycles.load(std::sync::atomic::Ordering::SeqCst);

    let response = c
        .app
        .clone()
        .oneshot(approve_detached(&c.approvals[0]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    settle(&c.runtime, 1).await;

    assert_eq!(
        c.cycles.load(std::sync::atomic::Ordering::SeqCst) - before,
        1
    );
    assert_eq!(agent_replies(&c.runtime).await.len(), 1);
}

/// **Defect 4.** A continuation that fails tells the person waiting for it.
///
/// The verdict and the grant are already durable at this point, so the
/// failure is recoverable — but only for somebody who knows it happened.
/// Before this the entire report was one `tracing::error!`: the agent was
/// not told the outcome, and neither was the operator, who saw an approval
/// they had granted produce nothing and had no way to tell a slow turn from
/// a dead one.
#[tokio::test]
async fn a_failed_continuation_tells_the_operator() {
    let home_dir = home();
    let c = multi_park_company(home_dir.path(), 2, Some("sales"), true).await;

    for id in &c.approvals {
        let response = c.app.clone().oneshot(approve_detached(id)).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "the verdict is durable regardless of what the turn then does"
        );
    }
    settle(&c.runtime, 1).await;

    let replies = agent_replies(&c.runtime).await;
    assert_eq!(
        replies.len(),
        1,
        "the operator is told exactly once that the work did not resume, got {replies:?}"
    );
    assert!(
        replies[0].starts_with("sales|"),
        "and told in the thread they approved in, got {replies:?}"
    );
    assert!(
        replies[0].contains("approving again is safe"),
        "the notice has to say what to do about it, got {replies:?}"
    );
    // Issue #966, asserted on the journaled row rather than on the
    // constructor: this drives the real approve path, so it pins that
    // `announce_continuation_failure` *calls* the named notice. Asserting
    // the constructor alone leaves the call site free to go back to an
    // inline `AgentReply` authored by the operator channel — a correct
    // system row byte-identical to one the pre-#885 defect damaged.
    let authors = agent_reply_authors(&c.runtime).await;
    assert_eq!(
        authors,
        vec![crate::ports::SYSTEM_AUTHOR.to_string()],
        "the runtime authored this notice, so it must not be stored under its destination"
    );
}

/// Codex review finding: a stream that errors mid-read used to fall
/// straight through to extraction on whatever partial bytes it had
/// collected. This pins the fix directly against a synthetic stream,
/// without needing a real workspace store behind it — a chunk, then an
/// error, must discard everything read so far rather than handing back
/// a truncated payload that looks complete.
#[tokio::test]
async fn drain_bounded_discards_everything_on_a_mid_stream_error() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> = vec![
        Ok(Bytes::from_static(b"the first chunk read fine")),
        Err(crate::error::OpenCompanyError::Store(
            "transient read failure".to_string(),
        )),
    ];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(drain_bounded(synthetic, 1_000_000).await, None);
}

/// The success twin: a stream with no error drains to its bytes, in
/// order, across however many chunks it arrives in.
#[tokio::test]
async fn drain_bounded_concatenates_every_chunk_when_the_stream_never_errors() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> = vec![
        Ok(Bytes::from_static(b"hello ")),
        Ok(Bytes::from_static(b"world")),
    ];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(
        drain_bounded(synthetic, 1_000_000).await,
        Some(b"hello world".to_vec())
    );
}

/// A stream that never errors but exceeds the cap is also discarded, not
/// truncated — the belt-and-braces the doc comment describes.
#[tokio::test]
async fn drain_bounded_discards_when_the_stream_exceeds_the_cap() {
    use bytes::Bytes;
    use futures::stream;

    let items: Vec<crate::error::Result<Bytes>> =
        vec![Ok(Bytes::from_static(b"way more than the cap allows"))];
    let synthetic: crate::ports::workspace::BlobStream = Box::pin(stream::iter(items));

    assert_eq!(drain_bounded(synthetic, 4).await, None);
}

/// **The Codex P1 finding:** `journal_chat_replies` resolved an agent
/// reply's mentions and stored them on `CompanyEvent::AgentReply`, but never
/// called `notify_mentions` — so an `@user` an agent typed *back* rendered
/// as a chip and left the named person with no durable notification and no
/// rail badge, unlike the operator's own message a few lines above it in
/// the very same function. Missing it worst for exactly the person it is
/// meant to reach: offline when the reply lands.
#[tokio::test]
async fn a_mention_in_an_agent_reply_notifies_the_person_it_names() {
    let home_dir = home();
    let state = build_state_with_brain(
        home_dir.path(),
        "running",
        AppConfig::default(),
        Some(Arc::new(MentioningReplyBrain)),
    )
    .await;
    // A second person for `@everyone` to reach — the sender is always
    // excluded from their own broadcast, so proving this needs somebody
    // else on the roster.
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let app = router(state.clone());
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    let member_id = runtime
        .users()
        .list_users(&id)
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.email == "harness-member@example.test")
        .expect("seeded member")
        .id;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/company/chat")
                .header("cookie", crate::server::test_support::fixed_cookie("acme"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"status?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let notified = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = runtime.notifications().list(&id, &member_id).await.unwrap();
            if !notes.is_empty() {
                return notes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the mentioned member was notified");

    assert_eq!(notified.len(), 1);
    assert_eq!(
        notified[0].notification.kind, "mention",
        "the reply's @everyone mention has to file the same kind of row an \
         operator message's does"
    );
}

/// **The Codex P1 finding:** the context a DM mention stores was decided by
/// the human user directory, but a DM's thread id is a roster teammate's
/// agent id — which no user record has — so a mention in a normal DM stored
/// the bare id. The console's rail keys a DM by `dm:<teammate-id>` (and the
/// console sends that bare id as the `chat` for a DM), so no rail row
/// displayed the badge and opening the DM could neither match nor clear it.
#[tokio::test]
async fn a_mention_in_a_dm_stores_the_console_dm_channel_id() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    // A second person for the broadcast to reach — the author is always
    // excluded from their own `@everyone`.
    crate::server::test_support::seed_fixed_member(&state, "acme").await;
    let app = router(state.clone());
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    let member_id = runtime
        .users()
        .list_users(&id)
        .await
        .unwrap()
        .into_iter()
        .find(|u| u.email == "harness-member@example.test")
        .expect("seeded member")
        .id;

    // A message addressed to the `designer` DM thread — the bare roster
    // teammate id, exactly what the console sends for a DM.
    let response = app
        .clone()
        .oneshot(chat_to("cc @everyone on this", Some("designer")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let notified = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let notes = runtime.notifications().list(&id, &member_id).await.unwrap();
            if !notes.is_empty() {
                return notes;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the mentioned member was notified");

    // The offline echo brain answers the same text, so `@everyone` may land
    // twice — once for the operator's message, once for the echoed reply.
    // The count is incidental; the invariant is that *every* mention filed
    // out of this exchange is keyed to the console's `dm:designer` channel,
    // not the bare roster thread id.
    assert!(!notified.is_empty(), "the mentioned member was notified");
    let contexts: Vec<_> = notified
        .iter()
        .map(|n| n.notification.context.as_deref())
        .collect();
    assert!(
        contexts.iter().all(|c| *c == Some("dm:designer")),
        "every mention in a DM has to store the console's DM channel id, \
         not the bare roster thread id — got {contexts:?}"
    );
}

/// [`mention_context`] canonicalizes a **`dm:`-prefixed** noncanonical key
/// too. An API client can address a DM with the console's channel shape but
/// a noncanonical payload — `dm:BACKEND_ENGINEER` for the teammate whose id
/// is `backend_engineer`. The routing resolves that case-insensitively, so
/// the stored context has to carry the canonical agent id: filing the raw
/// key under `dm:BACKEND_ENGINEER` badges a rail channel that does not
/// exist, and opening the actual DM can never clear it. Pre-fix, the
/// `dm:`-prefixed branch returned the key verbatim and bypassed
/// `assignee::resolve` entirely.
#[tokio::test]
async fn mention_context_canonicalizes_prefixed_dm_keys() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A case-variant of the teammate's id, carrying the `dm:` prefix the
    // console mints.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:BACKEND_ENGINEER")
            .await,
        "dm:backend_engineer",
        "a `dm:`-prefixed noncanonical teammate key has to store dm:<agent-id>"
    );
    // The already-canonical shape stays unchanged — the resolution must
    // not move a key that was already right.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a canonical dm:<teammate-id> key is kept as-is"
    );
    // A `dm:` key whose bare half names a desk (the desk-first ordering the
    // routing uses) files under the desk id, not a nonexistent `dm:<desk>`.
    assert_eq!(
        runtime.mention_context(&id, &[], "dm:Engineering").await,
        "engineering",
        "a `dm:` key that resolves to a desk has to store the desk id"
    );
}

/// A desk id that collides with a **human user id** still files under the
/// desk. `assignee::resolve`'s desk-first ordering — the same one
/// `responder_for` uses — outranks the user directory, and the directory
/// must not get a say ahead of it. Pre-fix, a `users` pre-check ran before
/// the resolution and returned `dm:<id>` for any bare key matching a human,
/// so a mention aimed at a desk whose id happened to match a human id would
/// badge a nonexistent DM channel and could never be cleared from the desk
/// it was meant for.
#[tokio::test]
async fn mention_context_a_human_id_matching_a_desk_id_stays_a_desk() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A human whose id collides with the `engineering` desk's id. The human
    // directory must not win: the message is aimed at the desk.
    let human = crate::ports::users::UserRecord {
        id: "engineering".to_string(),
        email: "human@example.test".to_string(),
        display_name: None,
        avatar: None,
        role: crate::ports::users::UserRole::Member,
        status: crate::ports::users::UserStatus::Active,
        password_hash: None,
        must_change_password: false,
        created_at_millis: crate::ports::now_millis(),
        last_seen_at_millis: None,
        updated_at_millis: crate::ports::now_millis(),
    };

    assert_eq!(
        runtime
            .mention_context(&id, std::slice::from_ref(&human), "engineering")
            .await,
        "engineering",
        "a desk id that matches a human id files under the desk, not dm:<id>"
    );
    assert_eq!(
        runtime
            .mention_context(&id, std::slice::from_ref(&human), "dm:engineering")
            .await,
        "engineering",
        "the same collision through a dm:-prefixed key still files under the desk"
    );
    // A DM the human is actually a teammate of still badges as a DM.
    assert_eq!(
        runtime
            .mention_context(&id, &[human], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a real DM channel is unaffected by the collision guard"
    );
}

/// [`mention_context`] resolves a `dm:`-prefixed key **as sent** before
/// stripping the prefix, so a desk literally named `dm:engineering` keeps
/// that id. Pre-fix, the unconditional strip resolved `engineering` instead
/// and filed the badge under the wrong transcript — the exact claim
/// [`assignee::dm_key`]'s contract warns about.
#[tokio::test]
async fn mention_context_a_desk_literally_named_dm_prefix_keeps_its_id() {
    let home_dir = home();
    let state = state_with_dm_prefixed_desk(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // The literal `dm:engineering` desk resolves as sent; stripping would
    // misroute to the plain `engineering` desk.
    assert_eq!(
        runtime.mention_context(&id, &[], "dm:engineering").await,
        "dm:engineering",
        "a desk literally named dm:<…> keeps its id — the raw key resolves first"
    );
    // The un-prefixed desk is untouched by the collision.
    assert_eq!(
        runtime.mention_context(&id, &[], "engineering").await,
        "engineering",
        "the un-prefixed desk still resolves to its own id"
    );
    // A genuine DM still re-keys onto the rail's DM channel.
    assert_eq!(
        runtime
            .mention_context(&id, &[], "dm:backend_engineer")
            .await,
        "dm:backend_engineer",
        "a real DM channel is unaffected by the literal dm: desk"
    );
}

/// [`mention_context`] stores the **canonical** id for a key typed in a
/// noncanonical shape — a desk by its display name, a teammate by a
/// case-variant of their id. `assignee::resolve` already returns canonical
/// ids (issue #214); storing the raw key instead would file the badge under
/// a channel id the rail never has, so it could neither render nor clear.
#[tokio::test]
async fn mention_context_stores_canonical_ids_for_noncanonical_keys() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    // A desk addressed by its display name files under the desk's id —
    // `"Engineering"` names the desk whose id is `engineering`.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a desk named by its display name has to store the desk id, not the raw key"
    );
    // A teammate addressed by a case-variant of their id files under the
    // canonical agent id, re-keyed into the console's DM channel space.
    assert_eq!(
        runtime.mention_context(&id, &[], "BACKEND_ENGINEER").await,
        "dm:backend_engineer",
        "a teammate named by a noncanonical key has to store dm:<agent-id>"
    );
}

/// [`mention_context`] files a mention in the General desk — the default an
/// unaddressed message lands in — under the console's canonical main-thread
/// id even when this company has no desk named/id `General`. This fixture's
/// only desk is `engineering`, so every general-chat spelling would
/// otherwise fall through to the raw string and badge a rail row that does
/// not exist (issue #1665 follow-up).
#[tokio::test]
async fn mention_context_maps_unresolvable_general_spellings_to_main() {
    let home_dir = home();
    let state = state_with_roster(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    for general in ["General", "general", "main", ""] {
        assert_eq!(
            runtime.mention_context(&id, &[], general).await,
            crate::server::chat_history::MAIN_THREAD_ID,
            "a mention in the General desk ({general:?}) has to store the console's \
             main-thread id, which the rail aliases onto its first rendered desk \
             channel"
        );
    }
    // A desk that does resolve keeps its canonical id — the general-chat
    // mapping must not swallow a real desk.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a real desk keeps its canonical id even when its name looks general"
    );
}

/// [`mention_context`] canonicalizes a **memberless** desk too. A desk that
/// exists but has nobody seated on it is still a real desk with a real rail
/// channel, so a key typed as its display name must file under its canonical
/// id: `"Sales"` has to badge `#sales`, and opening `#sales` has to clear it.
/// Pre-fix, `EmptyDesk` fell through the same wildcard as `Unknown` and
/// stored the raw key — a channel id no desk renders, so the badge was
/// invisible and could never clear.
#[tokio::test]
async fn mention_context_canonicalizes_a_memberless_desk() {
    let home_dir = home();
    let state = state_with_memberless_desk(home_dir.path()).await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).expect("company registered");

    assert_eq!(
        runtime.mention_context(&id, &[], "Sales").await,
        "sales",
        "a memberless desk named by its display name has to store the desk id, \
         not the raw key — the rail's channel id is `sales`"
    );
    // The desk that does have a lead keeps behaving as before.
    assert_eq!(
        runtime.mention_context(&id, &[], "Engineering").await,
        "engineering",
        "a desk with a lead still stores its canonical id"
    );
}

/// Issue #1781 review (Codex P1): [`company_events`]'s periodic refresh
/// must re-derive admin access from the live user record, not keep
/// answering with whatever it was when the SSE stream opened. Proven
/// directly against [`refreshed_is_admin`] — the seam that refresh loop
/// calls on every tick — rather than the SSE handler itself, since the
/// handler's own timing (a real `EventSource`, a 60s interval) is not
/// what this bug is about.
#[tokio::test]
async fn refreshed_is_admin_reflects_a_mid_stream_demotion() {
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

    assert!(
        refreshed_is_admin(&runtime, Some(&actor), false).await,
        "an active admin's record must resolve to admin, even starting from a stale `false`"
    );

    // The demotion itself: same shape `PATCH …/users/{id}` writes, and —
    // critically — it does not touch sessions, so a connection opened
    // before this write stays open exactly as it would in production.
    user.role = crate::ports::users::UserRole::Member;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a demoted user's live record must flip a stale `true` to `false` — this is \
         exactly the check `company_events` failed to make before this fix, leaking the \
         owner-fallback admin-only report to a demoted viewer for the rest of their stream"
    );

    // Suspension revokes admin the same way, even if role were untouched.
    user.role = crate::ports::users::UserRole::Admin;
    user.status = crate::ports::users::UserStatus::Suspended;
    runtime
        .users()
        .upsert_user(runtime.id(), &user)
        .await
        .unwrap();

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a suspended admin must not keep admin-only visibility either"
    );
}

/// Issue #1781 review, Codex P1 second follow-up: a human actor whose
/// current role cannot be confirmed — `Ok(None)` because the user record
/// has gone missing, folded in here with a genuine store error since both
/// hit the same match arm — must resolve to `false`, not `previous`.
///
/// `previous: true` here stands in for exactly the dangerous case: a
/// cached "was admin" value from before whatever made this actor
/// unconfirmable, revalidated at the one call site
/// (`is_admin_for_item`) that gates the admin-only owner-fallback report
/// on this result directly. Before this fix, an actor deleted out from
/// under an open SSE stream — or a transient read failure landing at the
/// exact moment a report needed gating — fell back to `previous` and kept
/// leaking the report, silently, for as long as the failure (or the
/// missing record) persisted.
#[tokio::test]
async fn refreshed_is_admin_fails_closed_when_the_user_record_cannot_be_found() {
    let home_dir = home();
    let state = state_with_company(home_dir.path(), "running").await;
    let id = CompanyId::new("acme");
    let runtime = state.registry().get(&id).unwrap();

    // Never upserted — `get_user` answers `Ok(None)`, the "record has
    // gone missing" half of the case this proves.
    let actor = Actor {
        kind: ActorKind::User,
        id: "ghost".to_string(),
    };

    assert!(
        !refreshed_is_admin(&runtime, Some(&actor), true).await,
        "a human actor with no resolvable user record must read as not-admin \
         even when the cached value being revalidated was `true` — trusting \
         `previous` here is exactly the fail-open gap this fix closes"
    );
}
