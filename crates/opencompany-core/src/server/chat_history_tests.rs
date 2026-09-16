use super::{AsideConversation, MessageView, aside_body, fold_asides};

/// A desk-visible row by `author`, or an aside when `to` names somebody.
fn row(id: &str, author: &str, text: &str, to: &[&str]) -> MessageView {
    MessageView {
        id: id.to_owned(),
        channel: author.to_owned(),
        admin_only: false,
        cue_author: author.to_owned(),
        author: author.to_owned(),
        cue_text: text.to_owned(),
        text: text.to_owned(),
        at_millis: 0.0,
        mine: false,
        by_person: false,
        referred_from: None,
        referral_conversation: None,
        aside_audience: to.iter().map(|id| (*id).to_owned()).collect(),
        aside_conversation: None,
        steps: Vec::new(),
        task_id: None,
        parent_id: None,
        reactions: Vec::new(),
        mentions: Vec::new(),
        attachments: Vec::new(),
        outputs: Vec::new(),
        resolution_user_facing: false,
        resolution_code: None,
        resolution_pair_agent_id: None,
        resolution_provider_slug: None,
    }
}

#[test]
fn an_aside_folds_onto_the_move_it_rode_under() {
    let mut messages = vec![
        row("1", "exchanges", "!propose #swap the clicky variant", &[]),
        row(
            "2",
            "exchanges",
            "!aside @refunds the difference is -$16.63",
            &["refunds"],
        ),
        row("3", "refunds", "!support #swap ^1", &[]),
    ];
    fold_asides(&mut messages);

    // The aside is lifted out of the transcript...
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["1", "3"],
        "the aside row no longer stands in the desk's own conversation"
    );
    // ...and hangs on its author's move, with the marker head stripped.
    let Some(AsideConversation { members, lines }) = &messages[0].aside_conversation else {
        panic!("the move carries the aside");
    };
    assert_eq!(members, &["exchanges".to_owned(), "refunds".to_owned()]);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].text, "the difference is -$16.63");
    assert!(
        !lines[0].text.contains("!aside"),
        "the grammar never reaches the operator's view"
    );
}

#[test]
fn an_orphan_aside_is_kept_rather_than_dropped() {
    // No move above it — a page that begins mid-exchange. A line in the wrong
    // shape beats a line nobody can read.
    let mut messages = vec![row(
        "1",
        "exchanges",
        "!aside @refunds mid-page",
        &["refunds"],
    )];
    fold_asides(&mut messages);
    assert_eq!(messages.len(), 1, "the row survives");
    assert!(messages[0].aside_conversation.is_none());
}

#[test]
fn an_aside_never_hangs_on_another_seats_move() {
    let mut messages = vec![
        row("1", "refunds", "!propose #refund take the return", &[]),
        row(
            "2",
            "exchanges",
            "!aside @refunds are you sure?",
            &["refunds"],
        ),
    ];
    fold_asides(&mut messages);
    // `exchanges` has no move above it, so its aside stays put rather than
    // being attributed to the seat that happened to speak last.
    assert_eq!(messages.len(), 2);
    assert!(messages[0].aside_conversation.is_none());
}

#[test]
fn aside_body_strips_every_addressee_and_leaves_other_text_alone() {
    assert_eq!(aside_body("!aside @a @b the point"), "the point");
    assert_eq!(aside_body("  !aside   @a   spaced  "), "spaced");
    // Not an aside: untouched, including a marker this host does not police.
    assert_eq!(aside_body("!propose #x y"), "!propose #x y");
    assert_eq!(aside_body("plain prose"), "plain prose");
}

use super::*;
use crate::ports::tasks::{
    COLUMN_DONE, COLUMN_IN_PROGRESS, COLUMN_IN_REVIEW, COLUMN_PAUSED, COLUMN_PLANNING,
    COLUMN_TODO,
};
use crate::ports::types::Actor;

fn agent_reply(chat_id: &str) -> CompanyEvent {
    CompanyEvent::AgentReply {
        audience: Vec::new(),
        mentions: Vec::new(),
        mention_depth: 0,
        parent: None,
        task_id: None,
        outputs: Vec::new(),
        chat_id: chat_id.to_string(),
        agent_id: "ceo".to_string(),
        text: "hi".to_string(),
        steps: Vec::new(),
    }
}

/// `None` is the shape the chat route stores for an unaddressed post.
fn operator_message(chat: Option<&str>) -> CompanyEvent {
    CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".to_string(),
        by: None,
        chat: chat.map(str::to_string),
        deliverable: None,
        attachments: Vec::new(),
    }
}

/// The whole difference between the two predicates, in one place.
///
/// `same_conversation` folds a missing id into General because an
/// unaddressed *message* went to the company-wide line.
/// `stamped_conversation_is` refuses to, because a missing id *stamped on a
/// record* means no conversation produced it — and handing those to General
/// is what let a thread-less parked blocker eat a founder's first line in
/// `#general` (B-059).
#[test]
fn a_stamped_origin_of_none_names_no_conversation_including_general() {
    for desk in [GENERAL_DESK, MAIN_THREAD_ID, "general", ""] {
        assert!(
            same_conversation(None, Some(desk)),
            "an unaddressed message still folds into General ({desk:?})"
        );
        assert!(
            !stamped_conversation_is(None, desk),
            "but a record stamped with no conversation belongs to none, {desk:?} included"
        );
    }
    assert!(!stamped_conversation_is(None, "engineering"));
}

/// Everything that *is* stamped compares exactly as `same_conversation`
/// does, so the carve-out cannot quietly become "refuse everything".
#[test]
fn a_stamped_origin_folds_general_and_compares_every_other_desk_verbatim() {
    for origin in [GENERAL_DESK, MAIN_THREAD_ID, "general", ""] {
        for desk in [GENERAL_DESK, MAIN_THREAD_ID, "general", ""] {
            assert!(
                stamped_conversation_is(Some(origin), desk),
                "every spelling of General is one conversation: {origin:?} vs {desk:?}"
            );
        }
    }
    assert!(stamped_conversation_is(Some("dm:eng"), "dm:eng"));
    assert!(!stamped_conversation_is(Some("dm:eng"), "dm:ops"));
    assert!(!stamped_conversation_is(Some("dm:eng"), GENERAL_DESK));
    assert!(
        !stamped_conversation_is(Some("Engineering"), "engineering"),
        "a desk id is opaque — the General fold is not a licence to loosen the rest"
    );
}

#[test]
fn general_desk_owns_agent_replies_under_general_and_main() {
    assert!(owns(GENERAL_DESK, GENERAL_DESK, &agent_reply(GENERAL_DESK)));
    assert!(owns(
        GENERAL_DESK,
        GENERAL_DESK,
        &agent_reply(MAIN_THREAD_ID)
    ));
    assert!(owns(GENERAL_DESK, GENERAL_DESK, &agent_reply("")));
    assert!(!owns(GENERAL_DESK, GENERAL_DESK, &agent_reply("strategy")));
}

/// The console asks for its default line as `?desk=main`, which resolves to
/// `("main", "main")` — no group chat is named `main` — so the desk side has
/// to fold too (issue #435).
///
/// The pair that made this reachable: an unaddressed chat post journals the
/// operator message with `chat: None` and its answer with
/// `chat_id: "General"`, so before this both halves of that conversation were
/// missing from the one transcript that should hold them.
#[test]
fn the_main_line_owns_what_was_journaled_under_general() {
    for stored in [GENERAL_DESK, MAIN_THREAD_ID, ""] {
        assert!(
            owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &agent_reply(stored)),
            "a reply stored as `{stored}` belongs to the main line",
        );
        assert!(
            owns(
                MAIN_THREAD_ID,
                MAIN_THREAD_ID,
                &operator_message(Some(stored))
            ),
            "an operator message stored as `{stored}` belongs to the main line",
        );
    }
    // The unaddressed post itself — the case that produces the pair above.
    assert!(owns(
        MAIN_THREAD_ID,
        MAIN_THREAD_ID,
        &operator_message(None)
    ));

    // …and the fold stops at the General family: a named desk's traffic
    // does not join the main line, in either direction.
    assert!(!owns(
        MAIN_THREAD_ID,
        MAIN_THREAD_ID,
        &agent_reply("strategy")
    ));
    assert!(!owns(
        "strategy",
        "Strategy desk",
        &operator_message(Some(GENERAL_DESK))
    ));
}

#[test]
fn non_general_desk_only_owns_its_own_id_or_name() {
    assert!(owns("strategy", "Strategy desk", &agent_reply("strategy")));
    assert!(owns(
        "strategy",
        "Strategy desk",
        &agent_reply("Strategy desk")
    ));
    assert!(!owns(
        "strategy",
        "Strategy desk",
        &agent_reply(MAIN_THREAD_ID)
    ));
    assert!(!owns("strategy", "Strategy desk", &agent_reply("")));
}

#[test]
fn general_desk_owns_every_operator_message() {
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".to_string(),
        by: Some(Actor {
            kind: ActorKind::User,
            id: "u1".to_string(),
        }),
        chat: Some(MAIN_THREAD_ID.to_string()),
        deliverable: None,
        attachments: Vec::new(),
    };
    assert!(owns(GENERAL_DESK, GENERAL_DESK, &event));
    assert!(!owns("strategy", "Strategy desk", &event));
}

// Regression: issue — operator messages vanished on reload because the read
// filter ignored the stored chat id.
#[test]
fn main_thread_owns_operator_messages_it_stored() {
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".to_string(),
        by: None,
        chat: Some(MAIN_THREAD_ID.to_string()),
        deliverable: None,
        attachments: Vec::new(),
    };
    // The console queries the main thread with desk = ("main", "main").
    assert!(owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &event));
    // And it is still owned when read under the General desk's own id/name.
    assert!(owns(GENERAL_DESK, GENERAL_DESK, &event));
    // But it must not leak into an unrelated desk.
    assert!(!owns("strategy", "Strategy desk", &event));
}

#[test]
fn desk_addressed_operator_message_belongs_to_that_desk() {
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".to_string(),
        by: None,
        chat: Some("strategy".to_string()),
        deliverable: None,
        attachments: Vec::new(),
    };
    assert!(owns("strategy", "Strategy desk", &event));
    assert!(!owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &event));
}

/* ---- issue #364: threads and reactions ---- */

fn user(id: &str) -> Option<Actor> {
    Some(Actor {
        kind: ActorKind::User,
        id: id.to_string(),
    })
}

fn at(seq: u64, event: CompanyEvent) -> StoredEvent {
    StoredEvent {
        seq: EventSeq::new(seq),
        company: crate::ports::types::CompanyId::new("acme"),
        event,
        at_millis: 1_700_000_000_000 + seq,
    }
}

fn reaction(seq: u64, message: u64, emoji: &str, on: bool, by: Option<Actor>) -> StoredEvent {
    at(
        seq,
        CompanyEvent::ReactionToggled {
            message_seq: EventSeq::new(message),
            emoji: emoji.to_string(),
            on,
            by,
        },
    )
}

fn labels() -> HashMap<String, String> {
    HashMap::from([
        ("u1".to_string(), "Ada".to_string()),
        ("u2".to_string(), "Grace".to_string()),
    ])
}

/// Two people reacting with the same emoji are two rows, not a count of
/// two, and only the reader's own row is `mine` — which is the whole reason
/// the durable record is per-person.
#[test]
fn reactions_fold_into_one_row_per_person() {
    let log = vec![
        reaction(10, 4, "👍", true, user("u1")),
        reaction(11, 4, "👍", true, user("u2")),
    ];
    let folded = fold_reactions(&log, &Viewer::User("u1".to_string()), &labels());
    let rows = folded.get("4").expect("message 4 has reactions");
    assert_eq!(
        rows,
        &vec![
            ReactionView {
                emoji: "👍".to_string(),
                by_label: "Ada".to_string(),
                mine: true,
            },
            ReactionView {
                emoji: "👍".to_string(),
                by_label: "Grace".to_string(),
                mine: false,
            },
        ]
    );

    // The same log read by the other person flips only `mine`.
    let folded = fold_reactions(&log, &Viewer::User("u2".to_string()), &labels());
    let mine: Vec<bool> = folded["4"].iter().map(|r| r.mine).collect();
    assert_eq!(mine, vec![false, true]);
}

/// The last event per (message, person, emoji) wins, so a clear removes the
/// row and a repeated set leaves exactly one — which is what makes the
/// route's explicit `on` flag idempotent rather than a toggle that drifts.
#[test]
fn reactions_fold_to_the_last_event_per_person_and_emoji() {
    let log = vec![
        reaction(10, 4, "👍", true, user("u1")),
        reaction(11, 4, "👍", true, user("u1")),
        reaction(12, 4, "🎉", true, user("u1")),
        reaction(13, 4, "🎉", false, user("u1")),
    ];
    let folded = fold_reactions(&log, &Viewer::User("u1".to_string()), &labels());
    let emojis: Vec<&str> = folded["4"].iter().map(|r| r.emoji.as_str()).collect();
    assert_eq!(emojis, vec!["👍"], "a cleared reaction leaves no row");
}

/// A reaction made with a machine credential reads back as the operator's,
/// exactly as an unattributed message does — the same collapse `project`
/// makes for authorship, so the two surfaces cannot disagree about who a
/// credential is.
#[test]
fn an_unattributed_reaction_belongs_to_the_operator() {
    let log = vec![reaction(10, 4, "👀", true, None)];
    let folded = fold_reactions(&log, &Viewer::Operator, &labels());
    assert_eq!(folded["4"][0].by_label, "operator");
    assert!(folded["4"][0].mine);
    // …and is nobody's own when a signed-in person reads it.
    let folded = fold_reactions(&log, &Viewer::User("u1".to_string()), &labels());
    assert!(!folded["4"][0].mine);
}

fn mention(target: MentionTarget, text: &str, offset: usize) -> Mention {
    Mention {
        target,
        text: text.to_string(),
        offset,
        quiet: false,
    }
}

fn message_mentioning(mentions: Vec<Mention>) -> CompanyEvent {
    CompanyEvent::OperatorMessage {
        mentions,
        parent: None,
        text: "ping".to_string(),
        by: None,
        chat: Some("studio".to_string()),
        deliverable: None,
        attachments: Vec::new(),
    }
}

/// Who *typed* a line is a fact only the host still holds (issue #1734).
///
/// Every downstream shortcut for it is wrong, and the two obvious ones are
/// wrong in ways that look right:
///
/// * `mine` is per-viewer, so a colleague's own message is `mine: false`
///   and lands on the company side of their reader's transcript, beside the
///   agent replies.
/// * `channel == "operator"` collides head-on. The offline echo brain names
///   its own outbound channel `operator` (`brain::echo`), exactly as this
///   arm does, so a journaled echo reply and a human's message carry the
///   same label. A console that split on it marked neither, which suppressed
///   the marker on precisely the replies it exists for — caught in a browser
///   against a live host, not by a unit test.
///
/// So the projection says it, and this test pins both directions with the
/// echo brain's own channel label in play, because that is the collision.
#[test]
fn only_a_persons_message_is_projected_as_by_person() {
    let typed = MessageView::project(
        at(
            1,
            CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: None,
                text: "on it".to_string(),
                by: Some(Actor {
                    kind: ActorKind::User,
                    id: "u1".to_string(),
                }),
                chat: Some("studio".to_string()),
                deliverable: None,
                attachments: Vec::new(),
            },
        ),
        // Projected for *another* reader, which is the case that matters:
        // for them this is `mine: false` and nothing else distinguishes it.
        &Viewer::User("u2".to_string()),
        &labels(),
    );
    assert!(typed.by_person, "a person typed this");
    assert!(!typed.mine, "and it is not this reader's own line");

    // The echo brain's reply as the runtime journals it: an `AgentReply`
    // whose agent id is the outbound channel the brain named — `operator`,
    // the very label the arm above hardcodes.
    let echoed = MessageView::project(
        at(
            2,
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: None,
                task_id: None,
                outputs: Vec::new(),
                chat_id: "studio".to_string(),
                agent_id: "operator".to_string(),
                text: "You said: on it".to_string(),
                steps: Vec::new(),
            },
        ),
        &Viewer::User("u2".to_string()),
        &labels(),
    );
    assert!(!echoed.by_person, "no person typed the echo brain's reply");
    assert_eq!(
        echoed.channel, typed.channel,
        "the collision is real: the channel label cannot tell these apart",
    );
}

/// A person's mention reaches a reader as a **label**, never as the user id
/// it is stored under — the same rule `by_label` follows for reactions.
#[test]
fn project_resolves_a_person_to_a_label_and_never_to_an_id() {
    let view = MessageView::project(
        at(
            7,
            message_mentioning(vec![mention(
                MentionTarget::User {
                    id: "u1".to_string(),
                },
                "@Ada",
                0,
            )]),
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(view.mentions.len(), 1);
    assert_eq!(view.mentions[0].label, "Ada");
    assert_eq!(view.mentions[0].text, "@Ada");
    assert_eq!(view.mentions[0].offset, 0);
    assert!(
        !view.mentions[0].label.contains("u1"),
        "the stored id must not reach a reader"
    );
}

/// `mine` is per viewer: the same stored row is the reader's own mention
/// for one person and somebody else's for everyone else.
#[test]
fn project_decides_mine_per_viewer() {
    let event = at(
        8,
        message_mentioning(vec![mention(
            MentionTarget::User {
                id: "u1".to_string(),
            },
            "@Ada",
            0,
        )]),
    );
    let ada = MessageView::project(event.clone(), &Viewer::User("u1".to_string()), &labels());
    assert!(ada.mentions[0].mine);

    let grace = MessageView::project(event, &Viewer::User("u2".to_string()), &labels());
    assert!(!grace.mentions[0].mine);
}

/// A broadcast is addressed to whoever is reading, so it is everybody's own
/// mention — that is what makes it badge every recipient.
#[test]
fn everyone_is_mine_for_every_reader() {
    let event = at(
        9,
        message_mentioning(vec![mention(MentionTarget::Everyone, "@everyone", 0)]),
    );
    for viewer in [
        Viewer::Operator,
        Viewer::User("u1".to_string()),
        Viewer::User("u2".to_string()),
    ] {
        let view = MessageView::project(event.clone(), &viewer, &labels());
        assert!(view.mentions[0].mine, "viewer: {viewer:?}");
        assert_eq!(view.mentions[0].label, "everyone");
    }
}

/// A person who has since been removed has no label to resolve to. The
/// literal text the author typed is the honest fallback — it is what a
/// reader would have seen anyway — and it must not be the raw id.
#[test]
fn a_mention_of_a_departed_person_falls_back_to_the_typed_text() {
    let view = MessageView::project(
        at(
            10,
            message_mentioning(vec![mention(
                MentionTarget::User {
                    id: "gone".to_string(),
                },
                "@Bob",
                0,
            )]),
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(view.mentions[0].label, "Bob");
}

#[test]
fn a_teammate_and_a_desk_project_their_ids_as_labels() {
    let view = MessageView::project(
        at(
            11,
            message_mentioning(vec![
                mention(
                    MentionTarget::Agent {
                        id: "engineer".to_string(),
                    },
                    "@engineer",
                    0,
                ),
                mention(
                    MentionTarget::Desk {
                        id: "engineering".to_string(),
                    },
                    "@engineering",
                    10,
                ),
            ]),
        ),
        &Viewer::Operator,
        &labels(),
    );
    let labels: Vec<&str> = view.mentions.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(labels, vec!["engineer", "engineering"]);
    assert!(
        view.mentions.iter().all(|m| !m.mine),
        "a teammate or a desk is never the human reader"
    );
}

#[test]
fn a_quiet_mention_projects_as_quiet() {
    let view = MessageView::project(
        at(
            12,
            message_mentioning(vec![Mention {
                quiet: true,
                ..mention(
                    MentionTarget::User {
                        id: "u1".to_string(),
                    },
                    "@Ada",
                    0,
                )
            }]),
        ),
        &Viewer::User("u1".to_string()),
        &labels(),
    );
    assert!(view.mentions[0].quiet);
}

#[test]
fn a_message_that_mentions_nobody_projects_an_empty_list() {
    let view = MessageView::project(
        at(13, message_mentioning(Vec::new())),
        &Viewer::Operator,
        &labels(),
    );
    assert!(view.mentions.is_empty());
}

/// A thread parent survives projection on both halves of an exchange, as
/// the message id a reader can resolve rather than a raw sequence number.
#[test]
fn project_carries_the_thread_parent() {
    let operator = MessageView::project(
        at(
            12,
            CompanyEvent::OperatorMessage {
                mentions: Vec::new(),
                parent: Some(EventSeq::new(4)),
                text: "a follow-up".to_string(),
                by: None,
                chat: Some("studio".to_string()),
                deliverable: None,
                attachments: Vec::new(),
            },
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(operator.parent_id.as_deref(), Some("4"));

    let reply = MessageView::project(
        at(
            13,
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                parent: Some(EventSeq::new(4)),
                task_id: None,
                outputs: Vec::new(),
                chat_id: "studio".to_string(),
                agent_id: "ceo".to_string(),
                text: "on it".to_string(),
                steps: Vec::new(),
            },
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(reply.parent_id.as_deref(), Some("4"));

    // A message with no parent is in the channel, not in a thread — which
    // is every message journaled before threads were persisted.
    let plain =
        MessageView::project(at(14, agent_reply("studio")), &Viewer::Operator, &labels());
    assert!(plain.parent_id.is_none());
}

#[test]
fn legacy_operator_message_without_chat_stays_on_general() {
    let event = CompanyEvent::OperatorMessage {
        mentions: Vec::new(),
        parent: None,
        text: "hi".to_string(),
        by: None,
        chat: None,
        deliverable: None,
        attachments: Vec::new(),
    };
    assert!(owns(GENERAL_DESK, GENERAL_DESK, &event));
    assert!(!owns("strategy", "Strategy desk", &event));
}

/* ---- issue #377: the dispatch terminal as a channel marker ---- */

/// A settled dispatch, as the harness journals it. `desk` is deliberately
/// an agent id (`engineer`) and never a channel id (`engineering`) — that
/// difference is the whole reason the origin has to be carried.
fn desk_task_completed(origin: Option<&str>, column: &str) -> CompanyEvent {
    threaded_desk_task_completed(origin, None, column)
}

/// The same settle, for a card raised inside a thread (#1890 B).
fn threaded_desk_task_completed(
    origin: Option<&str>,
    origin_parent: Option<u64>,
    column: &str,
) -> CompanyEvent {
    CompanyEvent::DeskTaskCompleted {
        task_id: "t-1".to_string(),
        desk: "engineer".to_string(),
        output: "the run's prose".to_string(),
        column: column.to_string(),
        artifact_ids: Vec::new(),
        origin_chat_id: origin.map(str::to_string),
        origin_parent: origin_parent.map(EventSeq::new),
    }
}

/// The terminal routes by the origin the card recorded, on exactly the same
/// terms a reply does: the desk's id or its name, and nothing else.
#[test]
fn a_terminal_belongs_to_the_channel_its_card_was_raised_in() {
    let event = desk_task_completed(Some("engineering"), COLUMN_IN_REVIEW);
    assert!(owns("engineering", "Engineering desk", &event));
    // …and by the desk's *name*, for a card whose origin was journaled
    // under it — the same either-spelling rule a reply routes by.
    let by_name = desk_task_completed(Some("Engineering desk"), COLUMN_IN_REVIEW);
    assert!(owns("engineering", "Engineering desk", &by_name));
    // …and nowhere else. A settle in one channel must not surface in
    // another, which is what would make the marker worse than no marker.
    assert!(!owns("strategy", "Strategy desk", &event));
    assert!(!owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &event));
    // The responder is not the channel — matching on it would file every
    // settle under a desk whose id happens to equal an agent's.
    assert!(!owns("engineer", "engineer", &event));
}

/// **The most bug-prone line in `owns`.** A card no conversation raised
/// belongs to no conversation's history — General emphatically included.
///
/// Everywhere else in this module a missing chat id means "unaddressed,
/// therefore General". On a terminal it means the opposite: the card was
/// created on the board, by a scheduler, or before the origin was recorded.
/// Folding it would post markers about board-only work into the operator's
/// main line, which is a *new* bug rather than the one #377 fixes.
#[test]
fn a_terminal_with_no_origin_belongs_to_nobody_not_to_general() {
    let event = desk_task_completed(None, COLUMN_IN_REVIEW);
    assert!(
        !owns(GENERAL_DESK, GENERAL_DESK, &event),
        "an origin-less terminal must not fold into the General desk",
    );
    assert!(
        !owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &event),
        "nor into the console's main line, which is General's other spelling",
    );
    assert!(!owns("", "", &event));
    assert!(!owns("engineering", "Engineering desk", &event));
}

/// A terminal whose origin *is* one of General's four spellings still folds
/// like every other event does — the exception above is about `None`, not
/// about loosening [`same_conversation`].
#[test]
fn a_terminal_raised_on_the_main_line_folds_like_any_other_event() {
    for origin in [GENERAL_DESK, MAIN_THREAD_ID, ""] {
        let event = desk_task_completed(Some(origin), COLUMN_PAUSED);
        assert!(
            owns(MAIN_THREAD_ID, MAIN_THREAD_ID, &event),
            "a terminal stored as `{origin}` belongs to the main line",
        );
        assert!(
            owns(GENERAL_DESK, GENERAL_DESK, &event),
            "…and to the General desk's own id/name",
        );
        assert!(
            !owns("strategy", "Strategy desk", &event),
            "…and to no named desk",
        );
    }
}

/// The marker's wording, pinned per column. The console holds the same
/// literals (`dispatchMarkerText`, `frontend/src/lib/chat.ts`) because the
/// live frame carries the raw column id; these two tests are what couple
/// them.
#[test]
fn the_marker_names_where_the_card_landed() {
    assert_eq!(
        dispatch_marker_text(COLUMN_IN_REVIEW),
        "finished → In review"
    );
    assert_eq!(dispatch_marker_text(COLUMN_PAUSED), "finished → Paused");
    assert_eq!(dispatch_marker_text(COLUMN_TODO), "finished → To-do");
    assert_eq!(dispatch_marker_text(COLUMN_DONE), "finished → Done");
    assert_eq!(dispatch_marker_text(COLUMN_PLANNING), "finished → Planning");
    assert_eq!(
        dispatch_marker_text(COLUMN_IN_PROGRESS),
        "finished → In progress"
    );
}

/// A column this build has not heard of reads a little raw rather than
/// rendering blank — the same fallback `relay_text` takes, and the reason a
/// newer host cannot produce an empty pill here.
#[test]
fn an_unknown_column_passes_through_verbatim() {
    assert_eq!(
        dispatch_marker_text("shipped_to_orbit"),
        "finished → shipped_to_orbit"
    );
}

/// The terminal projects as a system line carrying its card — not as the
/// `Debug` dump the defensive fallback would have rendered into a person's
/// transcript.
#[test]
fn project_renders_a_terminal_as_a_card_linked_system_marker() {
    let view = MessageView::project(
        at(
            21,
            desk_task_completed(Some("engineering"), COLUMN_IN_REVIEW),
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(view.author, "system");
    assert_eq!(view.channel, "system");
    assert_eq!(view.text, "finished → In review");
    assert_eq!(
        view.task_id.as_deref(),
        Some("t-1"),
        "the pill links the card"
    );
    assert!(!view.mine);
    assert!(view.steps.is_empty(), "a marker is not a turn");
    assert!(
        view.parent_id.is_none(),
        "a card raised at channel level settles flat in the channel",
    );
    assert_eq!(view.id, "21", "the host id the console dedupes a reload on");
}

/// Issue #1890 B — the whole of what this sub-issue repairs.
///
/// A card raised inside a thread used to settle flat in the channel, so the
/// thread that asked for the work never showed it finishing. The marker
/// carries the root now, in the same field and the same rendering an
/// operator message's parent takes, so the console files it into the thread
/// with no renderer change at all.
#[test]
fn a_terminal_raised_in_a_thread_projects_into_that_thread() {
    let view = MessageView::project(
        at(
            50,
            threaded_desk_task_completed(Some("engineering"), Some(41), COLUMN_IN_REVIEW),
        ),
        &Viewer::Operator,
        &labels(),
    );
    assert_eq!(
        view.parent_id.as_deref(),
        Some("41"),
        "the marker hangs off the root the card recorded",
    );
    // The channel half is unchanged: routing still runs through `owns` on
    // the origin channel, and the thread only narrows within it. A marker
    // that threaded but stopped belonging to its channel would vanish.
    assert!(owns(
        "engineering",
        "Engineering desk",
        &threaded_desk_task_completed(Some("engineering"), Some(41), COLUMN_IN_REVIEW),
    ));
}

/// The run's prose stays out of the marker. It already reaches this same
/// channel as the orchestrator's relay bubble (#151); repeating it here
/// would put one run's words into one conversation twice.
#[test]
fn the_marker_does_not_repeat_the_runs_prose() {
    let view = MessageView::project(
        at(22, desk_task_completed(Some("engineering"), COLUMN_PAUSED)),
        &Viewer::Operator,
        &labels(),
    );
    assert!(!view.text.contains("the run's prose"), "{}", view.text);
    assert_eq!(view.text, "finished → Paused");
}

/// Issue #885: the audit's classification rule.
///
/// The rule is "an `agent_id` naming no roster teammate", not
/// `== "operator"`, so these pin both the shape actually observed and the
/// generalisation — the same writer bug on another channel produces a
/// different wrong string and still has to be counted.
mod attribution_audit {
    use super::*;

    fn reply(seq: u64, agent_id: &str) -> StoredEvent {
        at(
            seq,
            CompanyEvent::AgentReply {
                audience: Vec::new(),
                mentions: Vec::new(),
                mention_depth: 0,
                chat_id: "engineering".to_string(),
                agent_id: agent_id.to_string(),
                text: "…".to_string(),
                steps: Vec::new(),
                task_id: None,
                outputs: Vec::new(),
                parent: None,
            },
        )
    }

    /// The roster for these: two real teammates and nothing else.
    ///
    /// Deliberately *excludes* the confined copilot, because that is the
    /// point of `is_known_author` — the copilot is a real author that no
    /// roster will ever resolve.
    fn on_roster(agent_id: &str) -> bool {
        matches!(agent_id, "engineer" | "product_manager")
    }

    /// A record whose roster is exactly `on_roster`'s two teammates.
    ///
    /// Built so the tests below call the **real** `is_known_author` rather
    /// than a local restatement of it. The first version of these tests
    /// re-implemented the predicate in the test module, which meant
    /// reverting the production function changed nothing and the tests
    /// passed either way — proving only that the test agreed with itself.
    fn record() -> CompanyRecord {
        let src = "[company]\nname = \"Acme\"\n\n[policy]\nmode = \"full\"\n\
                   \n[[agent]]\nid = \"engineer\"\nrole = \"Worker\"\ntier = \"orchestrator\"\n\
                   \n[[agent]]\nid = \"product_manager\"\nrole = \"Worker\"\ntier = \"orchestrator\"\n";
        let manifest: crate::company::CompanyManifest =
            toml::from_str(src).expect("manifest parses");
        CompanyRecord {
            overlay_desk_hive: Vec::new(),
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: crate::ports::types::CompanyId::new("acme"),
            manifest,
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_tool_grants: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
            setup: None,
            name_confirmed: false,
            activation_completed_at: None,
            created_at_millis: None,
        }
    }

    /// Issue #966. The runtime speaking for itself is a *correct* row, not
    /// damage. Counting it would inflate the blast-radius figure on a company
    /// doing nothing wrong, and would caption a legitimate system message as
    /// something nobody can attribute.
    #[test]
    fn a_host_authored_notice_is_a_known_author_not_an_affected_row() {
        let record = record();
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[reply(1, crate::ports::SYSTEM_AUTHOR), reply(2, "engineer")],
            |agent_id| is_known_author(agent_id, &record),
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(audit.affected, 0);
    }

    /// Issue #966. The console reaches the centred system pill by comparing
    /// the projected author against a literal `"system"`
    /// (`frontend/src/lib/chat.ts`), and `MessageView` projects an
    /// `AgentReply`'s `agent_id` straight into that field. So the *value* is
    /// the contract with the console, not merely the constant's identity.
    ///
    /// Redefining `SYSTEM_AUTHOR` to anything else keeps every other test
    /// here green and silently returns these three notices to rendering as
    /// company bubbles — the exact appearance this change exists to end.
    /// Two copies of one literal is the same coupling
    /// `dispatch_marker_text` already carries with that file, and it is
    /// deliberate for the same reason.
    #[test]
    fn the_notice_author_is_the_literal_the_console_keys_on() {
        assert_eq!(
            crate::ports::SYSTEM_AUTHOR,
            "system",
            "frontend/src/lib/chat.ts renders `author === \"system\"` as the centred pill"
        );
    }

    /// The whole point of the reserved id: a notice and a damaged reply used
    /// to be the same bytes. This pins that they are now different ones, so
    /// the distinction a marker would rely on actually exists in the data.
    #[test]
    fn a_notice_and_an_overwritten_reply_are_no_longer_the_same_author() {
        let record = record();
        assert_ne!(
            crate::ports::SYSTEM_AUTHOR,
            "operator",
            "a notice must not share the author a destination-overwrite produces"
        );
        assert!(is_known_author(crate::ports::SYSTEM_AUTHOR, &record));
        assert!(!is_known_author("operator", &record));
    }

    /// Issue #966. A copilot turn genuinely authored its reply, so the id it
    /// stores is a truthful author — not a destination that leaked into the
    /// field. Counting it would swap one wrong answer for a permanent false
    /// positive that climbs on a company doing nothing wrong.
    #[test]
    fn the_confined_copilot_is_a_known_author_not_an_affected_row() {
        let record = record();
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                reply(1, crate::ports::CONFINED_AGENT_ID),
                reply(2, "engineer"),
            ],
            |agent_id| is_known_author(agent_id, &record),
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(audit.affected, 0);
    }

    /// …and it is still not on the roster, which is what makes the widening
    /// necessary rather than incidental. If `resolve_roster_agent_id` ever
    /// started answering for it, this says so before the extra arm quietly
    /// becomes dead code.
    #[test]
    fn the_confined_copilot_is_not_reachable_through_the_roster_alone() {
        let record = record();
        assert!(
            record
                .resolve_roster_agent_id(crate::ports::CONFINED_AGENT_ID)
                .is_none(),
            "the confined id resolved on the roster; `is_known_author`'s extra arm is now \
             unnecessary and this test should be deleted deliberately, not left passing"
        );
        assert!(is_known_author(crate::ports::CONFINED_AGENT_ID, &record));
        assert!(is_known_author("engineer", &record));
        assert!(!is_known_author("operator", &record));
    }

    /// A delivered workflow report is journaled under
    /// [`crate::runtime::WORKFLOW_REPLY_AUTHOR`] on purpose — it is the
    /// workflow speaking, not a teammate's own reply. Counting it would
    /// flag every delivered report on a company with no roster match for
    /// "workflow" as damaged, and — worse — a teammate who *did* mint that
    /// id would have every report silently misattributed to them by
    /// `senderOf` before this reservation existed.
    #[test]
    fn a_workflow_report_is_a_known_author_not_an_affected_row() {
        let record = record();
        assert!(
            record
                .resolve_roster_agent_id(crate::runtime::WORKFLOW_REPLY_AUTHOR)
                .is_none(),
            "workflow reports resolve through the extra arm, not the roster"
        );
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                reply(1, crate::runtime::WORKFLOW_REPLY_AUTHOR),
                reply(2, "engineer"),
            ],
            |agent_id| is_known_author(agent_id, &record),
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(audit.affected, 0);
    }

    /// Issue #1781 review, Codex P2: an owner-fallback report is journaled
    /// under [`crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR`] on purpose —
    /// same reservation as `WORKFLOW_REPLY_AUTHOR`, one arm narrower — so it
    /// must not inflate the audit either. Before this arm existed, every
    /// legitimate no-mailbox fallback counted as damaged attribution.
    #[test]
    fn an_owner_fallback_report_is_a_known_author_not_an_affected_row() {
        let record = record();
        assert!(
            record
                .resolve_roster_agent_id(crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR)
                .is_none(),
            "owner-fallback reports resolve through the extra arm, not the roster"
        );
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                reply(1, crate::runtime::OWNER_FALLBACK_REPORT_AUTHOR),
                reply(2, "engineer"),
            ],
            |agent_id| is_known_author(agent_id, &record),
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(audit.affected, 0);
    }

    /// Review on PR #1781 (Codex P2): a company that named an overlay
    /// teammate "Workflow" before this reservation existed would have
    /// minted the bare id `workflow` — the id `WORKFLOW_REPLY_AUTHOR`
    /// itself used to be, until it was reshaped to the unmintable,
    /// hyphenated `workflow-report`. That persisted teammate is not
    /// migrated or renamed by this fix — there is nothing to migrate: the
    /// pseudo-author a workflow report is now journaled under is a
    /// **different, disjoint id** from the one that teammate holds, so
    /// the collision this reservation exists to prevent cannot occur for
    /// it, retroactively as well as going forward. Proven here rather than
    /// asserted, since the whole point is that the two ids must never
    /// again be able to resolve to the same author.
    #[test]
    fn a_persisted_teammate_named_workflow_does_not_shadow_the_reply_author() {
        let mut record = record();
        record
            .overlay_agents
            .push(crate::ports::types::OverlayAgent {
                provider: None,
                id: "workflow".to_string(),
                name: "Workflow".to_string(),
                role: "Worker".to_string(),
                description: None,
                tools: Some(Vec::new()),
                model: None,
                harness: None,
            });

        assert_ne!(
            "workflow",
            crate::runtime::WORKFLOW_REPLY_AUTHOR,
            "the two ids must be disjoint for the rest of this test to mean anything"
        );
        assert!(
            record.resolve_roster_agent_id("workflow").is_some(),
            "the pre-existing teammate is still on the roster, unmigrated"
        );
        assert!(
            record
                .resolve_roster_agent_id(crate::runtime::WORKFLOW_REPLY_AUTHOR)
                .is_none(),
            "the reply-author id does not resolve to that (or any) teammate"
        );

        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                // The teammate's own reply — attributed to them, as before.
                reply(1, "workflow"),
                // A new workflow report, delivered after this fix ships —
                // journaled under the disjoint id, not theirs.
                reply(2, crate::runtime::WORKFLOW_REPLY_AUTHOR),
            ],
            |agent_id| is_known_author(agent_id, &record),
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(
            audit.affected, 0,
            "both rows resolve, to two different authors"
        );
    }

    #[test]
    fn a_reply_authored_by_a_real_teammate_is_not_counted() {
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[reply(1, "engineer"), reply(2, "product_manager")],
            on_roster,
        );
        assert_eq!(audit.replies, 2);
        assert_eq!(audit.affected, 0);
        assert!(audit.by_agent_id.is_empty());
    }

    /// The observed #885 shape: the operator channel copied into the author.
    #[test]
    fn a_reply_authored_by_the_operator_channel_is_counted() {
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                reply(1, "operator"),
                reply(2, "engineer"),
                reply(3, "operator"),
            ],
            on_roster,
        );
        assert_eq!(audit.replies, 3);
        assert_eq!(audit.affected, 2);
        assert_eq!(audit.by_agent_id.get("operator"), Some(&2));
    }

    /// The generalisation. A Telegram chat id or a desk slug in the author
    /// field is the same defect, and a rule keyed on the literal
    /// `"operator"` would report a clean company.
    #[test]
    fn any_non_roster_author_is_counted_not_just_the_operator_channel() {
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[reply(1, "operator"), reply(2, "-100123456789")],
            on_roster,
        );
        assert_eq!(audit.affected, 2);
        assert_eq!(audit.by_agent_id.get("-100123456789"), Some(&1));
    }

    /// Only replies. An operator's own message is not an `AgentReply` and
    /// has no `agent_id` to be wrong, so counting it would inflate the
    /// blast radius of a data-integrity bug — the one number that has to be
    /// trustworthy here.
    #[test]
    fn a_non_reply_event_is_neither_scanned_nor_counted() {
        let mut audit = AttributionAudit::default();
        audit.fold(
            &[
                at(
                    1,
                    CompanyEvent::OperatorMessage {
                        mentions: Vec::new(),
                        text: "hello".to_string(),
                        by: None,
                        chat: None,
                        parent: None,
                        deliverable: None,
                        attachments: Vec::new(),
                    },
                ),
                reply(2, "operator"),
            ],
            on_roster,
        );
        assert_eq!(audit.replies, 1);
        assert_eq!(audit.affected, 1);
    }
}
