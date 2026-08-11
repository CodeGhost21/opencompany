//! Shared desk-history read logic (issue #65).
//!
//! Both the GraphQL `Chat.history` resolver
//! ([`crate::server::graphql::company`]) and the REST `GET .../chat/history`
//! route ([`crate::server::operator`]) need to answer the same question — "what
//! messages belong to this desk, as seen by this viewer?" — and they must never
//! be allowed to disagree about it. This module is the one place that answers
//! it; both surfaces call through it instead of each keeping their own copy of
//! the filter + projection logic.

use std::collections::HashMap;

use crate::company::runtime::CompanyRuntime;
use crate::error::OpenCompanyError;
use crate::ports::types::{Actor, ActorKind, CompanyEvent, EventSeq, StoredEvent, TurnStep};
use crate::server::ops::language::DEFAULT_DESK as GENERAL_DESK;

/// The console's default/orchestrator thread id
/// (`frontend/src/lib/threads.ts` `mainThread()`). The console addresses every
/// send on that thread with `chat: "main"`, so `AgentReply`s answering it are
/// journaled with `chat_id == "main"` rather than [`GENERAL_DESK`]. `owns`
/// admits both spellings for the General desk so a transcript is never split
/// across the two ids depending on which one happened to write it (issue #65).
pub const MAIN_THREAD_ID: &str = "main";

/// Does this stored chat id mean the General desk?
///
/// **Four spellings, one desk.** The console addresses its default thread as
/// `"main"`, the chat route stores an unaddressed message as `None`, older
/// events carry `""`, and the desk's own id/name is `"General"`. [`owns`] has
/// admitted all four since issue #65, which is what stops a transcript from
/// splitting across whichever id happened to write each message.
///
/// Exposed because that equivalence is **not** local to history rendering.
/// `CompanyRuntime::resolvable_parent` compares a remembered thread root's chat
/// id against the channel being answered into, and comparing the raw strings
/// there made a root stored as `None` fail to match the `"General"` it is
/// rendered under — so a threaded approval rooted in an unaddressed message
/// silently resumed in the channel, which is the exact symptom issue #435 set
/// out to remove. Two places deciding "same conversation?" by different rules
/// is the drift; one function is the fix. See [`same_conversation`].
pub fn is_general_chat(chat: Option<&str>) -> bool {
    match chat {
        None => true,
        Some(chat) => {
            chat.is_empty()
                || chat.eq_ignore_ascii_case(MAIN_THREAD_ID)
                || chat.eq_ignore_ascii_case(GENERAL_DESK)
        }
    }
}

/// Do two stored chat ids name the same conversation (issue #435)?
///
/// Every spelling of the General desk is one conversation — see
/// [`is_general_chat`] — and everything else compares verbatim, because a desk
/// id is an opaque identifier and two desks differing only in case are two
/// desks. Deliberately **not** a general-purpose case-insensitive compare: the
/// folding is a fact about one desk's history, not a licence to loosen the
/// others.
pub fn same_conversation(a: Option<&str>, b: Option<&str>) -> bool {
    if is_general_chat(a) || is_general_chat(b) {
        return is_general_chat(a) && is_general_chat(b);
    }
    a == b
}

/// Whether a stored event belongs to the desk identified by `desk_id` /
/// `desk_name`.
///
/// Both `AgentReply`s and `OperatorMessage`s route by their stored chat id,
/// matched against the desk's id and its name through [`same_conversation`] — so
/// a named desk still compares verbatim and the General desk answers to every
/// spelling of itself, and no historical message is orphaned by the id it
/// happened to be journaled under (issue #65).
///
/// **Folded on both sides, not just the event's** (issue #435). The General
/// check used to key on the *desk being asked for* being spelled `"General"`,
/// which the console never does: its default thread is `"main"`, so
/// `?desk=main` resolves to `("main", "main")` — no group chat is named `main` —
/// and every event journaled under `"General"` was excluded from the one
/// transcript that should hold them. An unaddressed chat post is exactly that
/// pair: the operator message stores `chat: None` and its answer is journaled
/// with `chat_id: "General"`, so the console's main line dropped both halves of
/// its own conversation. The asymmetry also put this function at odds with
/// `resolvable_parent`, which now folds through the same rule: a continuation
/// could be parented to a root the main line refuses to render, and the console
/// drops a reply whose parent it cannot find rather than showing it flat.
pub fn owns(desk_id: &str, desk_name: &str, event: &CompanyEvent) -> bool {
    let stored = match event {
        CompanyEvent::AgentReply { chat_id, .. } => Some(chat_id.as_str()),
        CompanyEvent::OperatorMessage { chat, .. } => chat.as_deref(),
        _ => return false,
    };
    same_conversation(stored, Some(desk_id)) || same_conversation(stored, Some(desk_name))
}

/// Who is reading a desk history. `mine` is relative to this.
///
/// There is no `From<StoredEvent> for MessageView`, and there cannot be:
/// `mine` depends on who is asking. With one operator it was safe to hardcode
/// `true`; with several users it would mark everyone's messages as everyone
/// else's.
#[derive(Clone, Debug, PartialEq)]
pub enum Viewer {
    /// An operator or platform credential. Legacy unattributed messages are
    /// theirs, because that is who sent them before users existed.
    Operator,
    /// A human collaborator, by user id.
    User(String),
}

/// One message in a desk history, independent of transport. Mirrors
/// `frontend/src/lib/chat.ts`. The GraphQL `Message` type and the REST
/// `chat/history` JSON shape both project from this.
#[derive(Clone, Debug)]
pub struct MessageView {
    /// The message id (its EventLog sequence position).
    pub id: String,
    /// The channel the message came in on.
    pub channel: String,
    /// The author label.
    pub author: String,
    /// The message text.
    pub text: String,
    /// When it was journaled, epoch millis.
    pub at_millis: f64,
    /// Whether it is the operator's own message.
    pub mine: bool,
    /// The scrubbed processing steps behind a company reply, so a rehydrated
    /// transcript renders the same tool-call timeline the live turn showed.
    /// Empty for operator messages and tool-less replies.
    pub steps: Vec<TurnStep>,
    /// The board card this reply is about (issue #246) — the card the turn
    /// opened, or the dispatched card the turn ran for (#185).
    ///
    /// Projected here so a rehydrated transcript renders the same "card opened"
    /// chip the live turn showed. Both surfaces read it from this one field, so
    /// REST and GraphQL cannot disagree about which messages carry a card
    /// (issue #65's whole point). `None` on operator messages and on every
    /// reply journaled before the field existed.
    pub task_id: Option<String>,
    /// The message this one replies to (issue #364), by that message's own id —
    /// what makes a thread survive a reload rather than living in one browser.
    ///
    /// `None` on a message posted straight into the channel, which is every
    /// message journaled before threads were persisted.
    pub parent_id: Option<String>,
    /// Who reacted to this message with what (issue #364), one row per person
    /// per emoji, oldest reaction first.
    ///
    /// Rows rather than a tally, because a tally cannot answer the two
    /// questions the console actually asks of a reaction — *who* reacted, and
    /// *have I* — and the second is what makes the chip a toggle rather than an
    /// ever-increasing counter. Grouping rows into a count is the renderer's
    /// job; deriving names from a count is impossible.
    pub reactions: Vec<ReactionView>,
}

/// One person's reaction to one message, as a reader sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReactionView {
    /// The emoji.
    pub emoji: String,
    /// Who reacted, as a display label — never a raw user id, for the same
    /// reason [`author_labels`] never hands out an email.
    pub by_label: String,
    /// Whether this row is the viewer's own. Relative to the [`Viewer`], on the
    /// same terms [`MessageView::mine`] is.
    pub mine: bool,
}

/// How a reaction's author is keyed while folding.
///
/// A signed-in person keys on their user id; anything else — a platform
/// credential, an event journaled before attribution existed — keys on the one
/// shared "operator" identity, which is the same collapse
/// [`MessageView::project`] makes for authorship. Two different machine
/// credentials therefore share a reaction row, which is correct: they are the
/// same principal as far as this company's history is concerned.
fn reaction_actor_key(by: &Option<Actor>) -> String {
    match by {
        Some(actor) if actor.kind == ActorKind::User => format!("user:{}", actor.id),
        _ => "operator".to_string(),
    }
}

/// Folds every [`CompanyEvent::ReactionToggled`] in a log into per-message
/// reaction rows, keyed by the reacted-to message's id.
///
/// Last event per `(message, actor, emoji)` wins — that is what makes the
/// route's explicit `on` flag idempotent — and a row that ends up `off` is
/// dropped entirely rather than kept as a zero. Order is first-set order, so a
/// message's chips do not reshuffle between reads.
fn fold_reactions(
    stored: &[StoredEvent],
    viewer: &Viewer,
    authors: &HashMap<String, String>,
) -> HashMap<String, Vec<ReactionView>> {
    // (message, actor, emoji) → (position among first-seen keys, currently on).
    let mut state: HashMap<(u64, String, String), (usize, bool)> = HashMap::new();
    let mut seen = 0usize;
    for event in stored {
        let CompanyEvent::ReactionToggled {
            message_seq,
            emoji,
            on,
            by,
        } = &event.event
        else {
            continue;
        };
        let key = (message_seq.value(), reaction_actor_key(by), emoji.clone());
        match state.get_mut(&key) {
            Some(slot) => slot.1 = *on,
            None => {
                state.insert(key, (seen, *on));
                seen += 1;
            }
        }
    }

    let mut rows: Vec<(usize, u64, String, String)> = state
        .into_iter()
        .filter(|(_, (_, on))| *on)
        .map(|((message, actor, emoji), (order, _))| (order, message, actor, emoji))
        .collect();
    rows.sort_unstable();

    let mut out: HashMap<String, Vec<ReactionView>> = HashMap::new();
    for (_, message, actor, emoji) in rows {
        let (by_label, mine) = match actor.strip_prefix("user:") {
            Some(user_id) => (
                authors
                    .get(user_id)
                    .cloned()
                    .unwrap_or_else(|| "someone".to_string()),
                *viewer == Viewer::User(user_id.to_string()),
            ),
            None => ("operator".to_string(), matches!(viewer, Viewer::Operator)),
        };
        out.entry(message.to_string())
            .or_default()
            .push(ReactionView {
                emoji,
                by_label,
                mine,
            });
    }
    out
}

impl MessageView {
    /// Projects a stored event for one viewer.
    ///
    /// `authors` maps user id → display label, resolved once per history
    /// rather than per message.
    pub fn project(
        stored: StoredEvent,
        viewer: &Viewer,
        authors: &HashMap<String, String>,
    ) -> Self {
        let id = stored.seq.value().to_string();
        let at_millis = stored.at_millis as f64;
        match stored.event {
            CompanyEvent::AgentReply {
                agent_id,
                text,
                steps,
                task_id,
                parent,
                ..
            } => MessageView {
                id,
                channel: agent_id.clone(),
                author: agent_id,
                text,
                at_millis,
                mine: false,
                steps,
                task_id,
                parent_id: parent.map(|seq| seq.value().to_string()),
                reactions: Vec::new(),
            },
            CompanyEvent::OperatorMessage {
                text, by, parent, ..
            } => {
                let (author, mine) = match &by {
                    // Sent by a signed-in human.
                    Some(actor) if actor.kind == ActorKind::User => {
                        let label = authors
                            .get(&actor.id)
                            .cloned()
                            .unwrap_or_else(|| "someone".to_string());
                        (label, *viewer == Viewer::User(actor.id.clone()))
                    }
                    // Sent with a machine credential, or journaled before
                    // attribution existed. Either way there is no person to
                    // name, and it belongs to whoever holds that credential.
                    _ => ("operator".to_string(), matches!(viewer, Viewer::Operator)),
                };
                MessageView {
                    id,
                    channel: "operator".to_string(),
                    author,
                    text,
                    at_millis,
                    mine,
                    steps: Vec::new(),
                    task_id: None,
                    parent_id: parent.map(|seq| seq.value().to_string()),
                    reactions: Vec::new(),
                }
            }
            // `owns` never admits other variants into a history.
            other => MessageView {
                id,
                channel: "system".to_string(),
                author: "system".to_string(),
                text: format!("{other:?}"),
                at_millis,
                mine: false,
                steps: Vec::new(),
                task_id: None,
                parent_id: None,
                reactions: Vec::new(),
            },
        }
    }
}

/// Loads roster display labels for a company: user id → label.
///
/// Prefers a display name, and falls back to the email's *local part* rather
/// than the whole address: a desk history is read by every member, and it
/// should not hand each of them everyone else's email.
pub async fn author_labels(
    runtime: &CompanyRuntime,
) -> Result<HashMap<String, String>, OpenCompanyError> {
    let users = runtime.users().list_users(runtime.id()).await?;
    Ok(users
        .into_iter()
        .map(|user| {
            let label = user.display_name.unwrap_or_else(|| {
                user.email
                    .split('@')
                    .next()
                    .unwrap_or("someone")
                    .to_string()
            });
            (user.id, label)
        })
        .collect())
}

/// One desk's message history for one viewer, most-recent last.
///
/// `before_seq` is an opaque EventLog cursor (a sequence position); only
/// messages before it are considered. `first` caps how many of the remaining,
/// most-recent messages come back. Returns the page plus the total count of
/// matching messages (before the `first` cap, after the `before_seq` cut).
///
/// Shared by the GraphQL `Chat.history` resolver and the REST
/// `GET .../chat/history` route so the two can never disagree about what a
/// desk's history contains (issue #65).
pub async fn history_for_desk(
    runtime: &CompanyRuntime,
    desk_id: &str,
    desk_name: &str,
    viewer: &Viewer,
    before_seq: Option<u64>,
    first: usize,
) -> Result<(Vec<MessageView>, i32), OpenCompanyError> {
    let stored = runtime
        .events()
        .read_from(runtime.id(), EventSeq::new(0), usize::MAX)
        .await?;
    // One roster read per history, not one per message: the scan above is
    // already O(log), and an N+1 on top of it would be worse.
    let authors = author_labels(runtime).await?;
    // Issue #364: reactions ride the same full-log read the messages do — a
    // second pass over a list already in memory, not a second query. Folded
    // before the messages are consumed, and attached only to messages this desk
    // owns, so a reaction can no more cross a desk boundary than the message it
    // is about can.
    let mut reactions = fold_reactions(&stored, viewer, &authors);

    let mut messages: Vec<MessageView> = stored
        .into_iter()
        .filter(|event| owns(desk_id, desk_name, &event.event))
        .filter(|event| before_seq.is_none_or(|before| event.seq.value() < before))
        .map(|event| MessageView::project(event, viewer, &authors))
        .map(|mut view| {
            view.reactions = reactions.remove(&view.id).unwrap_or_default();
            view
        })
        .collect();

    let total = messages.len() as i32;
    // Keep the most recent `first`, still in chronological order.
    if messages.len() > first {
        messages.drain(0..messages.len() - first);
    }
    Ok((messages, total))
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ports::types::Actor;

    fn agent_reply(chat_id: &str) -> CompanyEvent {
        CompanyEvent::AgentReply {
            parent: None,
            task_id: None,
            chat_id: chat_id.to_string(),
            agent_id: "ceo".to_string(),
            text: "hi".to_string(),
            steps: Vec::new(),
        }
    }

    /// `None` is the shape the chat route stores for an unaddressed post.
    fn operator_message(chat: Option<&str>) -> CompanyEvent {
        CompanyEvent::OperatorMessage {
            parent: None,
            text: "hi".to_string(),
            by: None,
            chat: chat.map(str::to_string),
        }
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
            parent: None,
            text: "hi".to_string(),
            by: Some(Actor {
                kind: ActorKind::User,
                id: "u1".to_string(),
            }),
            chat: Some(MAIN_THREAD_ID.to_string()),
        };
        assert!(owns(GENERAL_DESK, GENERAL_DESK, &event));
        assert!(!owns("strategy", "Strategy desk", &event));
    }

    // Regression: issue — operator messages vanished on reload because the read
    // filter ignored the stored chat id.
    #[test]
    fn main_thread_owns_operator_messages_it_stored() {
        let event = CompanyEvent::OperatorMessage {
            parent: None,
            text: "hi".to_string(),
            by: None,
            chat: Some(MAIN_THREAD_ID.to_string()),
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
            parent: None,
            text: "hi".to_string(),
            by: None,
            chat: Some("strategy".to_string()),
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

    /// A thread parent survives projection on both halves of an exchange, as
    /// the message id a reader can resolve rather than a raw sequence number.
    #[test]
    fn project_carries_the_thread_parent() {
        let operator = MessageView::project(
            at(
                12,
                CompanyEvent::OperatorMessage {
                    parent: Some(EventSeq::new(4)),
                    text: "a follow-up".to_string(),
                    by: None,
                    chat: Some("studio".to_string()),
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
                    parent: Some(EventSeq::new(4)),
                    task_id: None,
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
            parent: None,
            text: "hi".to_string(),
            by: None,
            chat: None,
        };
        assert!(owns(GENERAL_DESK, GENERAL_DESK, &event));
        assert!(!owns("strategy", "Strategy desk", &event));
    }
}
