//! What a private aside does to a real episode.
//!
//! [`super::aside`]'s own tests pin the fold — the party key, the budget, the
//! settlement debt — against synthetic transcripts. These drive the whole
//! [`EpisodeDriver`] and assert the two properties that only show up end to
//! end: that an authorized aside is journaled with a narrower audience, and
//! that it is worth **nothing** to the room's counting.
//!
//! See `docs/spec/runtime/hivemind-asides.md`.

use std::sync::Arc;

use super::test::{MemoryLog, ScriptedRunner, desk_of, seed_desk};
use super::*;
use crate::ports::events::EventLog;

/// Three seats on one desk with asides on, at the library's own default bounds.
fn aside_manifest() -> String {
    "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"planner\"\nrole = \"Planner\"\n\
     [[agent]]\nid = \"scout\"\nrole = \"Scout\"\n\
     [[agent]]\nid = \"critic\"\nrole = \"Critic\"\n\
     [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\n\
     description = \"Ship the rollout\"\n\
     members = [\"planner\", \"scout\", \"critic\"]\n\
     hive = { enabled = true, aside = { enabled = true } }\n"
        .to_string()
}

/// The same desk with the block absent entirely — the default every other
/// bundle in this repo runs under.
fn no_aside_manifest() -> String {
    "[company]\nname = \"Acme\"\n\
     [[agent]]\nid = \"planner\"\nrole = \"Planner\"\n\
     [[agent]]\nid = \"scout\"\nrole = \"Scout\"\n\
     [[agent]]\nid = \"critic\"\nrole = \"Critic\"\n\
     [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\n\
     description = \"Ship the rollout\"\n\
     members = [\"planner\", \"scout\", \"critic\"]\n"
        .to_string()
}

async fn run(manifest: &str, script: &[(&str, &str)]) -> (Arc<MemoryLog>, EpisodeOutcome) {
    let log = Arc::new(MemoryLog::default());
    let trigger = seed_desk(&log).await;
    let desk = desk_of(manifest, "eng").expect("a room");
    let runner = ScriptedRunner::new(script);
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
    (log, outcome)
}

/// The audience journaled on the first row whose text starts with `!aside`.
fn aside_audience(log: &MemoryLog) -> Option<Vec<String>> {
    aside_audiences(log).into_iter().next()
}

/// Every `!aside` row's audience, in journal order. An empty entry is a line
/// that asked for an aside and was refused into the open.
fn aside_audiences(log: &MemoryLog) -> Vec<Vec<String>> {
    log.addressed_replies("eng")
        .into_iter()
        .filter(|(_, text, _)| text.starts_with("!aside"))
        .map(|(_, _, audience)| audience)
        .collect()
}

#[tokio::test]
async fn an_authorized_aside_is_journaled_to_its_addressee_only() {
    let (log, _) = run(
        &aside_manifest(),
        &[
            ("planner", "!aside @scout is the checkout metric yours?"),
            ("scout", "!propose #ship Ship it all at once."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    assert_eq!(
        aside_audience(&log).as_deref(),
        Some(["scout".to_owned()].as_slice()),
        "the addressee, and not the author, is what the row carries",
    );
}

/// Every other row on the desk stays desk-visible. An aside must not be
/// contagious: the mechanism is one line at a time, not a mode the desk enters.
#[tokio::test]
async fn only_the_aside_row_is_narrowed() {
    let (log, _) = run(
        &aside_manifest(),
        &[
            ("planner", "!aside @scout is the checkout metric yours?"),
            ("scout", "!propose #ship Ship it all at once."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    for (author, text, audience) in log.addressed_replies("eng") {
        if text.starts_with("!aside") {
            continue;
        }
        assert!(
            audience.is_empty(),
            "`{author}` wrote a narrowed row it did not ask to narrow: {text}",
        );
    }
}

/// The rule the whole mechanism rests on. A `!support` written privately adds
/// no supporter, so a room that could otherwise carry an option does not.
///
/// The desk's quorum is two. The script deposits evidence, then a private
/// support and a public one — two supports in the transcript, only one of which
/// the fold may count — so an episode that converged here would be one that had
/// counted a vote nobody in the room could read.
#[tokio::test]
async fn a_support_written_inside_an_aside_carries_nothing() {
    let script: &[(&str, &str)] = &[
        (
            "planner",
            "!propose #stage Stage the rollout behind a flag.",
        ),
        (
            "critic",
            "!evidence #stage ^3 The last full rollout broke checkout.",
        ),
        ("scout", "!aside @planner !support #stage ^4 I am with you."),
        ("critic", "!question does anybody else hold this?"),
    ];
    let (private, private_outcome) = run(&aside_manifest(), script).await;
    assert!(
        aside_audience(&private).is_some(),
        "the fixture only means anything if the aside was authorized",
    );
    assert!(
        !matches!(private_outcome.ending, EpisodeEnding::Converged { .. }),
        "a private support carried the room: {private_outcome:?}",
    );

    // The identical script on a desk that never enabled asides. The same line
    // is an ordinary desk row there, its `!support` counts, and the difference
    // between the two runs is attributable to the audience and to nothing else.
    let (public, _) = run(&no_aside_manifest(), script).await;
    assert_eq!(
        aside_audience(&public),
        Some(Vec::new()),
        "with asides off the same line is journaled desk-visible",
    );
}

/// A line naming a peer who is not on this desk authorizes nothing, and the
/// line still lands where the room can read it. Failing open to *private*
/// would be the one direction that leaks.
#[tokio::test]
async fn an_aside_naming_a_stranger_stays_desk_visible() {
    let (log, _) = run(
        &aside_manifest(),
        &[
            ("planner", "!aside @auditor can you check this?"),
            ("scout", "!propose #ship Ship it."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    assert_eq!(
        aside_audience(&log),
        Some(Vec::new()),
        "`auditor` is on no desk here, so the audience resolved to nobody",
    );
}

/// `must_surface` is the bound that actually binds first.
///
/// With it on — the default — a pair that has not paid its last aside back to
/// the room cannot open another, and `max_messages` is never reached. The
/// refusal is *into the open*: the member has still said what it meant to say,
/// and a line the room can read is never a leak.
#[tokio::test]
async fn a_pair_that_owes_the_room_a_settlement_cannot_open_another_aside() {
    let (log, _) = run(
        &aside_manifest(),
        &[
            ("planner", "!aside @scout first question"),
            ("planner", "!aside @scout second, still owing"),
            ("scout", "!propose #ship Ship it."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    let asides = aside_audiences(&log);
    assert!(asides.len() >= 2, "the script wrote two: {asides:?}");
    assert!(!asides[0].is_empty(), "the first opens the aside");
    assert!(
        asides[1].is_empty(),
        "the pair owed a settlement, so the second belongs to the room: {asides:?}",
    );
}

/// And a `!surface` discharges the debt, so the pair may open another. This is
/// the half that makes `must_surface` a protocol rather than a one-shot limit.
#[tokio::test]
async fn a_settlement_lets_the_pair_open_another_aside() {
    let (log, _) = run(
        &aside_manifest(),
        &[
            ("planner", "!aside @scout first question"),
            ("planner", "!surface scout confirms the metric is theirs"),
            ("planner", "!aside @scout second question"),
            ("scout", "!propose #ship Ship it."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    let asides = aside_audiences(&log);
    assert!(asides.len() >= 2, "the script wrote two: {asides:?}");
    assert!(!asides[0].is_empty(), "the first opens the aside");
    assert!(
        !asides[1].is_empty(),
        "the surface settled it, so the second is authorized again: {asides:?}",
    );
}

/// With `must_surface` off, `max_messages` is what stops a pair. Two rows —
/// a question and an answer — and the third is desk-visible.
#[tokio::test]
async fn a_pair_that_spends_max_messages_is_pushed_back_into_the_open() {
    let manifest = "[company]\nname = \"Acme\"\n\
         [[agent]]\nid = \"planner\"\nrole = \"Planner\"\n\
         [[agent]]\nid = \"scout\"\nrole = \"Scout\"\n\
         [[agent]]\nid = \"critic\"\nrole = \"Critic\"\n\
         [[group_chat]]\nid = \"eng\"\nname = \"Engineering\"\n\
         description = \"Ship the rollout\"\n\
         members = [\"planner\", \"scout\", \"critic\"]\n\
         hive = { enabled = true, aside = { enabled = true, max_messages = 2, must_surface = false } }\n";

    let (log, _) = run(
        manifest,
        &[
            ("planner", "!aside @scout first question"),
            ("planner", "!aside @scout second, the last within budget"),
            ("planner", "!aside @scout third, over budget"),
            ("scout", "!propose #ship Ship it."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    let asides = aside_audiences(&log);
    assert!(asides.len() >= 3, "the script wrote three: {asides:?}");
    assert!(!asides[0].is_empty(), "the first is within budget");
    assert!(!asides[1].is_empty(), "the second is within budget");
    assert!(
        asides[2].is_empty(),
        "the third spent the pair's budget and belongs to the room: {asides:?}",
    );
}

/// A desk that never enabled asides behaves byte-for-byte as it did before the
/// mechanism existed: the marker is just text, and every row is desk-visible.
#[tokio::test]
async fn a_desk_that_did_not_opt_in_narrows_nothing() {
    let (log, _) = run(
        &no_aside_manifest(),
        &[
            ("planner", "!aside @scout is the checkout metric yours?"),
            ("scout", "!propose #ship Ship it."),
            ("critic", "!propose #stage Stage it."),
        ],
    )
    .await;

    assert!(
        log.addressed_replies("eng")
            .iter()
            .all(|(_, _, audience)| audience.is_empty()),
        "an opted-out desk journaled a narrowed row",
    );
}

/// The two markers are taught only where they can be used. A grammar is a fixed
/// cost paid in every agent's system text on every turn, and teaching a move
/// nobody may make spends that budget for nothing.
#[tokio::test]
async fn the_grammar_is_taught_only_to_a_desk_that_enabled_it() {
    for (manifest, expected) in [(aside_manifest(), true), (no_aside_manifest(), false)] {
        let log = Arc::new(MemoryLog::default());
        let trigger = seed_desk(&log).await;
        let desk = desk_of(&manifest, "eng").expect("a room");
        let runner = ScriptedRunner::new(&[("planner", "!propose #stage Stage it.")]);
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

        let taught = runner
            .asked()
            .iter()
            .any(|(_, prompt)| prompt.contains(ASIDE_MARKER) && prompt.contains(SURFACE_MARKER));
        assert_eq!(taught, expected, "aside grammar taught={taught}");
    }
}
