use super::*;
use crate::company::CompanyManifest;
use crate::ports::types::CompanyId;
use crate::runtime::RuntimeBuilder;

fn manifest() -> CompanyManifest {
    toml::from_str("[company]\nname = \"Acme\"\n[policy]\nmode = \"full\"\n")
        .expect("parse manifest")
}

async fn runtime(home: &std::path::Path) -> Arc<CompanyRuntime> {
    Arc::new(
        RuntimeBuilder::new(home.to_path_buf(), manifest())
            .with_id(CompanyId::new("acme"))
            .build()
            .await
            .expect("build a runtime"),
    )
}

/// Helper: the marker and the agent-authored line it caused, as one leg.
fn referral_leg(
    from_desk: &str,
    from_desk_name: &str,
    asker: &str,
    to_desk: &str,
    target: &str,
    returning: bool,
    text: &str,
) -> [CompanyEvent; 2] {
    referral_leg_answering(
        from_desk,
        from_desk_name,
        asker,
        to_desk,
        target,
        returning,
        text,
        None,
    )
}

/// The same, with the forward this return answers named explicitly — the
/// pointer the host records so the projection need not scan for it.
#[expect(clippy::too_many_arguments, reason = "a journal event's own shape")]
fn referral_leg_answering(
    from_desk: &str,
    from_desk_name: &str,
    asker: &str,
    to_desk: &str,
    target: &str,
    returning: bool,
    text: &str,
    answers: Option<u64>,
) -> [CompanyEvent; 2] {
    [
        CompanyEvent::ReferralEnqueued {
            // These fixtures are desk crossings, which run on the target's
            // own desk and name no pair conversation.
            conversation: None,
            answers,
            from_desk: from_desk.to_string(),
            from_desk_name: from_desk_name.to_string(),
            asker: asker.to_string(),
            asker_label: asker.to_string(),
            trigger_sequence: 1,
            to_desk: to_desk.to_string(),
            target: target.to_string(),
            returning,
        },
        CompanyEvent::OperatorMessage {
            text: text.to_string(),
            by: Some(Actor {
                kind: ActorKind::Agent,
                id: asker.to_string(),
            }),
            chat: Some(to_desk.to_string()),
            parent: None,
            deliverable: None,
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
    ]
}

/// **An episode's turns are the conversation; its closing row is not.**
///
/// The room journals every turn as an ordinary reply by the teammate that
/// took it, then one summary under `hive-report`. Rendered, that summary
/// appeared as a *teammate* — a participant in a channel where no such
/// teammate exists and none can, since the id is hyphenated exactly so no
/// roster id can equal it. The fold already reads it as `System`; this
/// makes the console agree.
///
/// The turns must survive: dropping the room and keeping only its summary
/// would hide the reasoning, the losing options and every objection — the
/// one thing a room produces that a single answer cannot.
#[tokio::test]
async fn an_episodes_turns_render_but_its_closing_row_does_not() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    for (agent, text) in [
        (
            "software_engineer",
            "!propose #lazy-load defer each section",
        ),
        (
            "junior_engineer",
            "!object >1 ^1 users bounce between sections",
        ),
        (
            crate::hivemind::HIVE_REPORT_AUTHOR,
            "The desk settled after 2 turns (#lazy-load, backed by software_engineer): defer each section",
        ),
    ] {
        runtime
            .events()
            .append(
                &id,
                CompanyEvent::AgentReply {
                    chat_id: "engineering".to_string(),
                    agent_id: agent.to_string(),
                    text: text.to_string(),
                    steps: Vec::new(),
                    task_id: None,
                    outputs: Vec::new(),
                    parent: None,
                    mentions: Vec::new(),
                    mention_depth: 0,
                    audience: Vec::new(),
                },
            )
            .await
            .expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");

    let voices: Vec<&str> = history.iter().map(|m| m.channel.as_str()).collect();
    assert!(
        voices.contains(&"software_engineer") && voices.contains(&"junior_engineer"),
        "every teammate's turn is on screen, the objection included: {voices:?}"
    );
    assert!(
        !voices.contains(&crate::hivemind::HIVE_REPORT_AUTHOR),
        "and the room's own bookkeeping is not a participant in it: {voices:?}"
    );
}

/// **A suppressed row must not shorten the page.**
///
/// Filtered after the page was assembled, an episode's closing row silently
/// cost the reader a message: a page asked for `n` came back with `n - 1`,
/// and the row that should have taken its place stayed unfetched. The
/// admission point already excludes an admin-only row for exactly this
/// reason, and says so.
#[tokio::test]
async fn a_suppressed_report_does_not_shorten_the_page() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    // Four teammate turns with the room's closing row in the middle.
    for (agent, text) in [
        ("software_engineer", "first"),
        ("junior_engineer", "second"),
        (crate::hivemind::HIVE_REPORT_AUTHOR, "The desk settled."),
        ("qa_engineer", "third"),
        ("software_engineer", "fourth"),
    ] {
        runtime
            .events()
            .append(
                &id,
                CompanyEvent::AgentReply {
                    chat_id: "engineering".to_string(),
                    agent_id: agent.to_string(),
                    text: text.to_string(),
                    steps: Vec::new(),
                    task_id: None,
                    outputs: Vec::new(),
                    parent: None,
                    mentions: Vec::new(),
                    mention_depth: 0,
                    audience: Vec::new(),
                },
            )
            .await
            .expect("journal");
    }

    let page = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        4,
        true,
    )
    .await
    .expect("history");

    assert_eq!(
        page.len(),
        4,
        "a page of four is four teammate turns, not three and a hole: {page:?}"
    );
    assert!(
        page.iter()
            .all(|m| m.channel != crate::hivemind::HIVE_REPORT_AUTHOR),
        "and none of them is the room's bookkeeping: {page:?}"
    );
}

/// **A failed turn still shows.** The report restates a tally whose inputs
/// are the visible turns, so hiding it costs nothing. A failure notice
/// describes a turn that does not exist — there is no gap for a reader to
/// notice — so hiding it would leave a transcript with an unaccounted hole.
#[tokio::test]
async fn a_failed_turn_is_still_reported_to_the_room() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    for (agent, text) in [
        (
            crate::hivemind::HIVE_FAILURE_AUTHOR,
            "qa_engineer was asked and could not answer.",
        ),
        (
            crate::hivemind::HIVE_REPORT_AUTHOR,
            "The desk settled after 2 turns.",
        ),
    ] {
        runtime
            .events()
            .append(
                &id,
                CompanyEvent::AgentReply {
                    chat_id: "engineering".to_string(),
                    agent_id: agent.to_string(),
                    text: text.to_string(),
                    steps: Vec::new(),
                    task_id: None,
                    outputs: Vec::new(),
                    parent: None,
                    mentions: Vec::new(),
                    mention_depth: 0,
                    audience: Vec::new(),
                },
            )
            .await
            .expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let voices: Vec<&str> = history.iter().map(|m| m.channel.as_str()).collect();

    assert!(
        voices.contains(&crate::hivemind::HIVE_FAILURE_AUTHOR),
        "a seat that could not answer is accounted for: {voices:?}"
    );
    assert!(
        !voices.contains(&crate::hivemind::HIVE_REPORT_AUTHOR),
        "while the closing summary stays out of the room: {voices:?}"
    );
}

/// **A referred line is the ASKING AGENT speaking, not the desk.**
///
/// `senderOf` in the console draws the byline off `channel`, and treats
/// "operator" as "no distinct speaker — use the room's own name". That is
/// right for a message a person sent and wrong for a referral, which
/// arrives authored by a teammate: hardcoding "operator" made design's own
/// name the speaker, so an engineer asking design read as design talking to
/// itself. An `AgentReply` already names its agent here; this makes the two
/// paths agree rather than teaching the console a second rule.
#[tokio::test]
async fn a_referred_message_is_voiced_by_the_agent_that_asked() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    for event in referral_leg(
        "engineering",
        "Engineering",
        "software_engineer",
        "design",
        "product_designer",
        false,
        "what would you change about the error messages?",
    ) {
        runtime.events().append(&id, event).await.expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "design",
        "design",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let referred = history
        .iter()
        .find(|m| m.referred_from.is_some())
        .expect("the referred line");

    assert_eq!(
        referred.channel, "software_engineer",
        "the byline names the agent, not the desk it landed on: {referred:?}"
    );
    assert!(
        !referred.by_person,
        "an agent is not a person, whatever the event it rides on"
    );
}

/// **One agent speaks in both rooms, and it is the asker.**
///
/// The asker asks on the other desk under its own name; that desk answers
/// on its own desk; the asker comes home and reports. The relay that
/// carried the answer back is an input to the asker, not a line anyone
/// reads — rendering it put the other desk's agent in a room it is not part
/// of, saying the same thing the asker was about to say.
#[tokio::test]
async fn the_asker_brings_the_answer_home_and_the_other_desk_stays_out_of_the_room() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    let mut events: Vec<CompanyEvent> = referral_leg(
        "engineering",
        "Engineering",
        "software_engineer",
        "design",
        "product_designer",
        false,
        "what would you change about the error messages?",
    )
    .into_iter()
    .chain(referral_leg(
        "design",
        "Design",
        "product_designer",
        "engineering",
        "software_engineer",
        true,
        "error messages look like a copy task and are not one",
    ))
    .collect();
    // The asker's report — the only thing #engineering should show.
    events.push(CompanyEvent::AgentReply {
        chat_id: "engineering".to_string(),
        agent_id: "software_engineer".to_string(),
        text: "design came back: error messages are a design-system problem".to_string(),
        steps: Vec::new(),
        task_id: None,
        outputs: Vec::new(),
        parent: None,
        mentions: Vec::new(),
        mention_depth: 0,
        audience: Vec::new(),
    });
    for event in events {
        runtime.events().append(&id, event).await.expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");

    assert!(
        history.iter().all(|m| m.channel != "product_designer"),
        "the answering desk never speaks in the room it was asked from: {history:?}"
    );
    let referred = history
        .iter()
        .find(|m| m.referred_from.is_some())
        .expect("something carries the provenance");
    assert_eq!(
        referred.channel, "software_engineer",
        "the chip rides the asker's own report: {referred:?}"
    );
    let origin = referred.referred_from.as_ref().expect("origin");
    assert!(origin.returning, "and it reads as an answer, not an ask");
    assert_eq!(origin.desk_name, "Design");

    // **The crossing itself rides the report, both legs of it.**
    //
    // The relayed rows still go — that is the assertion above, and the
    // reason for it — but the exchange they carried is kept here so an
    // operator can read what was actually asked and answered instead of
    // only the asker's paraphrase of it. `lines.len()` is the count the
    // collapsed label shows, which is why the QUESTION has to be captured
    // too: an answer on its own would always read "1 message".
    let crossing = referred
        .referral_conversation
        .as_ref()
        .expect("the crossing rides the report that brought it home");
    assert_eq!(crossing.asker_id, "software_engineer");
    assert_eq!(crossing.other_id, "product_designer");
    assert_eq!(crossing.other_desk_name, "Design");
    assert_eq!(crossing.lines.len(), 2, "{:?}", crossing.lines);
    assert!(crossing.lines[0].outbound, "the question goes out first");
    assert_eq!(
        crossing.lines[0].text,
        "what would you change about the error messages?"
    );
    assert!(!crossing.lines[1].outbound, "then the answer comes back");
    assert_eq!(
        crossing.lines[1].text,
        "error messages look like a copy task and are not one"
    );
}

/// **A crossing that convened the far desk folds what that desk SAID.**
///
/// The collapsed crossing exists so an operator can read the exchange
/// rather than the asker's paraphrase of it. A deliberated crossing has a
/// whole conversation to show — every turn journaled on the far desk — and
/// folding only the conclusion showed none of it: "asked #design ·
/// 2 messages" over a question and one summary, for a room that ran three
/// turns across both seats.
///
/// The question is unwrapped too. A room has to READ the question to
/// deliberate on it, so the whole referral prompt is journaled on that
/// desk, and the matcher finds that row — rendering "…has asked you a
/// question. Answer it from what you and this desk know." where the ask
/// belongs.
#[tokio::test]
async fn a_convened_desks_own_turns_are_the_folded_crossing() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    let asked = runtime
        .events()
        .append(
            &id,
            CompanyEvent::AgentReply {
                chat_id: "engineering".to_string(),
                agent_id: "software_engineer".to_string(),
                text: "!question @#design ^2 can the error messages be redone?".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
                audience: Vec::new(),
            },
        )
        .await
        .expect("journal");
    let forward = runtime
        .events()
        .append(
            &id,
            CompanyEvent::ReferralEnqueued {
                conversation: None,
                answers: None,
                from_desk: "engineering".to_string(),
                from_desk_name: "Engineering".to_string(),
                asker: "software_engineer".to_string(),
                asker_label: "software_engineer".to_string(),
                trigger_sequence: asked.value(),
                to_desk: "design".to_string(),
                target: "product_designer".to_string(),
                returning: false,
            },
        )
        .await
        .expect("journal");
    // The prompt the room was handed, on the desk being asked — the row a
    // deliberated crossing journals, the one the matcher finds, and the
    // thread every turn of that room is rooted on.
    let root = runtime
        .events()
        .append(
            &id,
            CompanyEvent::OperatorMessage {
                text: crate::hivemind::referral::referral_room_prompt(
                    "software_engineer",
                    "Engineering",
                    "can the error messages be redone?",
                ),
                by: Some(Actor {
                    kind: ActorKind::Agent,
                    id: "software_engineer".to_string(),
                }),
                chat: Some("design".to_string()),
                parent: None,
                deliverable: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
            },
        )
        .await
        .expect("journal");
    // The room: both seats, plus its closing row, which is the desk's own
    // bookkeeping and never a line somebody said. Every one parented on the
    // question, as a referred episode journals them.
    let mut events: Vec<CompanyEvent> = [
        (
            "product_designer",
            "!propose #copy they read like a copy task",
        ),
        ("researcher", "!support #copy ^1 and the tests agree"),
        (crate::hivemind::HIVE_REPORT_AUTHOR, "The desk settled."),
    ]
    .into_iter()
    .map(|(agent, text)| CompanyEvent::AgentReply {
        chat_id: "design".to_string(),
        agent_id: agent.to_string(),
        text: text.to_string(),
        steps: Vec::new(),
        task_id: None,
        outputs: Vec::new(),
        parent: Some(root),
        mentions: Vec::new(),
        mention_depth: 0,
        audience: Vec::new(),
    })
    .collect();
    // **Concurrent traffic on the same desk, inside the same window.** A
    // desk that was asked keeps working while the referred room runs, and a
    // fold scoped by sequence interval alone renders this as part of the
    // crossing.
    events.push(CompanyEvent::AgentReply {
        chat_id: "design".to_string(),
        agent_id: "product_designer".to_string(),
        text: "unrelated: the icon set ships Thursday".to_string(),
        steps: Vec::new(),
        task_id: None,
        outputs: Vec::new(),
        parent: None,
        mentions: Vec::new(),
        mention_depth: 0,
        audience: Vec::new(),
    });
    for event in events {
        runtime.events().append(&id, event).await.expect("journal");
    }
    // The return, naming the forward it answers, carrying the room's note.
    for event in [
        CompanyEvent::ReferralEnqueued {
            conversation: None,
            answers: Some(forward.value()),
            from_desk: "design".to_string(),
            from_desk_name: "Design".to_string(),
            asker: "product_designer".to_string(),
            asker_label: "product_designer".to_string(),
            trigger_sequence: asked.value(),
            to_desk: "engineering".to_string(),
            target: "software_engineer".to_string(),
            returning: true,
        },
        CompanyEvent::AgentReply {
            chat_id: "engineering".to_string(),
            agent_id: crate::hivemind::HIVE_REFERRAL_AUTHOR.to_string(),
            text: crate::hivemind::referral::room_note("Design", "The desk settled."),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
            parent: None,
            mentions: Vec::new(),
            mention_depth: 0,
            audience: Vec::new(),
        },
        // The asker's own report, which the crossing folds onto.
        CompanyEvent::AgentReply {
            chat_id: "engineering".to_string(),
            agent_id: "software_engineer".to_string(),
            text: "design says it is a copy problem".to_string(),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
            parent: None,
            mentions: Vec::new(),
            mention_depth: 0,
            audience: Vec::new(),
        },
    ] {
        runtime.events().append(&id, event).await.expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let crossing = history
        .iter()
        .find_map(|m| m.referral_conversation.as_ref())
        .expect("the crossing rides the asker's report");

    assert_eq!(
        crossing.lines.len(),
        3,
        "the question and both seats that answered it: {:?}",
        crossing.lines
    );
    let question = &crossing.lines[0];
    assert!(question.outbound);
    assert_eq!(
        question.text, "can the error messages be redone?",
        "the ask, not the instructions wrapped around it"
    );
    assert_eq!(crossing.lines[1].author_id, "product_designer");
    assert_eq!(crossing.lines[2].author_id, "researcher");
    assert!(
        crossing.lines[1..].iter().all(|line| !line.outbound),
        "both came back from the other desk"
    );
    assert!(
        crossing
            .lines
            .iter()
            .all(|line| !line.text.contains("The desk settled")),
        "the closing row is the desk's bookkeeping, not a line anybody said: {:?}",
        crossing.lines
    );
    assert!(
        crossing
            .lines
            .iter()
            .all(|line| !line.text.contains("answered the question")),
        "and the relayed note is dropped — it summarises exactly these lines: {:?}",
        crossing.lines
    );
    assert!(
        crossing
            .lines
            .iter()
            .all(|line| !line.text.contains("icon set")),
        "and a reply this desk made outside the crossing is not part of it: {:?}",
        crossing.lines
    );
}

/// **Another pair's crossing does not end this one's window.**
///
/// The child search is bounded so a crossing that FAILED cannot latch onto
/// its target's next unrelated reply. `page` is the company's journal
/// though, not this pair's, so bounding at the next marker of ANY kind let
/// a third desk's crossing end the window — and a child journaled after it
/// was skipped, dropping an answered crossing from the projection
/// altogether. Matching `to_desk` alone is not enough either: two desks can
/// ask the same one (Codex and CodeRabbit both, #2332).
#[tokio::test]
async fn an_unrelated_pairs_marker_does_not_end_this_crossings_window() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    let marker =
        |from: &str, to: &str, asker: &str, target: &str| CompanyEvent::ReferralEnqueued {
            conversation: None,
            answers: None,
            from_desk: from.to_string(),
            from_desk_name: from.to_string(),
            asker: asker.to_string(),
            asker_label: asker.to_string(),
            trigger_sequence: 1,
            to_desk: to.to_string(),
            target: target.to_string(),
            returning: false,
        };
    // This crossing: engineering asks design.
    runtime
        .events()
        .append(
            &id,
            marker(
                "engineering",
                "design",
                "software_engineer",
                "product_designer",
            ),
        )
        .await
        .expect("journal");
    // A crossing between two entirely unrelated desks, interleaved BEFORE
    // our child lands. It ends no window of ours — but the unscoped bound
    // stopped here, so the child below was never reached.
    runtime
        .events()
        .append(&id, marker("sales", "triage", "ae", "triager"))
        .await
        .expect("journal");
    // Our child: the target's own turn on the desk that was asked.
    runtime
        .events()
        .append(
            &id,
            CompanyEvent::AgentReply {
                chat_id: "design".to_string(),
                agent_id: "product_designer".to_string(),
                text: "they read like a copy task".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
                audience: Vec::new(),
            },
        )
        .await
        .expect("journal");

    let history = history_for_desk(
        &runtime,
        "design",
        "design",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let answered = history
        .iter()
        .find(|m| m.referred_from.is_some())
        .expect("the crossing still finds its child past another pair's marker");
    assert_eq!(
        answered.referred_from.as_ref().expect("origin").desk_id,
        "engineering",
        "and it is attributed to the desk that actually asked"
    );
    assert_eq!(
        answered.text, "they read like a copy task",
        "the child is the target's own turn, found past the unrelated marker"
    );
}

/// **The desk that was ASKED can read the question it answered.**
///
/// A room crossing journals the question on the asking desk — the asker's
/// own committed line — and nothing at all on the desk it goes to; the far
/// seat simply takes a turn there. So the answering side rendered an answer
/// to a question nobody on that desk could see: `product_designer`
/// explaining what they would change, over a chip reading "Asked by
/// @software_engineer", and the question itself one desk away.
///
/// Folded with the roles swapped (`inbound`), because every other field on
/// a crossing is named from the asking side. One line, not two: the row it
/// hangs on is this desk's answer already.
#[tokio::test]
async fn the_answering_desk_carries_the_question_it_was_asked() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    // The ask, as it was said — a committed MOVE on the asking desk, which
    // is the only place a room crossing's question exists. Its sequence is
    // read back from the append rather than assumed: the marker points at
    // this row, and a runtime that has journalled anything at boot makes
    // any guess wrong.
    let asked = runtime
        .events()
        .append(
            &id,
            CompanyEvent::AgentReply {
                chat_id: "engineering".to_string(),
                agent_id: "software_engineer".to_string(),
                text: "!question @#design ^2 can the error messages be redone?".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
                audience: Vec::new(),
            },
        )
        .await
        .expect("journal");
    for event in [
        CompanyEvent::ReferralEnqueued {
            conversation: None,
            answers: None,
            from_desk: "engineering".to_string(),
            from_desk_name: "Engineering".to_string(),
            asker: "software_engineer".to_string(),
            asker_label: "software_engineer".to_string(),
            trigger_sequence: asked.value(),
            to_desk: "design".to_string(),
            target: "product_designer".to_string(),
            returning: false,
        },
        // The far seat's turn, on its own desk and under its own id — the
        // only row this crossing leaves here.
        CompanyEvent::AgentReply {
            chat_id: "design".to_string(),
            agent_id: "product_designer".to_string(),
            text: "they read like a copy task and are not one".to_string(),
            steps: Vec::new(),
            task_id: None,
            outputs: Vec::new(),
            parent: None,
            mentions: Vec::new(),
            mention_depth: 0,
            audience: Vec::new(),
        },
    ] {
        runtime.events().append(&id, event).await.expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "design",
        "design",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let answered = history
        .iter()
        .find(|m| m.referred_from.is_some())
        .expect("the answering turn carries the provenance");
    assert_eq!(
        answered.text, "they read like a copy task and are not one",
        "the desk's own answer is still the row"
    );

    let crossing = answered
        .referral_conversation
        .as_ref()
        .expect("and the question it answered rides it");
    assert!(
        crossing.inbound,
        "this desk was asked; the label reads \"asked by\" rather than \"asked\""
    );
    assert_eq!(
        crossing.asker_id, "product_designer",
        "the local side first"
    );
    assert_eq!(crossing.other_id, "software_engineer");
    assert_eq!(crossing.other_desk_id, "engineering");
    assert_eq!(crossing.other_desk_name, "Engineering");
    assert_eq!(
        crossing.lines.len(),
        1,
        "the answer is the row, so folding it too would print it twice: {:?}",
        crossing.lines
    );
    let question = &crossing.lines[0];
    assert!(!question.outbound, "the question came IN to this desk");
    assert_eq!(question.author_id, "software_engineer");
    assert!(
        question.text.contains("can the error messages be redone?"),
        "{:?}",
        question.text
    );
    assert!(
        !question.text.starts_with('!'),
        "a room's move grammar is addressed to the fold, not to a reader: {:?}",
        question.text
    );
}

/// **The marker names its own forward, so the ask is found however far back
/// it is.**
///
/// The scan this replaces looked back a fixed number of events from the
/// oldest visible row, so a crossing whose ask fell outside that window
/// rendered with the answer alone and said "1 message" — quietly wrong, and
/// wrong in the direction that looks plausible. The host already located
/// that marker to authorize the return and was keeping only a bool;
/// `answers` records it instead.
///
/// Here the two legs are separated by far more than the scan's `LOOKBACK`,
/// so the fallback cannot reach the ask and only the pointer can.
#[tokio::test]
async fn a_marker_that_names_its_forward_pairs_beyond_the_scan_window() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    // The marker's OWN sequence, not a guess at it: the runtime journals
    // its own setup rows first, so the first referral event is not
    // sequence zero. Pointing `answers` at a sequence that happens to hold
    // something else is the confusion the pointer exists to remove.
    let mut forward_seq = 0u64;
    for (i, event) in referral_leg(
        "engineering",
        "Engineering",
        "software_engineer",
        "design",
        "product_designer",
        false,
        "what would you change about the error messages?",
    )
    .into_iter()
    .enumerate()
    {
        let seq = runtime.events().append(&id, event).await.expect("journal");
        if i == 0 {
            forward_seq = seq.value();
        }
    }
    // The forward marker is sequence 0, its question 1. Bury them under
    // enough unrelated traffic that the scan's window cannot reach back.
    for i in 0..200 {
        runtime
            .events()
            .append(
                &id,
                CompanyEvent::AgentReply {
                    chat_id: "engineering".to_string(),
                    agent_id: "software_engineer".to_string(),
                    text: format!("unrelated line {i}"),
                    steps: Vec::new(),
                    task_id: None,
                    outputs: Vec::new(),
                    parent: None,
                    mentions: Vec::new(),
                    mention_depth: 0,
                    audience: Vec::new(),
                },
            )
            .await
            .expect("journal");
    }
    for event in referral_leg_answering(
        "design",
        "Design",
        "product_designer",
        "engineering",
        "software_engineer",
        true,
        "error messages look like a copy task and are not one",
        Some(forward_seq),
    ) {
        runtime.events().append(&id, event).await.expect("journal");
    }
    runtime
        .events()
        .append(
            &id,
            CompanyEvent::AgentReply {
                chat_id: "engineering".to_string(),
                agent_id: "software_engineer".to_string(),
                text: "design came back: it is a design-system problem".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
                audience: Vec::new(),
            },
        )
        .await
        .expect("journal");

    // Only the tail is on screen, so the ask is far outside the scan.
    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        5,
        true,
    )
    .await
    .expect("history");
    let crossing = history
        .iter()
        .find_map(|m| m.referral_conversation.as_ref())
        .expect("the crossing rides the report");
    assert_eq!(
        crossing.lines.len(),
        2,
        "the pointer reaches an ask the scan cannot: {:?}",
        crossing.lines
    );
    assert!(crossing.lines[0].outbound);
    assert_eq!(
        crossing.lines[0].text,
        "what would you change about the error messages?"
    );
}

/// **A rendered relay shows the answer and none of the host's note.**
///
/// Seen in the console, not reasoned about: the asker's turn died on an
/// empty model response, the fallback rendered the relay, and #engineering
/// was told "you are the only one who has seen it" by the design desk's
/// agent. The note is written FOR the asker and is private to it; the
/// fallback exists to preserve the ANSWER, so that is all it may publish.
#[tokio::test]
async fn a_rendered_relay_keeps_the_answer_and_drops_the_note() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    let answer = "use a skeleton, not a spinner";
    let note = format!(
        "{}product_designer on the Design desk answered what you asked them. \
         This did not appear in your channel — you are the only one who has seen it.",
        crate::ports::types::RELAY_NOTE_MARKER
    );
    // No reply follows, so the fallback renders this relay.
    for event in referral_leg(
        "design",
        "Design",
        "product_designer",
        "engineering",
        "software_engineer",
        true,
        &format!("{answer}{note}"),
    ) {
        runtime.events().append(&id, event).await.expect("journal");
    }

    let history = history_for_desk(
        &runtime,
        "engineering",
        "engineering",
        &Viewer::Operator,
        None,
        50,
        true,
    )
    .await
    .expect("history");
    let relayed = history
        .iter()
        .find(|m| m.referred_from.is_some())
        .expect("the relay renders, because nothing else carries the answer");

    assert_eq!(
        relayed.text, answer,
        "the other desk's own words, and only those"
    );
    assert!(
        !relayed.text.contains("only one who has seen it"),
        "a note addressed to the asker is not published to the channel"
    );
}

/// **The fail-safe half: a relay renders while the report is still missing.**
///
/// The test below drops the relay once the asker has reported. Until then
/// there is nothing else carrying design's answer, and dropping it would
/// lose the answer outright — so it renders, in the wrong voice, saying
/// truthfully that it is an answer rather than an ask.
///
/// **Which leg this is, is the host's to say (the "Answered by" chip).**
///
/// Both legs are agent-authored lines on a desk, so every signal the
/// console holds reads identically on each — it guessed from `from` and
/// called every returning answer an ask. `tinyhivemind` decided it already
/// (`ReferralKind`), and the marker carries that decision.
#[tokio::test]
async fn a_relay_with_no_report_yet_still_renders_and_says_it_is_an_answer() {
    let home = tempfile::tempdir().expect("tempdir");
    let runtime = runtime(home.path()).await;
    let id = CompanyId::new("acme");

    for event in referral_leg(
        "engineering",
        "Engineering",
        "software_engineer",
        "design",
        "product_designer",
        false,
        "what would you change about the error messages?",
    )
    .into_iter()
    .chain(referral_leg(
        "design",
        "Design",
        "product_designer",
        "engineering",
        "software_engineer",
        true,
        "error messages look like a copy task and are not one",
    )) {
        runtime.events().append(&id, event).await.expect("journal");
    }

    for (desk, desk_name, returning) in [
        ("design", "Engineering", false),
        ("engineering", "Design", true),
    ] {
        let history = history_for_desk(&runtime, desk, desk, &Viewer::Operator, None, 50, true)
            .await
            .expect("history");
        let origin = history
            .iter()
            .find_map(|m| m.referred_from.as_ref())
            .unwrap_or_else(|| panic!("#{desk} carries a referral origin"));
        assert_eq!(origin.desk_name, desk_name, "on #{desk}");
        assert_eq!(
            origin.returning,
            returning,
            "#{desk} draws the {} chip",
            if returning {
                "\"Answered by\""
            } else {
                "\"Asked by\""
            }
        );
    }
}
