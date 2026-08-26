//! Recent-chat history seed for a resumed agent turn (issue #1840).
//!
//! # Why this exists
//!
//! One `Agent` is reused for every chat of a `(company, agent_id)` pair, and its
//! in-memory `history` is cleared and re-bound on every chat switch
//! ([`super::CompanyAgent::run_with_steer`]). The re-seed there originally went
//! through OpenHuman's `seed_resume_from_thread_transcript`, which reads a
//! per-thread OpenHuman **file** transcript — a file OpenCompany never writes for
//! a `chat_id` (its web-channel session is built with `.auto_save(false)` and no
//! thread binding). So the lookup always missed, the agent started every chat
//! reply at `history_len = 0`, and the model answered without the recent
//! conversation in front of it (the #1725/#1730 regression).
//!
//! OpenCompany already holds the authoritative transcript: the company
//! [`EventLog`]. This module projects the last `window` messages **that belong to
//! the incoming desk** out of that log into the lossy `(role, content)` shape
//! [`Agent::seed_resume_from_messages`](openhuman_core::openhuman::agent::Agent::seed_resume_from_messages)
//! accepts, so the switch branch can seed the correct thread's own recent turns
//! directly instead of chasing a file that isn't there.
//!
//! # Isolation
//!
//! The ownership test is [`chat_history::owns`] — the *same* predicate the
//! console's history surfaces use, so a seed contains exactly the lines the UI
//! renders for that desk and nothing from any other. That reuse is deliberate:
//! it gives DM (`dm:<id>`) vs named-desk parity for free and keeps a switch from
//! ever leaking the previous chat's lines into the next one.

use std::sync::Arc;

use crate::ports::types::{CompanyEvent, CompanyId};
use crate::ports::{CompanyStore, EventLog};
use crate::server::chat_history;
use crate::server::ops::language::DEFAULT_DESK;

/// What a chat turn needs to build its recent-history seed, carried into
/// [`super::CompanyAgent::run_with_steer`] rather than an already-projected
/// `Vec`.
///
/// The projection itself ([`build_chat_seed`]) is only done inside
/// `run_with_steer`, after the chat-switch decision (under the same
/// `bound_chat` lock) confirms a re-seed is actually needed — never for a
/// turn that keeps the same bound chat as the one before it. Handing the
/// caller-built `Vec` in unconditionally meant the (filesystem-backend-costly
/// — see [`build_chat_seed`]'s docs) journal walk ran on *every* chat turn,
/// switch or not, since the caller has no way to see the switch decision
/// before it (codex review finding).
///
/// `None` for every non-chat turn — background, workflow, or [confined]
/// (`confine::run_confined`) — which want no seed regardless of switch
/// status, exactly like passing an empty seed did before this type existed.
///
/// [confined]: super::confine
pub struct ChatSeedRequest {
    /// The turn's raw, pre-memory-injection text — what
    /// [`strip_current_message`] matches against. Deliberately NOT
    /// `run_with_steer`'s own `message` argument: that one is the
    /// memory-augmented turn text, which the journal never recorded (see
    /// `strip_current_message`'s docs).
    pub raw_message: String,
    /// The company journal [`build_chat_seed`] projects the seed from.
    pub events: Arc<dyn EventLog>,
    /// Resolves the incoming chat id to its desk id/name pair (see
    /// [`resolve_seed_desk`]).
    pub store: Arc<dyn CompanyStore>,
}

impl ChatSeedRequest {
    /// Projects this desk's recent history and strips the current message's
    /// own duplicate, in one call — the two steps [`super::CompanyAgent::run_with_steer`]'s
    /// switch branch needs, together.
    pub async fn build(&self, company: &CompanyId, chat_id: &str) -> Vec<(String, String)> {
        let (desk_id, desk_name) = resolve_seed_desk(&self.store, company, Some(chat_id)).await;
        let mut seed = build_chat_seed(
            &self.events,
            company,
            &desk_id,
            &desk_name,
            CHAT_SEED_WINDOW,
        )
        .await;
        strip_current_message(&mut seed, &self.raw_message);
        seed
    }
}

/// How many of the most-recent owning messages a chat seed carries.
///
/// A conversational window, not the whole transcript: enough that a reply lands
/// in the flow of the recent exchange, small enough that a resumed turn does not
/// re-send an unbounded history on every switch. OpenHuman's own
/// `max_history_messages` bound still applies on top of this (see
/// `bound_cached_transcript_messages`), so this is an upper request, not a
/// guarantee.
pub const CHAT_SEED_WINDOW: usize = 30;

/// How many raw journal events to pull per backward page while filtering down to
/// owning messages. A busy company interleaves unrelated events between two chat
/// turns, so the event page is larger than the message window — mirrors
/// [`chat_history::history_for_desk`]'s `EVENT_PAGE`.
const EVENT_PAGE: usize = 512;

/// Resolves an incoming `chat_id` to the `(desk_id, desk_name)` pair
/// [`chat_history::owns`] filters on, exactly as the REST history route's
/// `resolve_desk` does (issue #65).
///
/// `owns` matches a stored event's chat id against *both* the desk id and the
/// desk name, because a named desk's messages can be journaled under either
/// spelling. Passing `(chat_id, chat_id)` for a desk the operator addressed by
/// id would therefore silently miss any line stored under its name — a seed that
/// "looks fixed" but is empty. So a non-General selector is resolved against the
/// manifest's group chats the same way the console resolves it.
///
/// * `None` → the synthetic General/operator desk.
/// * A General spelling (`"main"` / `"general"` / `""`) short-circuits: every
///   spelling folds together in [`chat_history::same_conversation`], so no
///   manifest read is needed and `(chat, chat)` already owns all of them.
/// * Anything else is matched (case-insensitive, by id or name) against the
///   manifest's group chats; an unmatched selector passes through as `(id, name)
///   = (chat, chat)`, so an ad-hoc thread id still finds what was journaled under
///   that exact string.
pub async fn resolve_seed_desk(
    store: &Arc<dyn CompanyStore>,
    company: &CompanyId,
    chat_id: Option<&str>,
) -> (String, String) {
    let Some(desk) = chat_id else {
        return (DEFAULT_DESK.to_string(), DEFAULT_DESK.to_string());
    };
    if chat_history::is_general_chat(Some(desk)) {
        return (desk.to_string(), desk.to_string());
    }
    match store.load(company).await {
        Ok(Some(record)) => record
            .manifest
            .group_chats
            .iter()
            .find(|chat| chat.id.eq_ignore_ascii_case(desk) || chat.name.eq_ignore_ascii_case(desk))
            .map(|chat| (chat.id.clone(), chat.name.clone()))
            .unwrap_or_else(|| (desk.to_string(), desk.to_string())),
        // A store miss or read error must not fail the turn — fall back to the
        // verbatim selector, which still owns everything journaled under that
        // exact string (the common case, where the console addresses id == name).
        Ok(None) | Err(_) => (desk.to_string(), desk.to_string()),
    }
}

/// Projects the last `window` messages owned by `(desk_id, desk_name)` out of the
/// company [`EventLog`] into chronological `(role, content)` pairs for
/// [`Agent::seed_resume_from_messages`](openhuman_core::openhuman::agent::Agent::seed_resume_from_messages).
///
/// Walks the log newest-first (`read_before`), keeps only the events
/// [`chat_history::owns`] admits for this desk, maps each to a role
/// (`OperatorMessage` → `user`, `AgentReply` → `agent`), stops once `window`
/// messages are gathered, and reverses to chronological order. Non-conversational
/// owned events (a settled-dispatch terminal, reactions, anything without body
/// text) are skipped even when `owns` admits them — a seed needs role + text, not
/// structural markers.
///
/// Best-effort: a read error yields an empty seed (the caller then falls back to
/// the OpenHuman transcript lookup) rather than failing the turn.
pub async fn build_chat_seed(
    events: &Arc<dyn EventLog>,
    company: &CompanyId,
    desk_id: &str,
    desk_name: &str,
    window: usize,
) -> Vec<(String, String)> {
    if window == 0 {
        return Vec::new();
    }

    // Newest-first accumulation; reversed to chronological before returning.
    let mut collected: Vec<(String, String)> = Vec::with_capacity(window);
    let mut cursor = None;

    while collected.len() < window {
        let page = match events.read_before(company, cursor, EVENT_PAGE).await {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(
                    company = %company,
                    desk = desk_id,
                    %error,
                    "[chat-seed] event-log read failed; seeding no recent history (falling back to transcript lookup)"
                );
                return Vec::new();
            }
        };
        if page.is_empty() {
            break;
        }
        cursor = page.last().map(|event| event.seq);
        for stored in page {
            if !chat_history::owns(desk_id, desk_name, &stored.event) {
                continue;
            }
            let mapped = match &stored.event {
                CompanyEvent::OperatorMessage { text, .. } => Some(("user", text.as_str())),
                CompanyEvent::AgentReply { text, .. } => Some(("agent", text.as_str())),
                // `owns` also admits `DeskTaskCompleted` (a structural "finished →
                // In review" marker), but it carries no conversational body — do
                // not seed it as a turn.
                _ => None,
            };
            let Some((role, text)) = mapped else { continue };
            if text.trim().is_empty() {
                continue;
            }
            collected.push((role.to_string(), text.to_string()));
            if collected.len() == window {
                break;
            }
        }
    }

    collected.reverse();
    collected
}

/// Drops a trailing `("user", …)` entry whose text matches `current_message`.
///
/// The operator's current message is journaled **before** the harness turn runs
/// (the server appends it, then the brain dispatches), so it is already the
/// newest owning event when [`build_chat_seed`] reads the tail. Seeding it as
/// prior history would duplicate it on the wire — `run_single` appends the
/// current message to `history` itself. OpenHuman's `seed_resume_from_messages`
/// performs the same drop, but it can only match against the *augmented* message
/// the runner passes it; the raw operator text is only in scope here, so strip it
/// here where the match is exact.
pub fn strip_current_message(seed: &mut Vec<(String, String)>, current_message: &str) {
    if let Some((role, text)) = seed.last()
        && role == "user"
        && text.trim() == current_message.trim()
    {
        seed.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use futures::stream::{self, BoxStream};

    use crate::ports::events::EventStreamItem;
    use crate::ports::types::{EventSeq, StoredEvent};

    /// A log that replays a fixed history in ascending sequence order. The
    /// trait's default `read_before` (forward-read + reverse + truncate) then
    /// gives us newest-first paging for free — exactly what production backends
    /// override but what a fixture does not need to.
    struct FixedLog(Vec<StoredEvent>);

    #[async_trait]
    impl EventLog for FixedLog {
        async fn append(&self, _id: &CompanyId, _event: CompanyEvent) -> crate::Result<EventSeq> {
            unreachable!("the seed projector only reads")
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            seq: EventSeq,
            limit: usize,
        ) -> crate::Result<Vec<StoredEvent>> {
            Ok(self
                .0
                .iter()
                .filter(|e| e.seq.value() >= seq.value())
                .take(limit)
                .cloned()
                .collect())
        }
        fn subscribe(&self, _id: &CompanyId) -> BoxStream<'static, EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    /// A log whose reads always fail, to prove the projector degrades to an empty
    /// seed rather than propagating.
    struct BrokenLog;

    #[async_trait]
    impl EventLog for BrokenLog {
        async fn append(&self, _id: &CompanyId, _event: CompanyEvent) -> crate::Result<EventSeq> {
            unreachable!()
        }
        async fn read_from(
            &self,
            _id: &CompanyId,
            _seq: EventSeq,
            _limit: usize,
        ) -> crate::Result<Vec<StoredEvent>> {
            Err(OpenCompanyError::InvalidRequest("boom".to_string()))
        }
        fn subscribe(&self, _id: &CompanyId) -> BoxStream<'static, EventStreamItem> {
            Box::pin(stream::empty())
        }
    }

    use crate::error::OpenCompanyError;

    fn operator(seq: u64, chat: Option<&str>, text: &str) -> StoredEvent {
        StoredEvent {
            seq: EventSeq::new(seq),
            company: CompanyId::new("acme"),
            event: CompanyEvent::OperatorMessage {
                text: text.to_string(),
                by: None,
                chat: chat.map(str::to_string),
                parent: None,
                deliverable: None,
                mentions: Vec::new(),
                attachments: Vec::new(),
            },
            at_millis: seq,
        }
    }

    fn reply(seq: u64, chat_id: &str, text: &str) -> StoredEvent {
        StoredEvent {
            seq: EventSeq::new(seq),
            company: CompanyId::new("acme"),
            event: CompanyEvent::AgentReply {
                chat_id: chat_id.to_string(),
                agent_id: "ceo".to_string(),
                text: text.to_string(),
                steps: Vec::new(),
                task_id: None,
                parent: None,
                mentions: Vec::new(),
                mention_depth: 0,
            },
            at_millis: seq,
        }
    }

    fn desk_completed(seq: u64, origin_chat_id: Option<&str>) -> StoredEvent {
        StoredEvent {
            seq: EventSeq::new(seq),
            company: CompanyId::new("acme"),
            event: CompanyEvent::DeskTaskCompleted {
                task_id: "t-1".to_string(),
                desk: "eng".to_string(),
                output: "shipped".to_string(),
                column: "done".to_string(),
                artifact_ids: Vec::new(),
                origin_chat_id: origin_chat_id.map(str::to_string),
            },
            at_millis: seq,
        }
    }

    async fn seed_of(
        log: FixedLog,
        desk_id: &str,
        desk_name: &str,
        window: usize,
    ) -> Vec<(String, String)> {
        let events: Arc<dyn EventLog> = Arc::new(log);
        build_chat_seed(&events, &CompanyId::new("acme"), desk_id, desk_name, window).await
    }

    /// The core projection: a journal interleaving the General desk's own
    /// operator/agent turns with an unrelated desk's message, a structural
    /// dispatch terminal, and an empty reply. Only the General desk's real
    /// conversational turns survive, in chronological order, with the right roles.
    #[tokio::test]
    async fn projects_only_owning_conversational_turns_in_order() {
        let log = FixedLog(vec![
            operator(0, Some("general"), "u1"),
            reply(1, "general", "a1"),
            // Another desk entirely — must never appear in General's seed.
            operator(2, Some("engineering"), "OTHER-DESK"),
            reply(3, "engineering", "OTHER-REPLY"),
            // `owns` admits this (origin is General) but it is a structural
            // marker, not a turn — the projector must skip it.
            desk_completed(4, Some("general")),
            // A blank reply carries no body to seed.
            reply(5, "general", "   "),
            operator(6, Some("general"), "u2"),
        ]);

        let seed = seed_of(log, "general", "general", CHAT_SEED_WINDOW).await;

        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "u1".to_string()),
                ("agent".to_string(), "a1".to_string()),
                ("user".to_string(), "u2".to_string()),
            ],
            "only General's own operator/agent turns, chronological, correctly roled"
        );
    }

    /// The General desk answers to every spelling of itself, so a reply journaled
    /// under `"General"` and a `"main"` operator line both land in the seed for a
    /// desk addressed as `"main"` — the folding `owns`/`same_conversation` give.
    #[tokio::test]
    async fn general_desk_folds_its_spellings() {
        let log = FixedLog(vec![
            operator(0, None, "unaddressed"),
            reply(1, "General", "under-General"),
            operator(2, Some("main"), "under-main"),
        ]);

        let seed = seed_of(log, "main", "main", CHAT_SEED_WINDOW).await;

        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "unaddressed".to_string()),
                ("agent".to_string(), "under-General".to_string()),
                ("user".to_string(), "under-main".to_string()),
            ],
        );
    }

    /// DM parity: a `dm:<id>` thread is an opaque verbatim key, so its own turns
    /// project and a sibling DM's do not.
    #[tokio::test]
    async fn dm_thread_projects_and_isolates() {
        let log = FixedLog(vec![
            operator(0, Some("dm:alice"), "hi alice"),
            reply(1, "dm:alice", "hi back"),
            operator(2, Some("dm:bob"), "hi bob"),
        ]);

        let seed = seed_of(log, "dm:alice", "dm:alice", CHAT_SEED_WINDOW).await;

        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "hi alice".to_string()),
                ("agent".to_string(), "hi back".to_string()),
            ],
            "only the addressed DM's own turns, never the sibling DM's"
        );
    }

    /// A named desk's turns can be journaled under either its id or its name;
    /// `owns` matches both, so passing the resolved `(id, name)` pair seeds every
    /// line regardless of which spelling wrote it.
    #[tokio::test]
    async fn named_desk_matches_id_and_name() {
        let log = FixedLog(vec![
            operator(0, Some("eng-123"), "by id"),
            reply(1, "Engineering", "by name"),
            operator(2, Some("marketing"), "OTHER"),
        ]);

        let seed = seed_of(log, "eng-123", "Engineering", CHAT_SEED_WINDOW).await;

        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "by id".to_string()),
                ("agent".to_string(), "by name".to_string()),
            ],
        );
    }

    /// The window keeps the most-recent `window` owning turns and drops older
    /// ones, even when unrelated events sit between them.
    #[tokio::test]
    async fn window_keeps_the_most_recent_turns() {
        let mut events = Vec::new();
        for n in 0..10u64 {
            events.push(operator(n, Some("general"), &format!("m{n}")));
        }
        let log = FixedLog(events);

        let seed = seed_of(log, "general", "general", 3).await;

        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "m7".to_string()),
                ("user".to_string(), "m8".to_string()),
                ("user".to_string(), "m9".to_string()),
            ],
            "the three newest owning turns, in chronological order"
        );
    }

    /// A read failure yields an empty seed, never a propagated error — the caller
    /// then falls back to the OpenHuman transcript lookup.
    #[tokio::test]
    async fn read_failure_degrades_to_empty() {
        let events: Arc<dyn EventLog> = Arc::new(BrokenLog);
        let seed = build_chat_seed(
            &events,
            &CompanyId::new("acme"),
            "general",
            "general",
            CHAT_SEED_WINDOW,
        )
        .await;
        assert!(seed.is_empty());
    }

    #[test]
    fn strip_current_message_drops_only_a_matching_trailing_user() {
        let mut seed = vec![
            ("user".to_string(), "u1".to_string()),
            ("agent".to_string(), "a1".to_string()),
            ("user".to_string(), "  current  ".to_string()),
        ];
        strip_current_message(&mut seed, "current");
        assert_eq!(
            seed,
            vec![
                ("user".to_string(), "u1".to_string()),
                ("agent".to_string(), "a1".to_string()),
            ],
            "a trailing user line matching the current message (trim-insensitive) is dropped"
        );

        // A trailing agent line is never the current operator message.
        let mut ends_in_agent = vec![("agent".to_string(), "current".to_string())];
        strip_current_message(&mut ends_in_agent, "current");
        assert_eq!(ends_in_agent.len(), 1, "an agent tail is never stripped");

        // A non-matching trailing user line stays.
        let mut different = vec![("user".to_string(), "something else".to_string())];
        strip_current_message(&mut different, "current");
        assert_eq!(different.len(), 1, "a non-matching user tail stays");
    }

    // ---- resolve_seed_desk ------------------------------------------------

    struct RecordStore(Option<CompanyRecord>);

    #[async_trait]
    impl CompanyStore for RecordStore {
        async fn load(&self, _id: &CompanyId) -> crate::Result<Option<CompanyRecord>> {
            Ok(self.0.clone())
        }
        async fn save(&self, _record: &CompanyRecord) -> crate::Result<()> {
            unreachable!("resolve only reads")
        }
        async fn list(&self) -> crate::Result<Vec<CompanySummary>> {
            unreachable!("resolve only reads")
        }
        async fn append_ledger(
            &self,
            _id: &CompanyId,
            _entry: crate::ports::types::LedgerEntry,
        ) -> crate::Result<()> {
            unreachable!("resolve only reads")
        }
    }

    use crate::ports::types::{CompanyRecord, CompanySummary};

    fn record_with_group_chat(id: &str, name: &str) -> CompanyRecord {
        let manifest = toml::from_str(&format!(
            r#"
[company]
name = "Acme"

[policy]
mode = "full"

[[agent]]
id = "ceo"
role = "Chief Executive"
description = "Sets direction."

[[group_chat]]
id = "{id}"
name = "{name}"
"#,
        ))
        .expect("valid manifest");
        CompanyRecord {
            overlay_retired_agents: Vec::new(),
            overlay_agent_edits: Vec::new(),
            id: CompanyId::new("acme"),
            manifest,
            ledger: Vec::new(),
            lifecycle: "running".to_string(),
            setup: None,
            overlay_agents: Vec::new(),
            overlay_desk_members: Vec::new(),
            overlay_desk_order: Vec::new(),
            overlay_desks: Vec::new(),
            overlay_workflows: Vec::new(),
            overlay_budgets: Vec::new(),
            overlay_policy: None,
            overlay_desk_tools: Default::default(),
            disabled_workflows: Vec::new(),
            template_provenance: None,
        }
    }

    async fn resolve(store: RecordStore, chat_id: Option<&str>) -> (String, String) {
        let store: Arc<dyn CompanyStore> = Arc::new(store);
        resolve_seed_desk(&store, &CompanyId::new("acme"), chat_id).await
    }

    #[tokio::test]
    async fn resolve_none_is_the_general_desk() {
        assert_eq!(
            resolve(RecordStore(None), None).await,
            (DEFAULT_DESK.to_string(), DEFAULT_DESK.to_string())
        );
    }

    #[tokio::test]
    async fn resolve_general_spelling_short_circuits_without_a_store_read() {
        // The store would panic on `save`/`list`, but a General spelling must not
        // even reach `load` — it returns `(chat, chat)`, which owns folds.
        assert_eq!(
            resolve(RecordStore(None), Some("main")).await,
            ("main".to_string(), "main".to_string())
        );
    }

    #[tokio::test]
    async fn resolve_named_desk_by_id_returns_the_manifest_name() {
        // Addressed by id; the seed must carry the name too, or a line journaled
        // under the name would be missed. This is the exact "looks fixed but seeds
        // nothing" trap the resolution guards against.
        let store = RecordStore(Some(record_with_group_chat("eng-123", "Engineering")));
        assert_eq!(
            resolve(store, Some("eng-123")).await,
            ("eng-123".to_string(), "Engineering".to_string())
        );
    }

    #[tokio::test]
    async fn resolve_unmatched_selector_passes_through_verbatim() {
        let store = RecordStore(Some(record_with_group_chat("eng-123", "Engineering")));
        assert_eq!(
            resolve(store, Some("ad-hoc-thread")).await,
            ("ad-hoc-thread".to_string(), "ad-hoc-thread".to_string())
        );
    }
}
