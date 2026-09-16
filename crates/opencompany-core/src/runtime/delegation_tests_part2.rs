use super::tests_core2::*;

/// "By construction" means the card is on the board **while** the delegate
/// works, not reconstructed once they are done. Proven by reading the board
/// from inside the delegate's own turn: it is already there, already theirs,
/// already In progress.
///
/// This is the assertion that distinguishes the fix from a cosmetic one —
/// a card written only after the answer came back would satisfy every
/// count-based test above and still leave the work invisible for the whole
/// time it was actually happening.
#[tokio::test]
async fn the_hand_off_card_is_on_the_board_while_the_delegate_works() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("done")]);
    let outcome = fx
        .runner(&turns)
        .run_delegation(
            handoff("draft the launch plan"),
            None,
            MessageContext::default(),
        )
        .await
        .expect("delegation runs");
    assert!(outcome.spawned_task.is_some());
    assert_eq!(turns.calls().len(), 1, "the delegate ran exactly once");
    assert_eq!(
        turns.board_at_turn(0),
        vec![("engineer".to_string(), COLUMN_IN_PROGRESS.to_string())],
        "the card is open, assigned and in progress before the delegate starts"
    );
    // …and settles for a person once they are done.
    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].column, COLUMN_IN_REVIEW);
}

/// A hand-off an operator cancels mid-flight keeps its card and returns it
/// to To-do. The alternative — no card — would erase the fact that the work
/// was ever asked for.
#[tokio::test]
async fn a_cancelled_hand_off_returns_its_card_to_todo() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::cancelled("half-written")]);
    let outcome = fx
        .runner(&turns)
        .run_delegation(
            handoff("write the migration plan"),
            None,
            MessageContext::default(),
        )
        .await
        .expect("delegation runs");
    assert!(outcome.cancelled);
    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].column, COLUMN_TODO);
}

/// The constraint that keeps the fix from becoming its own bug, on the
/// hand-off path: relaying a question to a desk is not commissioning work.
#[tokio::test]
async fn a_question_relayed_to_a_desk_mints_no_card() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(
        &fx,
        vec![
            Turn::queueing("asking", vec![handoff("what's the status of the build?")]),
            Turn::reply("engineering says it's green"),
            Turn::reply("it's green"),
        ],
    );
    let turn = fx
        .runner(&turns)
        .handle_operator_message("chief", "is the build ok?", None)
        .await
        .expect("operator message handled");
    assert!(fx.cards().await.is_empty(), "a question is not work");
    assert!(turn.spawned_task.is_none());
}

/// A hand-off made from inside a **dispatched card** must not open a second
/// one — that card already is the tracking, and #204 hands it to the
/// delegate.
#[tokio::test]
async fn a_hand_off_inside_a_dispatched_card_opens_no_second_card() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("done")]);
    let outcome = fx
        .runner(&turns)
        .for_task("card-1")
        .run_delegation(
            handoff("write the migration plan"),
            None,
            MessageContext::default(),
        )
        .await
        .expect("delegation runs");
    assert!(outcome.spawned_task.is_none());
    assert!(fx.cards().await.is_empty());
}

// ── path one: a desk asked directly ─────────────────────────────────────

/// Asking a desk lead directly used to be the one path with no way to reach
/// the board at all: the card-opening tools are wired only onto the
/// orchestrator, so the desk did the work inline and nothing tracked it.
#[tokio::test]
async fn a_desk_asked_directly_opens_its_own_card() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("modules.md is written")]);
    let turn = fx
        .runner(&turns)
        .handle_operator_message(
            "engineer",
            "read the pricing repo and write modules.md",
            Some("eng_desk"),
        )
        .await
        .expect("operator message handled");

    // The issue's requirement is that the tracking decision is settled
    // BEFORE the work starts, so this reads the board from inside the desk's
    // own turn rather than only afterwards.
    assert_eq!(
        turns.board_at_turn(0),
        vec![("engineer".to_string(), COLUMN_IN_PROGRESS.to_string())],
        "the card is open before the desk begins working"
    );
    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0].assignee, "engineer");
    assert_eq!(cards[0].column, COLUMN_IN_REVIEW);
    assert_eq!(cards[0].origin_chat_id(), Some("eng_desk"));
    assert_eq!(
        cards[0].origin_parent(),
        None,
        "an unthreaded turn raises a card on the channel-level conversation",
    );
    assert_eq!(turn.spawned_task.as_deref(), Some(cards[0].id.as_str()));
}

/// Issue #1890 B — the card records **which thread** asked, not only which
/// channel.
///
/// Without this the settle marker lands flat in the channel, so an operator
/// who asked inside a thread watches their own thread never report the work
/// finishing. The runner is already bound to the raising turn's root (A's
/// `in_thread` builder); this is that root reaching the board.
#[tokio::test]
async fn a_card_raised_inside_a_thread_records_its_root() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("modules.md is written")]);
    fx.runner(&turns)
        .in_thread(Some(EventSeq::new(41)))
        .handle_operator_message(
            "engineer",
            "read the pricing repo and write modules.md",
            Some("eng_desk"),
        )
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(
        cards[0].origin_chat_id(),
        Some("eng_desk"),
        "the channel half is unchanged",
    );
    assert_eq!(
        cards[0].origin_parent(),
        Some(EventSeq::new(41)),
        "and the thread half is the root the turn was bound to",
    );
}

/// The same, for the card a `spawn_task` queues rather than the one a
/// hand-off opens. Two card-raising sites, one rule — and they are far
/// enough apart in this file that only a test keeps them agreeing.
#[tokio::test]
async fn a_spawned_card_records_the_thread_that_queued_it() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(
        &fx,
        vec![Turn::queueing(
            "opening a card",
            vec![Delegation::SpawnTask {
                title: "write the migration plan".to_string(),
                note: None,
                assignee: Some("engineer".to_string()),
            }],
        )],
    );
    fx.runner(&turns)
        .in_thread(Some(EventSeq::new(41)))
        .handle_operator_message("chief", "open a card for the migration", Some("general"))
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(cards[0].origin_chat_id(), Some("general"));
    assert_eq!(cards[0].origin_parent(), Some(EventSeq::new(41)));
}

/// **Issue #984, the reported probe.** The message that opened a card on
/// staging, run through the path that opened it.
///
/// `"verifying the Send button responds to a real mouse click. No action
/// needed from anyone."` is 15 words, so it clears
/// [`SMALLTALK_MAX_WORDS`]; it names no [`WORK_VERBS`] entry (`verifying`
/// and `send` are both deliberately absent — `send` is a noun here); and it
/// is not interrogative. So [`is_trackable_work`] falls through to its
/// "anything else is work" rung and returns true, which is how a message
/// that explicitly disclaimed any action became a card assigned to a desk.
///
/// The lexical layer cannot fix this without inverting its own default, so
/// the model is asked — and having been asked, its answer is now used.
#[tokio::test]
async fn a_desk_asked_something_the_model_calls_chatter_opens_no_card() {
    let probe = "verifying the Send button responds to a real mouse click. \
                 No action needed from anyone.";
    assert!(
        crate::company::task_intent::triage_message_detailed(probe).abstained(),
        "fixture must be a message no lexical rule decides"
    );
    assert!(
        is_trackable_work(probe),
        "fixture must be one the card detector would otherwise track — that \
         is the bug this closes"
    );

    let fx = Fixture::new();
    let escalation = ScriptedTriage::new(crate::harness::triage::TriageVerdict::Chatter);
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("ack")]);
    let turn = fx
        .runner(&turns)
        .with_triage(&escalation)
        .handle_operator_message("engineer", probe, Some("eng_desk"))
        .await
        .expect("operator message handled");

    assert_eq!(
        escalation.asked(),
        vec![probe.to_string()],
        "the abstention is what gets escalated"
    );
    assert!(
        fx.cards().await.is_empty(),
        "a message the model read as conversation opens no card"
    );
    assert_eq!(turn.spawned_task, None, "and nothing is linked to one");
}

/// The other direction, which is the one that must not regress: the model
/// says `work`, and the card is opened exactly as before.
///
/// This is what makes the change subtractive-only. `Work` and `Unavailable`
/// both leave the deterministic decision alone, so an escalation that is
/// slow, unreachable or unparseable cannot cost a card — only an explicit
/// `chatter` can.
#[tokio::test]
async fn a_non_chatter_verdict_still_opens_the_direct_card() {
    let residue = "the pricing page copy, before Friday if you can";
    assert!(
        crate::company::task_intent::triage_message_detailed(residue).abstained(),
        "fixture must be a message no lexical rule decides"
    );
    for verdict in [
        crate::harness::triage::TriageVerdict::Work,
        crate::harness::triage::TriageVerdict::Unavailable,
    ] {
        let fx = Fixture::new();
        let escalation = ScriptedTriage::new(verdict);
        let turns = ScriptedTurns::new(&fx, vec![Turn::reply("on it")]);
        fx.runner(&turns)
            .with_triage(&escalation)
            .handle_operator_message("engineer", residue, Some("eng_desk"))
            .await
            .expect("operator message handled");
        let cards = fx.cards().await;
        assert_eq!(
            cards.len(),
            1,
            "{verdict:?} must leave the card the abstention would have opened"
        );
        assert_eq!(cards[0].assignee, "engineer");
    }
}

/// One message, one card — including the road #463 could not see (issue #1035).
///
/// The REST chat handler opens a card on **two** signals: the triage naming
/// a title, and the operator's composer asking for a workflow, which it
/// takes as an override and supplies a title for when the triage declined
/// to. The runtime re-derived "did the handler card this?" from the triage
/// alone, which is true for the first road and false for the second — so a
/// workflow request whose wording no lexical rule recognises arrived here
/// looking uncarded and got a second card beside the one it already had.
///
/// The fixture is the same residue `a_non_chatter_verdict_still_opens_the_direct_card`
/// uses, and that is the point: with no deliverable it cards, so a run that
/// opens nothing here is the flag doing the work rather than the message
/// being unremarkable.
#[tokio::test]
async fn a_workflow_the_handler_already_carded_opens_no_second_card() {
    let residue = "the pricing page copy, before Friday if you can";
    assert!(
        crate::company::task_intent::triage_message_detailed(residue)
            .triage
            .title()
            .is_none(),
        "fixture must be a message the triage does NOT name — that is the \
         road the handler took its override on"
    );

    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("on it")]);
    let turn = fx
        .runner(&turns)
        .requested(Some(crate::ports::types::MessageIntent::Workflow))
        .handle_operator_message("engineer", residue, Some("eng_desk"))
        .await
        .expect("operator message handled");

    assert!(
        fx.cards().await.is_empty(),
        "the handler carded this message on the operator's request; the \
         runtime must not open a second one"
    );
    assert_eq!(turn.spawned_task, None, "and nothing is linked to one");
}

/// The same message with no composer choice still cards, so the test above
/// is not passing because the fixture stopped being trackable.
///
/// Without this pair the fix is unfalsifiable in the direction that matters:
/// a bug that suppressed *every* card would satisfy the assertion above and
/// fail nothing.
#[tokio::test]
async fn the_same_message_without_a_composer_choice_still_cards() {
    let residue = "the pricing page copy, before Friday if you can";
    for choice in [None, Some(crate::ports::types::MessageIntent::Once)] {
        let fx = Fixture::new();
        let turns = ScriptedTurns::new(&fx, vec![Turn::reply("on it")]);
        fx.runner(&turns)
            .requested(choice)
            .handle_operator_message("engineer", residue, Some("eng_desk"))
            .await
            .expect("operator message handled");
        assert_eq!(
            fx.cards().await.len(),
            1,
            "{choice:?} is not a workflow request, so the handler opened \
             nothing and this path still owes a card"
        );
    }
}

/// A copilot thread is the one surface where the deliverable must NOT be
/// read as "the handler carded it" (issue #1035).
///
/// The handler's condition is `!confined && deliverable == Workflow`, and
/// reproducing only the second half inverts this fix exactly here: a
/// conversation ABOUT one graph is not a request to build one, so the
/// handler deliberately cards nothing — and a runtime that concluded
/// otherwise would stand down the only paths left to open one.
#[tokio::test]
async fn a_workflow_request_on_a_copilot_thread_still_cards() {
    let residue = "the pricing page copy, before Friday if you can";
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("on it")]);
    fx.runner(&turns)
        .requested(Some(crate::ports::types::MessageIntent::Workflow))
        .handle_operator_message("engineer", residue, Some("workflow-copilot:weekly_report"))
        .await
        .expect("operator message handled");

    assert_eq!(
        fx.cards().await.len(),
        1,
        "the handler suppresses its override on a copilot thread, so this \
         message has no card yet and the runtime still owes one"
    );
}

/// **Issue #1152, the direct path.** The operator said this message is not
/// work, so the runtime opens no card for it either.
///
/// A handler-only fix would pass every REST test and still be wrong here.
/// The chat route is not the only thing that cards a chat message: this seam
/// opens one *by construction* whenever work is handed to an agent, and
/// [`is_trackable_work`]'s default is "everything is work". So "Just
/// chatting" would hold on an unaddressed message and fail on a message to a
/// desk — a label the company keeps only sometimes, which is worse than not
/// shipping the control.
///
/// The fixture is the residue `the_same_message_without_a_composer_choice_still_cards`
/// drives, and that pairing is what makes this non-vacuous: the same words
/// with `None` and with `Once` open exactly one card there, so a run that
/// opens none here is the operator's statement doing the work rather than
/// the message being unremarkable.
#[tokio::test]
async fn a_message_the_operator_sent_as_chat_opens_no_direct_card() {
    let residue = "the pricing page copy, before Friday if you can";
    assert!(
        crate::company::task_intent::triage_message_detailed(residue)
            .triage
            .title()
            .is_none(),
        "fixture must be a message the handler did NOT card on the triage, \
         or `carded_by_handler` would suppress this path anyway"
    );
    assert!(
        is_trackable_work(residue),
        "and one the card detector would otherwise track, or this proves nothing"
    );

    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("noted")]);
    let turn = fx
        .runner(&turns)
        .requested(Some(crate::ports::types::MessageIntent::Chat))
        .handle_operator_message("engineer", residue, Some("eng_desk"))
        .await
        .expect("operator message handled");

    assert_eq!(
        turn.reply, "noted",
        "the message is still answered — withholding a card is not silence"
    );
    assert!(
        fx.cards().await.is_empty(),
        "a message the operator sent as chat opens no card"
    );
    assert_eq!(turn.spawned_task, None, "and nothing is linked to one");
}

/// **Issue #1152, and it outranks the model too.** A `Work` verdict from the
/// triage escalation does not resurrect the card.
///
/// The two facts are peers, not a hierarchy the model sits on top of:
/// [`MessageContext::chatter`] is the model's reading of words it was shown,
/// and `not_work` is the author of those words saying what they meant. Where
/// they disagree the person wins. Without this, "Just chatting" would be
/// advisory on exactly the companies that wire an escalation — the ones
/// paying for a second opinion — and nothing would report the difference.
#[tokio::test]
async fn a_work_verdict_does_not_override_the_operators_own_statement() {
    let residue = "the pricing page copy, before Friday if you can";
    let fx = Fixture::new();
    let escalation = ScriptedTriage::new(crate::harness::triage::TriageVerdict::Work);
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("noted")]);
    fx.runner(&turns)
        .with_triage(&escalation)
        .requested(Some(crate::ports::types::MessageIntent::Chat))
        .handle_operator_message("engineer", residue, Some("eng_desk"))
        .await
        .expect("operator message handled");

    assert!(
        fx.cards().await.is_empty(),
        "the operator's own statement outranks a `work` verdict about their words"
    );
}

/// With **no escalation wired** — the default build, and any host without a
/// triage model — the behaviour is byte-identical to before issue #984.
///
/// Named because it is the property that makes this safe to ship: the fix
/// consults a model that most deployments do not have, and where it is
/// absent nothing about the board changes.
#[tokio::test]
async fn without_an_escalation_the_probe_still_cards_exactly_as_before() {
    let probe = "verifying the Send button responds to a real mouse click. \
                 No action needed from anyone.";
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("ack")]);
    fx.runner(&turns)
        .handle_operator_message("engineer", probe, Some("eng_desk"))
        .await
        .expect("operator message handled");
    assert_eq!(
        fx.cards().await.len(),
        1,
        "no model, no change — the bug is still here, and that is the point: \
         this path was not touched"
    );
}

/// **Issue #465, the reported card.** A desk asked directly, whose first
/// tool call parks for approval, produced nothing — so its card must not
/// present as reviewable work.
///
/// This path settled with a hardcoded [`TaskRunEnd::Completed`] and never
/// consulted the approval queue, so the card landed in In Review announcing
/// a result to check on work that had not started. It now parks, which is
/// where the operator can see it is blocked and where the console offers the
/// Resume that puts it back in flight once the call is authorised.
#[tokio::test]
async fn a_desk_whose_first_call_parks_leaves_its_card_blocked_not_reviewable() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(
        &fx,
        vec![Turn::parked(
            "I need approval before I can read the repo",
            "fs_read",
        )],
    );
    fx.runner(&turns)
        .handle_operator_message(
            "frontend_engineer",
            "read the pricing repo and write modules.md",
            Some("eng_desk"),
        )
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(
        cards[0].column, COLUMN_PAUSED,
        "a turn that parked its first call produced nothing to review"
    );
    assert_ne!(
        cards[0].column, COLUMN_IN_REVIEW,
        "In Review is what a review verdict approves straight to Done — \
         unstarted work must never sit there"
    );
}

/// The other half of the same decision: parking is what moves the landing,
/// not the mere presence of an approval queue. A turn that ran clean still
/// reaches the reviewer.
///
/// Paired with the test above so a fix that simply stopped writing In Review
/// would fail here.
#[tokio::test]
async fn a_desk_that_finished_cleanly_still_lands_in_review() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("modules.md is written")]);
    fx.runner(&turns)
        .handle_operator_message(
            "frontend_engineer",
            "read the pricing repo and write modules.md",
            Some("eng_desk"),
        )
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(cards[0].column, COLUMN_IN_REVIEW, "{cards:?}");
    assert_eq!(fx.approvals.queued(), 0, "nothing was parked");
}

/// An approval left over from an *earlier* turn must not park this card.
/// The count is differenced across the turn precisely so a queue the cycle
/// was already holding cannot be misread as something this turn did.
#[tokio::test]
async fn an_approval_parked_before_this_turn_does_not_park_its_card() {
    let fx = Fixture::new();
    // Something a previous turn parked and nobody has resolved yet.
    fx.approvals.push(crate::harness::policy::ApprovalRequest {
        tool: "send_email".to_string(),
        reason: "supervised".to_string(),
        effect: crate::ports::types::Effect {
            kind: "send_email".to_string(),
            group: crate::ports::types::EffectGroup::Other,
            amount_usd: None,
            established_thread: false,
            first_time_counterparty: false,
            payload: serde_json::json!({}),
            agent: Some("someone_else".to_string()),
            run_id: None,
        },
    });

    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("modules.md is written")]);
    fx.runner(&turns)
        .handle_operator_message(
            "frontend_engineer",
            "read the pricing repo and write modules.md",
            Some("eng_desk"),
        )
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(
        cards[0].column, COLUMN_IN_REVIEW,
        "this turn parked nothing of its own: {cards:?}"
    );
}

/// A desk **hand-off** whose turn parks has the same shape as the direct
/// path, and settles the same way — the delegate stopped at an unauthorised
/// call, so its card is blocked rather than reviewable.
#[tokio::test]
async fn a_hand_off_whose_turn_parks_also_leaves_its_card_blocked() {
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(
        &fx,
        vec![
            Turn::queueing("on it", vec![handoff("read the pricing repo")]),
            Turn::parked("I need approval before I can read the repo", "fs_read"),
            Turn::reply("relayed"),
        ],
    );
    fx.runner(&turns)
        .handle_operator_message("chief", "map out the pricing repo", Some("general"))
        .await
        .expect("operator message handled");

    let cards = fx.cards().await;
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert_eq!(
        cards[0].column, COLUMN_PAUSED,
        "the delegate parked its first call: {cards:?}"
    );
}

/// One message, one card. The REST chat handler already opens a To-do card
/// for a leading-imperative message before the cycle starts, so this path
/// must stand down for exactly those — otherwise "draft the launch plan"
/// lands on the board twice.
///
/// Found on a live host, not here: the unit tests above all used requests
/// the other detector is silent on, so nothing caught the overlap.
#[tokio::test]
async fn a_message_the_chat_handler_already_carded_opens_no_second_card() {
    // A leading imperative — `detect_task_intent` fires on this, so the REST
    // layer has already opened its card by the time the cycle runs.
    let imperative = "draft the launch plan for next quarter";
    assert!(
        crate::company::task_intent::detect_task_intent(imperative).is_some(),
        "fixture must be a message the chat handler cards, or this proves nothing"
    );
    let fx = Fixture::new();
    let turns = ScriptedTurns::new(&fx, vec![Turn::reply("planned")]);
    let turn = fx
        .runner(&turns)
        .handle_operator_message("engineer", imperative, Some("eng_desk"))
        .await
        .expect("operator message handled");
    assert!(
        fx.cards().await.is_empty(),
        "the chat handler's card is the card; this path opens none"
    );
    assert!(turn.spawned_task.is_none());
}
