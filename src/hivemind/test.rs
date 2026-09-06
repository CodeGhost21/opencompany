//! Unit tests for the hive-mind desk seam.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use tinyhivemind_hive::{SESSION_WINDOW, Sequence, SessionAuthor, SessionQuery, project_session};

use super::*;
use crate::Result;
use crate::ports::events::{EventLog, EventStreamItem};
use crate::ports::types::{CompanyEvent, CompanyId, CompanyRecord, EventSeq, StoredEvent};

/// An in-memory journal, the smallest thing that satisfies the port.
///
/// `read_before` is implemented directly rather than inherited from the port's
/// forward-scan default, because the session adapter's paging is one of the
/// things under test and a default that reads the whole log would hide a cursor
/// bug rather than expose it.
#[derive(Default)]
pub(super) struct MemoryLog {
    events: Mutex<Vec<StoredEvent>>,
}

impl MemoryLog {
    pub(super) fn company() -> CompanyId {
        CompanyId::new("acme")
    }

    fn rows(&self) -> Vec<StoredEvent> {
        self.events.lock().expect("journal poisoned").clone()
    }

    /// Every `AgentReply` on `chat`, as `(author, text)` in journal order.
    pub(super) fn replies(&self, chat: &str) -> Vec<(String, String)> {
        self.rows()
            .into_iter()
            .filter_map(|stored| match stored.event {
                CompanyEvent::AgentReply {
                    chat_id,
                    agent_id,
                    text,
                    ..
                } if chat_id == chat => Some((agent_id, text)),
                _ => None,
            })
            .collect()
    }
}

#[async_trait]
impl EventLog for MemoryLog {
    async fn append(&self, _id: &CompanyId, event: CompanyEvent) -> Result<EventSeq> {
        let mut events = self.events.lock().expect("journal poisoned");
        let seq = EventSeq::new(events.len() as u64 + 1);
        events.push(StoredEvent {
            seq,
            company: MemoryLog::company(),
            event,
            at_millis: 0,
        });
        Ok(seq)
    }

    async fn read_from(
        &self,
        _id: &CompanyId,
        seq: EventSeq,
        limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        Ok(self
            .rows()
            .into_iter()
            .filter(|stored| stored.seq.value() >= seq.value())
            .take(limit)
            .collect())
    }

    async fn read_before(
        &self,
        _id: &CompanyId,
        before: Option<EventSeq>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        let mut rows: Vec<StoredEvent> = self
            .rows()
            .into_iter()
            .filter(|stored| before.is_none_or(|cursor| stored.seq.value() < cursor.value()))
            .collect();
        rows.reverse();
        rows.truncate(limit);
        Ok(rows)
    }

    fn subscribe(&self, _id: &CompanyId) -> BoxStream<'static, EventStreamItem> {
        Box::pin(stream::empty())
    }
}

/// A participant that answers from a script, keyed by agent id.
///
/// A queue per agent rather than one flat list: the library decides who speaks,
/// so a test that scripted a flat sequence would be asserting the bid order by
/// accident and would break for reasons that have nothing to do with what it
/// meant to check.
struct ScriptedRunner {
    lines: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<(String, String)>>,
}

impl ScriptedRunner {
    fn new(lines: &[(&str, &str)]) -> Self {
        Self {
            lines: Mutex::new(
                lines
                    .iter()
                    .map(|(id, line)| ((*id).to_owned(), (*line).to_owned()))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// Every `(agent, prompt)` the episode asked for, in order.
    fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().expect("script poisoned").clone()
    }
}

#[async_trait]
impl HiveTurnRunner for ScriptedRunner {
    async fn speak(&self, agent_id: &str, prompt: &str) -> Result<String> {
        self.asked
            .lock()
            .expect("script poisoned")
            .push((agent_id.to_owned(), prompt.to_owned()));
        let mut lines = self.lines.lock().expect("script poisoned");
        let at = lines.iter().position(|(id, _)| id == agent_id);
        Ok(match at {
            Some(at) => lines.remove(at).1,
            // A member with nothing scripted left still has to say something —
            // the episode is entitled to a reply for every turn it authorizes.
            None => format!("!question {agent_id} has nothing further."),
        })
    }
}

pub(super) fn record(manifest: &str) -> CompanyRecord {
    let manifest: crate::company::CompanyManifest =
        toml::from_str(manifest).expect("test manifest parses");
    CompanyRecord {
        overlay_retired_agents: Vec::new(),
        overlay_agent_edits: Vec::new(),
        id: MemoryLog::company(),
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

/// Three teammates on one desk, all seated.
fn three_member_manifest() -> String {
    "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"planner\"\nrole = \"Planner\"\n\
     [[agent]]\nid = \"scout\"\nrole = \"Scout\"\n\
     [[agent]]\nid = \"critic\"\nrole = \"Critic\"\n\
     [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\n\
     description = \"Ship the rollout\"\n\
     members = [\"planner\", \"scout\", \"critic\"]\n"
        .to_string()
}

pub(super) fn desk_of(manifest: &str, chat: &str) -> Option<HiveDesk> {
    desk_episode(&record(manifest), Some(chat))
}

// ---------------------------------------------------------------------------
// The manifest knob
// ---------------------------------------------------------------------------

#[test]
fn a_desk_with_two_members_deliberates_by_default() {
    let desk = desk_of(&three_member_manifest(), "eng").expect("three members is a room");
    assert_eq!(desk.id, "eng");
    assert_eq!(desk.name, "Engineering");
    assert_eq!(desk.description.as_deref(), Some("Ship the rollout"));
    assert_eq!(desk.member_ids(), ["planner", "scout", "critic"]);
    assert_eq!(desk.members[0].role, "Planner");
}

#[test]
fn a_single_member_desk_never_enters_the_driver() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"solo\"\nrole = \"Everything\"\n\
         [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\nmembers = [\"solo\"]\n";
    assert!(desk_of(manifest, "eng").is_none());
    // And saying so explicitly does not conjure a room out of one member.
    let opted_in = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"solo\"\nrole = \"Everything\"\n\
         [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\nmembers = [\"solo\"]\n\
         hive = { enabled = true }\n";
    assert!(desk_of(opted_in, "eng").is_none());
}

#[test]
fn an_explicit_opt_out_keeps_the_single_responder_path() {
    let manifest = format!("{}hive = {{ enabled = false }}\n", three_member_manifest());
    assert!(desk_of(&manifest, "eng").is_none());
}

#[test]
fn general_dms_and_unknown_keys_are_not_desks() {
    let manifest = three_member_manifest();
    let record = record(&manifest);
    for chat in [None, Some(""), Some("main"), Some("General")] {
        assert!(
            desk_episode(&record, chat).is_none(),
            "the company's own line is not a deliberating desk: {chat:?}"
        );
    }
    assert!(desk_episode(&record, Some("dm:planner")).is_none());
    assert!(desk_episode(&record, Some("planner")).is_none());
    assert!(desk_episode(&record, Some("nowhere")).is_none());
}

#[test]
fn the_manifest_parses_a_hive_block_and_rejects_zero_bounds() {
    let manifest = format!(
        "{}hive = {{ enabled = true, turn_budget = 6, quorum = 2, blind_round = false }}\n",
        three_member_manifest()
    );
    let desk = desk_of(&manifest, "eng").expect("an opted-in three-member desk is a room");
    assert_eq!(desk.config.turn_budget, Some(6));
    assert_eq!(desk.config.quorum, Some(2));
    assert_eq!(desk.config.blind_round, Some(false));
    let policy = desk.policy();
    assert_eq!(policy.turn_budget, 6);
    assert_eq!(policy.quorum.threshold, 2);
    assert!(!policy.blind_round);

    let problems = record(&format!(
        "{}hive = {{ quorum = 0 }}\n",
        three_member_manifest()
    ))
    .manifest
    .validate();
    assert!(
        problems.iter().any(|p| p.contains("hive.quorum = 0")),
        "{problems:?}"
    );

    let problems = record(&format!(
        "{}hive = {{ turn_budget = 0 }}\n",
        three_member_manifest()
    ))
    .manifest
    .validate();
    assert!(
        problems.iter().any(|p| p.contains("hive.turn_budget = 0")),
        "{problems:?}"
    );
}

#[test]
fn the_derived_policy_scales_with_the_room() {
    let policy = |members: usize| HivePolicy::from_config(&HiveConfig::default(), members).episode;
    // A pair can only ever need one supporter — the other one — so a majority
    // that still leaves somebody outside it is exactly one.
    assert_eq!(policy(2).quorum.threshold, 1);
    assert_eq!(policy(3).quorum.threshold, 2);
    assert_eq!(policy(4).quorum.threshold, 3);
    assert_eq!(policy(5).quorum.threshold, 3);
    assert_eq!(policy(3).turn_budget, 9);
    assert!(policy(3).blind_round);
    // An operator's number is honoured, but never one the room could not meet.
    let over = HiveConfig {
        quorum: Some(99),
        ..HiveConfig::default()
    };
    assert_eq!(
        HivePolicy::from_config(&over, 3).episode.quorum.threshold,
        3
    );
}

// ---------------------------------------------------------------------------
// The log adapter
// ---------------------------------------------------------------------------

async fn seed_desk(log: &MemoryLog) -> EventSeq {
    let company = MemoryLog::company();
    // Rows the desk must not see, interleaved so the adapter has to filter
    // rather than merely truncate.
    log.append(
        &company,
        CompanyEvent::OperatorMessage {
            text: "not this desk".into(),
            by: None,
            chat: Some("sales".into()),
            parent: None,
            deliverable: None,
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
    )
    .await
    .unwrap();
    log.append(
        &company,
        CompanyEvent::LifecycleChanged {
            from: "running".into(),
            to: "paused".into(),
            by: crate::ports::types::Actor {
                kind: crate::ports::types::ActorKind::Operator,
                id: "operator".into(),
            },
        },
    )
    .await
    .unwrap();
    log.append(
        &company,
        CompanyEvent::OperatorMessage {
            text: "Decide the rollout.".into(),
            by: None,
            // Addressed by display name, in the wrong case: the adapter
            // canonicalises it, so the room still opens on the desk.
            chat: Some("ENGINEERING".into()),
            parent: None,
            deliverable: None,
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
    )
    .await
    .unwrap()
}

fn session_log(log: &Arc<MemoryLog>) -> EventLogSessionLog {
    EventLogSessionLog::new(
        Arc::clone(log) as Arc<dyn EventLog>,
        MemoryLog::company(),
        "eng".into(),
        "Engineering".into(),
    )
}

fn conversation() -> tinyhivemind_hive::Conversation {
    tinyhivemind_hive::Conversation {
        desk_id: "eng".into(),
        desk_name: "Engineering".into(),
        thread_root: None,
    }
}

#[tokio::test]
async fn the_log_adapter_attributes_and_pages_desk_rows() {
    let log = Arc::new(MemoryLog::default());
    let trigger = seed_desk(&log).await;
    let company = MemoryLog::company();
    log.append(
        &company,
        CompanyEvent::AgentReply {
            chat_id: "eng".into(),
            agent_id: "planner".into(),
            text: "!propose #stage Stage the rollout.".into(),
            steps: Vec::new(),
            task_id: None,
            parent: None,
            mentions: Vec::new(),
            mention_depth: 0,
        },
    )
    .await
    .unwrap();
    log.append(
        &company,
        CompanyEvent::AgentReply {
            chat_id: "eng".into(),
            agent_id: HIVE_REPORT_AUTHOR.into(),
            text: "An earlier episode ended.".into(),
            steps: Vec::new(),
            task_id: None,
            parent: None,
            mentions: Vec::new(),
            mention_depth: 0,
        },
    )
    .await
    .unwrap();

    let adapter = session_log(&log);
    let projected = project_session(
        &adapter,
        &SessionQuery {
            conversation: conversation(),
            before: None,
            window: SESSION_WINDOW,
        },
    )
    .await
    .expect("the page contract holds");

    let authors: Vec<&SessionAuthor> = projected.iter().map(|m| &m.author).collect();
    assert_eq!(projected.len(), 3, "only this desk's chat: {projected:?}");
    assert!(matches!(authors[0], SessionAuthor::Operator));
    assert!(
        matches!(authors[1], SessionAuthor::Agent { id, .. } if id == "planner"),
        "{authors:?}"
    );
    // The room's own outcome row is a system line, so it can never be counted
    // as a supporter and is never hidden by a blind round.
    assert!(
        matches!(authors[2], SessionAuthor::System { kind, .. } if kind == HIVE_REPORT_AUTHOR),
        "{authors:?}"
    );
    assert_eq!(projected[0].sequence, Sequence(trigger.value()));

    // The port's own paging contract: newest-first, `before` exclusive, and a
    // page no larger than asked for.
    let page = tinyhivemind_hive::SessionLog::read_before(&adapter, None, 2)
        .await
        .expect("a bounded page");
    assert_eq!(page.messages.len(), 2);
    assert!(page.messages[0].sequence > page.messages[1].sequence);
    let cursor = page.next_before.expect("more rows remain");
    let older = tinyhivemind_hive::SessionLog::read_before(&adapter, Some(cursor), 8)
        .await
        .expect("the older page");
    assert!(
        older.messages.iter().all(|m| m.sequence < cursor),
        "`before` is exclusive: {older:?}"
    );
}

// ---------------------------------------------------------------------------
// The episode driver
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_scripted_room_converges_and_journals_the_right_authors() {
    let log = Arc::new(MemoryLog::default());
    let trigger = seed_desk(&log).await;
    let desk = desk_of(&three_member_manifest(), "eng").expect("a room");
    let runner = ScriptedRunner::new(&[
        (
            "planner",
            "!propose #stage Stage the rollout behind a flag.",
        ),
        ("scout", "!propose #ship Ship it all at once."),
        (
            "critic",
            "!evidence #stage ^3 The last full rollout took the checkout down.",
        ),
        (
            "planner",
            "!support #stage ^3 Staging bounds the blast radius.",
        ),
        ("scout", "!support #stage ^3 Agreed, and it is reversible."),
        ("critic", "!commit #stage ^3 The room settled on staging."),
        ("planner", "!commit #stage ^3 Recorded."),
        ("scout", "!commit #stage ^3 Recorded."),
    ]);

    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .run(trigger)
    .await
    .expect("the episode runs");

    assert!(
        matches!(&outcome.ending, EpisodeEnding::Converged { topic, .. } if topic == "stage"),
        "{outcome:?}"
    );
    assert!(outcome.turns >= 3, "{outcome:?}");
    assert!(outcome.first_seq.is_some() && outcome.last_seq.is_some());
    assert!(outcome.report_seq.is_some());

    let replies = log.replies("eng");
    // Every deliberation turn is authored by the teammate that took it, and the
    // one closing row by the reserved, unmintable outcome author.
    let (last_author, last_text) = replies.last().expect("a closing row");
    assert_eq!(last_author, HIVE_REPORT_AUTHOR);
    assert!(last_text.contains("#stage"), "{last_text}");
    assert!(
        replies[..replies.len() - 1]
            .iter()
            .all(|(author, _)| ["planner", "scout", "critic"].contains(&author.as_str())),
        "{replies:?}"
    );
    // One line per turn: what the room counts, never a paragraph that would
    // crowd out the transcript window on the next prompt.
    assert!(
        replies.iter().all(|(_, text)| !text.contains('\n')),
        "{replies:?}"
    );
}

#[tokio::test]
async fn the_blind_round_hides_peers_and_the_prompt_says_so() {
    let log = Arc::new(MemoryLog::default());
    let trigger = seed_desk(&log).await;
    let desk = desk_of(&three_member_manifest(), "eng").expect("a room");
    let runner = ScriptedRunner::new(&[
        ("planner", "!propose #stage Stage the rollout."),
        ("scout", "!propose #ship Ship it all at once."),
        ("critic", "!propose #wait Wait a week."),
    ]);
    EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .run(trigger)
    .await
    .expect("the episode runs");

    let asked = runner.asked();
    assert!(asked.len() >= 3, "{asked:?}");
    // The opening round is blind: the second speaker is told so, and the first
    // speaker's line is not in the transcript it was handed. The operator's own
    // message is — it is the task, and it predates the watermark.
    let (_, second) = &asked[1];
    assert!(
        second.contains("You cannot yet see your peers' positions"),
        "{second}"
    );
    assert!(
        !second.contains("#stage"),
        "a peer's position leaked:\n{second}"
    );
    assert!(second.contains("Decide the rollout."), "{second}");
    // And every seat is told who else is in the room, and what its own id is.
    let (first_agent, first) = &asked[0];
    assert!(
        first.contains(&format!("You are @{first_agent}")),
        "{first}"
    );
    assert!(first.contains("In the room with you: @"), "{first}");
}

#[test]
fn the_marker_line_is_what_the_room_keeps() {
    assert_eq!(
        marker_line("Here is my thinking.\n\n!support #stage ^3 It is reversible.\n\nThanks!"),
        "!support #stage ^3 It is reversible."
    );
    // ANSI escapes a tool's captured output may have left behind.
    assert_eq!(
        marker_line("\u{1b}[32m!propose #ship Ship it.\u{1b}[0m"),
        "!propose #ship Ship it."
    );
    // A turn that deposits no trace is still a legal turn.
    assert_eq!(marker_line("I am not sure yet."), "I am not sure yet.");
    assert_eq!(marker_line("   \n\n"), "(no answer)");
}

// ---------------------------------------------------------------------------
// The episode watermark divider
// ---------------------------------------------------------------------------

/// One transcript row, as the projection hands it to a prompt.
fn message(sequence: u64, author: &str, content: &str) -> tinyhivemind_hive::SessionMessage {
    tinyhivemind_hive::SessionMessage {
        sequence: Sequence(sequence),
        author: SessionAuthor::Agent {
            id: author.to_owned(),
            label: author.to_owned(),
        },
        content: content.to_owned(),
    }
}

/// One authorized turn, for rendering a prompt without driving an episode.
fn hive_turn(
    agent: &str,
    visibility: tinyhivemind_hive::Visibility,
    watermark: Sequence,
) -> tinyhivemind_hive::HiveTurn {
    tinyhivemind_hive::HiveTurn {
        agent_id: agent.to_owned(),
        phase: tinyhivemind_hive::Phase::Deliberate,
        visibility,
        reason: tinyhivemind_hive::BidReason::Salience,
        next_state: tinyhivemind_hive::EpisodeState::opened(
            tinyhivemind_hive::Conversation {
                desk_id: "eng".to_owned(),
                desk_name: "Engineering".to_owned(),
                thread_root: None,
            },
            watermark,
        ),
    }
}

#[test]
fn a_transcript_spanning_the_watermark_renders_the_divider_between_episodes() {
    // The live shape: a prior episode's propose/support pair sits below the
    // trigger, and this episode's own pair sits above it.
    let desk = desk_of(&three_member_manifest(), "eng").expect("a room");
    let member = desk.member("planner").expect("planner is seated").clone();
    let quorum = desk.policy().quorum;
    let visible = [
        message(1, "scout", "!propose #euler249 Old guess."),
        message(2, "scout", "!support #euler249 ^1 Because it is odd."),
        message(3, "planner", "!propose #euler301 New guess."),
        message(4, "critic", "!support #euler301 ^3 Because it holds."),
    ];
    let visible_refs: Vec<&tinyhivemind_hive::SessionMessage> = visible.iter().collect();

    let turn = hive_turn(
        "planner",
        tinyhivemind_hive::Visibility::Full,
        Sequence(0),
    );
    let prompt = EpisodePrompt::new(&member, &desk, "Decide the answer.", quorum, &[])
        .with_trigger(Sequence(2))
        .render(&turn, &visible_refs);

    let prior_end = prompt.find("[2] scout").expect("prior row 2 is rendered");
    let divider_at = prompt
        .find("this episode's floor")
        .expect("the divider names its own meaning");
    let current_start = prompt.find("[3] planner").expect("current row 3 is rendered");
    assert!(prior_end < divider_at, "{prompt}");
    assert!(divider_at < current_start, "{prompt}");
    // Sequence numbers stay exactly as the projection assigned them, on both
    // sides of the divider.
    for needle in ["[1] scout", "[2] scout", "[3] planner", "[4] critic"] {
        assert!(prompt.contains(needle), "{needle} missing:\n{prompt}");
    }
}

#[test]
fn a_transcript_entirely_after_the_trigger_renders_with_no_divider() {
    let desk = desk_of(&three_member_manifest(), "eng").expect("a room");
    let member = desk.member("planner").expect("planner is seated").clone();
    let quorum = desk.policy().quorum;
    let visible = [message(3, "planner", "!propose #stage Stage it.")];
    let visible_refs: Vec<&tinyhivemind_hive::SessionMessage> = visible.iter().collect();
    let turn = hive_turn("planner", tinyhivemind_hive::Visibility::Full, Sequence(0));

    let without_trigger = EpisodePrompt::new(&member, &desk, "Decide the rollout.", quorum, &[])
        .render(&turn, &visible_refs);
    let with_trigger_below_everything =
        EpisodePrompt::new(&member, &desk, "Decide the rollout.", quorum, &[])
            .with_trigger(Sequence(1))
            .render(&turn, &visible_refs);

    assert_eq!(
        without_trigger, with_trigger_below_everything,
        "nothing precedes the watermark, so there is nothing to divide"
    );
    assert!(
        !without_trigger.contains("this episode's floor"),
        "{without_trigger}"
    );
}

#[test]
fn a_blind_turn_still_hides_only_this_episodes_peers_not_prior_context() {
    // [1] predates the trigger and stays visible even blind. [2] is the
    // turn-holder's own line and stays visible for the same reason a blind
    // turn always sees its own work. [3] is a peer's line inside this
    // episode, which a blind opening round must not show.
    let desk = desk_of(&three_member_manifest(), "eng").expect("a room");
    let member = desk.member("planner").expect("planner is seated").clone();
    let quorum = desk.policy().quorum;
    let full = vec![
        message(1, "scout", "!propose #euler249 Old guess."),
        message(2, "planner", "!propose #euler301 My own guess."),
        message(3, "critic", "!propose #euler301-alt A rushed guess."),
    ];
    let turn = hive_turn("planner", tinyhivemind_hive::Visibility::Blind, Sequence(1));
    let visible = tinyhivemind_hive::project_for(&turn, &full);

    let prompt = EpisodePrompt::new(&member, &desk, "Pick a strategy.", quorum, &[])
        .with_trigger(Sequence(1))
        .render(&turn, &visible);

    assert!(
        prompt.contains("[1] scout"),
        "prior context stays visible even blind:\n{prompt}"
    );
    assert!(prompt.contains("[2] planner"), "{prompt}");
    assert!(
        !prompt.contains("[3]"),
        "a peer's live position leaked into a blind turn:\n{prompt}"
    );
    // The roster always names every teammate ("In the room with you: ..."),
    // so the leak this guards against is the peer's own *line*, not its id.
    assert!(
        !prompt.contains("euler301-alt"),
        "a peer's live position leaked into a blind turn:\n{prompt}"
    );
    let prior_end = prompt.find("[1] scout").expect("prior row rendered");
    let divider_at = prompt
        .find("this episode's floor")
        .expect("the divider names its own meaning");
    let current_start = prompt.find("[2] planner").expect("current row rendered");
    assert!(prior_end < divider_at, "{prompt}");
    assert!(divider_at < current_start, "{prompt}");
}
