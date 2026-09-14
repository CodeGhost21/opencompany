//! Tests for the per-member move grammar, desk memory, and speaker diversity.
//!
//! Everything here is scripted through [`HiveTurnRunner`]: no model, no store,
//! no provider. A room whose members are handed exact lines is the only way to
//! assert that a *barred* move is corrected and then demoted — a live room
//! would be asserting the model's compliance rather than this host's
//! enforcement.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::memory::{HiveMemory, HiveMemoryHit, HiveMemoryNote};
use super::test::{MemoryLog, desk_of, record};
use super::*;
use crate::Result;
use crate::ports::events::EventLog;
use crate::ports::types::{CompanyEvent, EventSeq};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A room whose members answer from a per-agent queue, and which may be told to
/// fail one member's turn a fixed number of times.
pub(super) struct Runner {
    lines: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<(String, String)>>,
    /// Agent id → how many of its next turns must fail.
    fail: Mutex<BTreeMap<String, usize>>,
}

impl Runner {
    pub(super) fn new(lines: &[(&str, &str)]) -> Self {
        Self {
            lines: Mutex::new(
                lines
                    .iter()
                    .map(|(id, line)| ((*id).to_owned(), (*line).to_owned()))
                    .collect(),
            ),
            asked: Mutex::new(Vec::new()),
            fail: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn failing(self, agent_id: &str, times: usize) -> Self {
        self.fail
            .lock()
            .expect("poisoned")
            .insert(agent_id.to_owned(), times);
        self
    }

    pub(super) fn asked(&self) -> Vec<(String, String)> {
        self.asked.lock().expect("poisoned").clone()
    }

    /// Every prompt `agent_id` was handed, in order.
    pub(super) fn prompts_for(&self, agent_id: &str) -> Vec<String> {
        self.asked()
            .into_iter()
            .filter(|(id, _)| id == agent_id)
            .map(|(_, prompt)| prompt)
            .collect()
    }
}

#[async_trait]
impl HiveTurnRunner for Runner {
    async fn speak(&self, agent_id: &str, prompt: &str) -> Result<String> {
        self.asked
            .lock()
            .expect("poisoned")
            .push((agent_id.to_owned(), prompt.to_owned()));
        {
            let mut fail = self.fail.lock().expect("poisoned");
            if let Some(left) = fail.get_mut(agent_id)
                && *left > 0
            {
                *left -= 1;
                return Err(crate::error::OpenCompanyError::Config(
                    "turn for 'verifier' hit the harness's per-turn wall-clock ceiling after \
                     10m 00s"
                        .to_owned(),
                ));
            }
        }
        let mut lines = self.lines.lock().expect("poisoned");
        let at = lines.iter().position(|(id, _)| id == agent_id);
        Ok(match at {
            Some(at) => lines.remove(at).1,
            None => format!("!question {agent_id} has nothing further."),
        })
    }
}

/// A memory that answers a fixed set of hits and records every note written.
#[derive(Default)]
struct ScriptedMemory {
    hits: Vec<String>,
    notes: Mutex<Vec<HiveMemoryNote>>,
    /// When set, both halves fail — the best-effort contract under test.
    broken: bool,
}

impl ScriptedMemory {
    fn with_hits(hits: &[&str]) -> Self {
        Self {
            hits: hits.iter().map(|hit| (*hit).to_owned()).collect(),
            ..Self::default()
        }
    }

    fn notes(&self) -> Vec<HiveMemoryNote> {
        self.notes.lock().expect("poisoned").clone()
    }
}

#[async_trait]
impl HiveMemory for ScriptedMemory {
    async fn recall(&self, _query: &str, limit: usize) -> Result<Vec<HiveMemoryHit>> {
        if self.broken {
            return Err(crate::error::OpenCompanyError::Store("no store".to_owned()));
        }
        Ok(self
            .hits
            .iter()
            .take(limit)
            .map(|snippet| HiveMemoryHit {
                snippet: snippet.clone(),
            })
            .collect())
    }

    async fn remember(&self, note: HiveMemoryNote) -> Result<()> {
        if self.broken {
            return Err(crate::error::OpenCompanyError::Store("no store".to_owned()));
        }
        self.notes.lock().expect("poisoned").push(note);
        Ok(())
    }
}

/// Three teammates, with `moves` assigned per the caller's TOML fragment.
pub(super) fn manifest_with(hive: &str) -> String {
    format!(
        "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"planner\"\nrole = \"Planner\"\n\
         [[agent]]\nid = \"scout\"\nrole = \"Scout\"\n\
         [[agent]]\nid = \"critic\"\nrole = \"Critic\"\n\
         [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\n\
         description = \"Ship the rollout\"\n\
         members = [\"planner\", \"scout\", \"critic\"]\n\
         {hive}\n"
    )
}

/// The operator's message, and the watermark the episode opens on.
pub(super) async fn open(log: &MemoryLog) -> EventSeq {
    log.append(
        &MemoryLog::company(),
        CompanyEvent::OperatorMessage {
            text: "Decide the rollout.".into(),
            by: None,
            chat: Some("eng".into()),
            parent: None,
            deliverable: None,
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
    )
    .await
    .expect("the journal accepts the operator's message")
}

// ---------------------------------------------------------------------------
// The grammar itself
// ---------------------------------------------------------------------------

#[test]
fn an_unnamed_member_keeps_every_move() {
    let config = HiveConfig::default();
    assert_eq!(config.moves_for("planner"), MOVE_KINDS.to_vec());
    assert!(config.may("planner", "propose"));
    // And so does a member named with an empty list: a table somebody started
    // and never filled in must not silence a seat.
    let mut moves = BTreeMap::new();
    moves.insert("planner".to_owned(), Vec::new());
    let config = HiveConfig {
        moves,
        ..HiveConfig::default()
    };
    assert_eq!(config.moves_for("planner"), MOVE_KINDS.to_vec());
}

#[test]
fn a_line_kind_reads_the_marker_and_folds_unpin_onto_pin() {
    assert_eq!(moves::line_kind("!support #a ^2 why"), Some("support"));
    assert_eq!(moves::line_kind("  !propose #a x"), Some("propose"));
    assert_eq!(moves::line_kind("!unpin ^2"), Some("pin"));
    assert_eq!(moves::line_kind("!shout at everyone"), None);
    assert_eq!(moves::line_kind("no marker here"), None);
}

#[test]
fn demoting_keeps_the_words_and_loses_the_trace() {
    let demoted = moves::demote("!propose #ship Ship it all at once.");
    assert_eq!(demoted, "propose #ship Ship it all at once.");
    // The property the whole mechanism turns on: the fold reads nothing off it.
    let traces = tinyhivemind_hive::trace::resolve(
        &demoted,
        None,
        &tinyhivemind_hive::SessionAuthor::Agent {
            id: "planner".into(),
            label: "planner".into(),
        },
        tinyhivemind_hive::Sequence(4),
    );
    assert!(traces.is_empty(), "a demoted line must fold to nothing");
}

#[tokio::test]
async fn a_forbidden_move_is_re_prompted_once_and_then_demoted() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = manifest_with(
        "hive = { turn_budget = 4, quorum = 2, blind_round = false, \
         moves = { scout = [\"support\", \"evidence\", \"question\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    // Scout may not propose, and tries twice.
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage the rollout."),
        ("scout", "!propose #ship Ship it all at once."),
        ("scout", "!propose #ship I still say ship it."),
        ("critic", "!question What broke last time?"),
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
    .expect("a barred move never fails the episode");

    // Asked twice, and the second prompt carries the one-line correction.
    let prompts = runner.prompts_for("scout");
    assert!(prompts.len() >= 2, "scout must be re-prompted: {prompts:?}");
    assert!(
        prompts[1].contains("You may not `!propose`"),
        "the correction names the move: {}",
        prompts[1]
    );
    assert!(prompts[1].contains("!support"), "{}", prompts[1]);

    // The second attempt is journaled with its marker stripped.
    let replies = log.replies("eng");
    assert!(
        replies
            .iter()
            .any(|(author, text)| author == "scout" && text == "propose #ship I still say ship it."),
        "{replies:?}"
    );
    assert!(
        !replies
            .iter()
            .any(|(author, text)| author == "scout" && text.starts_with("!propose")),
        "no barred proposal may keep its marker: {replies:?}"
    );

    // And the operator is told, in the closing row.
    assert_eq!(outcome.violations.len(), 1, "{:?}", outcome.violations);
    assert_eq!(outcome.violations[0].agent_id, "scout");
    assert_eq!(outcome.violations[0].attempted, "propose");
    assert!(
        outcome.summary().contains("@scout !propose"),
        "{}",
        outcome.summary()
    );
}

#[tokio::test]
async fn a_corrected_member_that_complies_is_not_recorded_as_a_violation() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = manifest_with(
        "hive = { turn_budget = 3, quorum = 2, blind_round = false, \
         moves = { scout = [\"support\", \"evidence\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage the rollout."),
        ("scout", "!propose #ship Ship it."),
        (
            "scout",
            "!evidence #stage ^1 The last rollout took checkout down.",
        ),
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
    assert!(outcome.violations.is_empty(), "{:?}", outcome.violations);
    let replies = log.replies("eng");
    assert!(
        replies
            .iter()
            .any(|(author, text)| author == "scout" && text.starts_with("!evidence")),
        "{replies:?}"
    );
}

#[tokio::test]
async fn the_prompt_renders_only_the_moves_a_member_has() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = manifest_with(
        "hive = { turn_budget = 2, blind_round = false, \
         moves = { planner = [\"propose\", \"commit\"], scout = [\"object\", \"refute\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!object >1 ^1 That is slower."),
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

    let planner = runner.prompts_for("planner");
    let planner = planner.first().expect("planner spoke");
    assert!(planner.contains("!propose #topic"), "{planner}");
    // `commit` is phase-gated on top of the grammar, so its move line is absent
    // while the room deliberates even though the seat holds it. (The rules
    // block still names the marker, to say not to write it.)
    assert!(!planner.contains("!commit #topic ^N"), "{planner}");
    assert!(!planner.contains("!support #topic"), "{planner}");
    assert!(
        planner.contains("These are the ONLY markers this desk gives you"),
        "an assigned seat is told the grammar is assigned: {planner}"
    );

    let scout = runner.prompts_for("scout");
    let scout = scout.first().expect("scout spoke");
    assert!(scout.contains("!object >N ^M"), "{scout}");
    assert!(scout.contains("!refute #topic ^N"), "{scout}");
    assert!(!scout.contains("!propose #topic"), "{scout}");
}

// ---------------------------------------------------------------------------
// The quorum knobs
// ---------------------------------------------------------------------------

#[test]
fn require_evidential_and_the_caps_reach_the_policy() {
    let config = HiveConfig {
        require_evidential: Some(true),
        refutation_cap: Some(2),
        dominance_cap: Some(4),
        repetition_cap: Some(1),
        ..HiveConfig::default()
    };
    let policy = HivePolicy::from_config(&config, 3).episode;
    assert!(policy.quorum.require_evidential);
    assert!(
        policy.quorum.require_grounded,
        "require_evidential implies require_grounded"
    );
    assert_eq!(policy.quorum.refutation_cap, Some(2));
    assert_eq!(policy.dominance_cap, 4);
    assert_eq!(policy.repetition_cap, 1);
    // The defaults are the library's own, unchanged.
    let default = HivePolicy::from_config(&HiveConfig::default(), 3).episode;
    assert!(!default.quorum.require_evidential);
    assert_eq!(default.quorum.refutation_cap, None);
    assert_eq!(default.dominance_cap, 50);
    assert_eq!(default.repetition_cap, 3);
}

#[tokio::test]
async fn under_require_evidential_a_proposal_plus_an_evidential_support_carries() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    // One proposer, and support that has to reach an `!evidence` to count.
    let manifest = manifest_with(
        "hive = { turn_budget = 9, quorum = 2, blind_round = false, require_evidential = true, \
         moves = { scout = [\"evidence\", \"support\", \"commit\", \"question\"], \
         critic = [\"evidence\", \"support\", \"object\", \"commit\", \"question\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        (
            "planner",
            "!propose #stage Stage the rollout behind a flag.",
        ),
        (
            "scout",
            "!evidence #stage ^1 The last full rollout took checkout down for 40 minutes.",
        ),
        (
            "critic",
            "!support #stage ^3 The outage is the reason to stage.",
        ),
        ("planner", "!commit #stage ^3 Recorded."),
        ("scout", "!commit #stage ^3 Recorded."),
        ("critic", "!commit #stage ^3 Recorded."),
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
        "an evidential chain carries: {outcome:?}"
    );
}

/// **The evidential retry is graded against the seat's moves too.**
///
/// `line_from` enforces `moves_for` on a member's first reply and on the
/// move-correction retry, then hands whatever clears that off to `grounded`,
/// which may send the member back once more for citation discipline. Before
/// this test's fix, whatever `grounded`'s retry answered was journaled
/// as-is with no re-check at all — a member could answer the
/// citation-correction prompt with a marker kind its seat is not entitled to
/// make, and it folded into the transcript as a legitimate move. Here critic
/// may not `!propose`; its first `!support` cites the proposal rather than the
/// evidence line (missing evidence, so `grounded` retries it), and the retry
/// answer switches straight to `!propose`.
#[tokio::test]
async fn an_evidential_retry_is_still_graded_against_the_seats_moves() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = manifest_with(
        "hive = { turn_budget = 9, quorum = 2, blind_round = false, require_evidential = true, \
         moves = { scout = [\"evidence\", \"support\", \"commit\", \"question\"], \
         critic = [\"evidence\", \"support\", \"object\", \"commit\", \"question\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        (
            "planner",
            "!propose #stage Stage the rollout behind a flag.",
        ),
        (
            "scout",
            "!evidence #stage ^1 The last full rollout took checkout down for 40 minutes.",
        ),
        (
            "critic",
            // Cites the proposal (^1), not the evidence (^2) — misses evidence,
            // which sends critic back for the evidential retry.
            "!support #stage ^1 The proposal alone is reason enough.",
        ),
        (
            "critic",
            // The evidential retry's answer: a move barred for this seat.
            "!propose #rush Ship immediately without staging.",
        ),
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
    let replies = log.replies("eng");
    let critic_retry = replies
        .iter()
        .find(|(author, text)| author == "critic" && text.contains("Ship immediately"))
        .map(|(_, text)| text.clone())
        .expect("critic's evidential retry answer is journaled somewhere");
    assert!(
        !critic_retry.starts_with('!'),
        "a barred move on the evidential retry must be demoted, not journaled as a legitimate \
         move: {critic_retry:?} (outcome: {outcome:?})"
    );
    assert_eq!(
        outcome.violations.len(),
        1,
        "the evidential retry's barred move must be reported as a seat violation too: {outcome:?}"
    );
}

#[tokio::test]
async fn two_bare_proposals_of_one_topic_do_not_carry_when_only_one_seat_may_propose() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    // The live failure, in miniature: two members both `!propose #answer`, and
    // in tinyhivemind a proposal counts as its author's own support — so
    // without a move grammar the topic reaches a quorum of two on nobody having
    // read anybody. Here only planner may propose, so scout's proposal is
    // demoted and deposits nothing.
    // `scout` holds `support` so the desk can reach its quorum of two at all —
    // `desk_episode` now declines a room whose eligible supporters are fewer
    // than its quorum, and `manifest.rs` refuses the same shape outright. That
    // is orthogonal to what this test asserts: `scout` still may not
    // `!propose`, which is the whole claim, and the script never has it
    // `!support` anything, so the topic still carries nothing.
    let manifest = manifest_with(
        "hive = { turn_budget = 4, quorum = 2, blind_round = false, \
         moves = { scout = [\"question\", \"evidence\", \"support\"], \
         critic = [\"question\", \"evidence\"] } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #answer 233168"),
        ("scout", "!propose #answer 233168"),
        ("scout", "!propose #answer 233168"),
        ("critic", "!question Has anybody checked the bound?"),
        ("planner", "!question Anybody?"),
    ]);
    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Sum the multiples of 3 or 5 below 1000.",
    )
    .run(trigger)
    .await
    .expect("the episode runs");
    assert!(
        !matches!(outcome.ending, EpisodeEnding::Converged { .. }),
        "a demoted second proposal must not complete a quorum: {outcome:?}"
    );
    assert_eq!(outcome.violations.len(), 1, "{:?}", outcome.violations);
}

// ---------------------------------------------------------------------------
// Desk memory
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_recall_block_reaches_every_prompt_of_the_episode() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 2, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let memory = Arc::new(ScriptedMemory::with_hits(&[
        "Decide the rollout — #stage\n\nCarried: #stage\nSupporters: planner, scout",
    ]));
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!support #stage ^1 Agreed."),
    ]);
    EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .with_memory(Arc::clone(&memory) as Arc<dyn HiveMemory>)
    .run(trigger)
    .await
    .expect("the episode runs");

    let asked = runner.asked();
    assert!(!asked.is_empty());
    for (agent, prompt) in &asked {
        assert!(
            prompt.contains("The desk remembers:"),
            "{agent} was not shown the recall block:\n{prompt}"
        );
        assert!(prompt.contains("Carried: #stage"), "{prompt}");
        // Attributed as memory, never as a transcript line: it carries no
        // sequence, so nothing can cite it.
        assert!(
            prompt.contains("cannot be cited with ^"),
            "the block must say what it is: {prompt}"
        );
    }
}

#[tokio::test]
async fn a_converged_episode_writes_exactly_one_note_with_the_topic_and_the_evidence() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 9, quorum = 2, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let memory = Arc::new(ScriptedMemory::default());
    let runner = Runner::new(&[
        (
            "planner",
            "!propose #stage Stage the rollout behind a flag.",
        ),
        (
            "critic",
            "!evidence #stage ^1 The last full rollout took checkout down.",
        ),
        (
            "scout",
            "!support #stage ^3 Staging bounds the blast radius.",
        ),
        ("planner", "!pin ^3 Keep the outage on the board."),
        ("critic", "!commit #stage ^3 The room settled on staging."),
        ("scout", "!commit #stage ^3 Recorded."),
        ("planner", "!commit #stage ^3 Recorded."),
    ]);
    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.\nSecond line the note must not carry.",
    )
    .with_memory(Arc::clone(&memory) as Arc<dyn HiveMemory>)
    .run(trigger)
    .await
    .expect("the episode runs");
    assert!(matches!(outcome.ending, EpisodeEnding::Converged { .. }));

    let notes = memory.notes();
    assert_eq!(notes.len(), 1, "exactly one note per episode: {notes:?}");
    let note = &notes[0];
    assert_eq!(note.desk_id, "eng");
    assert!(note.title.starts_with("Decide the rollout."), "{note:?}");
    assert!(!note.title.contains("Second line"), "{note:?}");
    assert!(note.body.contains("Carried: #stage"), "{}", note.body);
    assert!(note.body.contains("Supporters:"), "{}", note.body);
    assert!(
        note.body
            .contains("!evidence #stage ^1 The last full rollout"),
        "the evidence lines are the point of the note:\n{}",
        note.body
    );
    assert!(note.body.contains("Pinned:"), "{}", note.body);
    assert!(note.body.contains("Committed:"), "{}", note.body);
    // And the label it lands under is desk-scoped.
    assert!(note_label(&note.desk_id, &note.title).starts_with("hive/eng/"));
}

#[tokio::test]
async fn an_unresolved_episode_writes_a_short_note_naming_the_competing_topics() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 3, quorum = 3, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let memory = Arc::new(ScriptedMemory::default());
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!propose #ship Ship it."),
        ("critic", "!question Which is reversible?"),
    ]);
    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .with_memory(Arc::clone(&memory) as Arc<dyn HiveMemory>)
    .run(trigger)
    .await
    .expect("the episode runs");
    assert!(!matches!(outcome.ending, EpisodeEnding::Converged { .. }));
    let notes = memory.notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].title.starts_with("Unresolved:"), "{:?}", notes[0]);
    assert!(notes[0].body.contains("#stage"), "{}", notes[0].body);
    assert!(notes[0].body.contains("#ship"), "{}", notes[0].body);
}

#[tokio::test]
async fn a_broken_memory_never_fails_the_episode() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 2, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let memory = Arc::new(ScriptedMemory {
        broken: true,
        ..ScriptedMemory::default()
    });
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!support #stage ^1 Agreed."),
    ]);
    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .with_memory(Arc::clone(&memory) as Arc<dyn HiveMemory>)
    .run(trigger)
    .await
    .expect("a store that is down is not a reason to refuse the operator");
    assert!(outcome.turns >= 1, "{outcome:?}");
    let asked = runner.asked();
    assert!(
        asked
            .iter()
            .all(|(_, prompt)| !prompt.contains("The desk remembers:")),
        "a failed recall renders no block"
    );
}

// ---------------------------------------------------------------------------
// Speaker diversity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_unspoken_line_appears_when_the_floor_comes_straight_back() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    // `dominance_cap` and `repetition_cap` are the library's own damping; this
    // is the host's addition on top, and it never overrides the fold. A room of
    // one speaker is forced by giving everybody but the planner nothing to say
    // and letting the market hand the floor back.
    let manifest = manifest_with(
        "hive = { turn_budget = 6, quorum = 3, blind_round = false, dominance_cap = 50 }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[]);
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

    // Whenever the same member is asked twice in a row while somebody has still
    // not spoken, that second prompt names the missing members.
    let asked = runner.asked();
    let repeated: Vec<usize> = asked
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0].0 == pair[1].0)
        .map(|(index, _)| index + 1)
        .collect();
    if repeated.is_empty() {
        // The fold rotated the floor on its own, which is the outcome the line
        // exists to encourage — nothing to assert, and nothing wrong.
        return;
    }
    let spoke_before: Vec<String> = asked[..repeated[0]]
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    let everybody_spoke = ["planner", "scout", "critic"]
        .iter()
        .all(|id| spoke_before.iter().any(|seen| seen == id));
    if everybody_spoke {
        return;
    }
    let (_, prompt) = &asked[repeated[0]];
    assert!(
        prompt.contains("Members who have not spoken yet:"),
        "a repeated speaker must be told who is missing:\n{prompt}"
    );
    assert!(prompt.contains("!defer #topic"), "{prompt}");
}

#[test]
fn the_unspoken_block_is_rendered_from_the_builder() {
    // The rendering itself, asserted without depending on the market handing
    // the floor back — which is the library's decision, not this host's.
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 4, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let unspoken = vec!["scout".to_owned(), "critic".to_owned()];
    let prompt = EpisodePrompt::new(
        &desk.members[0],
        &desk,
        "Decide the rollout.",
        desk.policy().quorum,
        &[],
    )
    .with_unspoken(&unspoken);
    let turn = tinyhivemind_hive::HiveTurn {
        agent_id: "planner".into(),
        phase: tinyhivemind_hive::Phase::Deliberate,
        visibility: tinyhivemind_hive::Visibility::Full,
        reason: tinyhivemind_hive::BidReason::Salience,
        next_state: tinyhivemind_hive::EpisodeState::opened(
            tinyhivemind_hive::Conversation {
                desk_id: "eng".into(),
                desk_name: "Engineering".into(),
                thread_root: None,
            },
            tinyhivemind_hive::Sequence(1),
        ),
    };
    let rendered = prompt.render(&turn, &[]);
    assert!(
        rendered.contains("Members who have not spoken yet: @scout, @critic"),
        "{rendered}"
    );
}

// ---------------------------------------------------------------------------
// A failed turn is not a failed room
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_members_failed_turn_does_not_end_the_episode() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 9, quorum = 2, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage the rollout."),
        (
            "critic",
            "!evidence #stage ^1 The last rollout took checkout down.",
        ),
        ("scout", "!support #stage ^3 Staging bounds the damage."),
        ("planner", "!commit #stage ^3 Recorded."),
        ("critic", "!commit #stage ^3 Recorded."),
        ("scout", "!commit #stage ^3 Recorded."),
    ])
    .failing("scout", 1);

    let outcome = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .run(trigger)
    .await
    .expect("one member's turn timing out must not throw away the room's work");

    assert_eq!(outcome.failed_turns, 1, "{outcome:?}");
    assert!(
        matches!(&outcome.ending, EpisodeEnding::Converged { topic, .. } if topic == "stage"),
        "{outcome:?}"
    );
    // The miss is on the desk, authored by the room rather than by the member,
    // and carries no marker — so the fold reads nothing off it.
    let replies = log.replies("eng");
    let note = replies
        .iter()
        .find(|(author, text)| author == HIVE_FAILURE_AUTHOR && text.contains("did not finish"))
        .expect("the miss is journaled");
    assert!(note.1.contains("@scout's turn did not finish"), "{note:?}");
    assert!(note.1.contains("wall-clock ceiling"), "{note:?}");
    assert!(!note.1.trim_start().starts_with('!'), "{note:?}");
    assert!(
        outcome.summary().contains("1 turn did not finish"),
        "{}",
        outcome.summary()
    );
}

#[tokio::test]
async fn a_room_where_every_seat_fails_twice_over_stops() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let desk = desk_of(
        &manifest_with("hive = { turn_budget = 12, blind_round = false }"),
        "eng",
    )
    .expect("a room");
    let runner = Runner::new(&[])
        .failing("planner", 99)
        .failing("scout", 99)
        .failing("critic", 99);
    let failed = EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .run(trigger)
    .await;
    let error = failed.expect_err("a harness that is down is not a room having a bad turn");
    assert!(
        error.to_string().contains("turns in a row failed"),
        "{error}"
    );
    // Six failures for a desk of three: the cap is members x 2, not the budget.
    assert_eq!(
        log.replies("eng")
            .iter()
            .filter(|(author, _)| author == HIVE_FAILURE_AUTHOR)
            .count(),
        6,
        "the cap is members x 2"
    );
}

// ---------------------------------------------------------------------------
// Manifest validation
// ---------------------------------------------------------------------------

#[test]
fn validation_rejects_an_unknown_move_kind_and_an_unknown_member() {
    let problems = record(&manifest_with(
        "hive = { moves = { scout = [\"support\", \"shout\"] } }",
    ))
    .manifest
    .validate();
    assert!(
        problems.iter().any(|problem| problem.contains("shout")),
        "{problems:?}"
    );

    let problems = record(&manifest_with(
        "hive = { moves = { nobody = [\"support\"] } }",
    ))
    .manifest
    .validate();
    assert!(
        problems
            .iter()
            .any(|problem| problem.contains("`nobody`") && problem.contains("not a member")),
        "{problems:?}"
    );
}

#[test]
fn a_table_that_names_nobody_for_commit_is_accepted() {
    // `commit` is not a gated move: the fold hands the Commit phase to whoever
    // the attention market picks, so a desk that could bar a seat from
    // recording a decision would reach quorum and then have nothing legal to
    // say. A table naming no committer is therefore an ordinary table.
    let problems = record(&manifest_with(
        "hive = { moves = { planner = [\"propose\"], scout = [\"support\"], \
         critic = [\"object\"] } }",
    ))
    .manifest
    .validate();
    assert!(
        !problems.iter().any(|problem| problem.contains("!commit")),
        "{problems:?}"
    );
    // And a table that does name one is accepted too — the entry describes
    // what the seat could already do, so it is ignored rather than refused.
    let problems = record(&manifest_with(
        "hive = { moves = { planner = [\"propose\", \"commit\"], scout = [\"support\"], \
         critic = [\"object\"] } }",
    ))
    .manifest
    .validate();
    assert!(
        !problems.iter().any(|problem| problem.contains("!commit")),
        "{problems:?}"
    );
}

#[test]
fn validation_rejects_a_zero_cap() {
    for key in ["dominance_cap", "repetition_cap", "refutation_cap"] {
        let problems = record(&manifest_with(&format!("hive = {{ {key} = 0 }}")))
            .manifest
            .validate();
        assert!(
            problems
                .iter()
                .any(|problem| problem.contains(&format!("hive.{key} = 0"))),
            "{key}: {problems:?}"
        );
    }
}

#[test]
fn the_manifest_round_trips_the_new_keys() {
    let desk = desk_of(
        &manifest_with(
            "hive = { require_evidential = true, refutation_cap = 2, dominance_cap = 4, \
             repetition_cap = 1, moves = { scout = [\"support\", \"evidence\"] } }",
        ),
        "eng",
    )
    .expect("a room");
    assert_eq!(desk.config.require_evidential, Some(true));
    assert_eq!(desk.config.refutation_cap, Some(2));
    assert_eq!(desk.config.dominance_cap, Some(4));
    assert_eq!(desk.config.repetition_cap, Some(1));
    // The declared kinds, plus the three no table can take away.
    assert_eq!(
        desk.config.moves_for("scout"),
        vec!["support", "evidence", "question", "defer", "commit"]
    );
    assert_eq!(desk.config.moves_for("planner"), MOVE_KINDS.to_vec());
}
