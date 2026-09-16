//! Tests for cross-desk referral: what crosses, what does not, and what a
//! host owes the library.
//!
//! Everything here is scripted through [`HiveTurnRunner`] and
//! [`HiveReferralRunner`] — no model, no store, no provider — because the
//! properties under test are this host's, not a model's. Whether a room asks a
//! good question is a model's business; whether the answer lands on the right
//! desk, under an author that cannot be counted as a supporter, and only when
//! the desk opted in, is entirely ours.

use std::sync::Arc;


use super::moves_fixtures_tests::Runner;
use super::referral;
use super::referral_fixtures_tests::*;
use super::test::{MemoryLog, desk_of, record};
use super::*;
use crate::ports::events::EventLog;

// ---------------------------------------------------------------------------
// The prompt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_referring_desk_is_shown_its_peers_and_told_to_ask_early() {
    let far = FarDesk::answering("Answered.");
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = two_desks(REFERRING);
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!support #stage ^1 Fine."),
        ("critic", "!commit #stage ^1 Recorded."),
    ]);
    let federation = desk_federation(&record(&manifest), &desk).expect("a federation");
    EpisodeDriver::new(
        MemoryLog::company(),
        desk,
        Arc::clone(&log) as Arc<dyn EventLog>,
        &runner,
        "Decide the rollout.",
    )
    .with_federation(federation, &far)
    .run(trigger)
    .await
    .expect("the episode runs");

    let prompt = runner
        .prompts_for("planner")
        .into_iter()
        .next()
        .expect("planner spoke");
    assert!(
        prompt.contains("@#platform (Platform) — Owns the database and the edge"),
        "the seat is shown the desk it may ask and what that desk is for:\n{prompt}"
    );
    assert!(
        prompt.contains("EARLY"),
        "and told when to ask, which is the largest measured effect in the mechanism:\n{prompt}"
    );
    assert!(
        prompt.contains("it is not a vote"),
        "and told what an answer is worth:\n{prompt}"
    );
}

#[tokio::test]
async fn a_desk_without_referral_is_shown_no_peer_block_at_all() {
    let log = Arc::new(MemoryLog::default());
    let trigger = open(&log).await;
    let manifest = two_desks(PLAIN);
    let desk = desk_of(&manifest, "eng").expect("a room");
    let runner = Runner::new(&[
        ("planner", "!propose #stage Stage it."),
        ("scout", "!support #stage ^1 Fine."),
        ("critic", "!commit #stage ^1 Recorded."),
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

    let prompt = runner
        .prompts_for("planner")
        .into_iter()
        .next()
        .expect("planner spoke");
    assert!(
        !prompt.contains("Other desks you may put"),
        "a seat that cannot ask must not be told it can — a member that spends its one \
         line on an impossible move has spent a turn of the room's budget on nothing:\n{prompt}"
    );
}

// ---------------------------------------------------------------------------
// The rendered rows
// ---------------------------------------------------------------------------

/// **A question put to a DESK is answered by that desk, not by its first seat.**
///
/// `forward_to_desk` in the library resolves `@#platform` to that desk's one
/// responder — the first active member other than the author — and hands it
/// over as the whole of the crossing. On `members = ["sre", "dba"]` that is
/// `sre` every time and `dba` is not reachable by a desk crossing at all. The
/// asking room did not ask `sre`; it asked the desk, and the objection the
/// host's own `forward` comment raises — that a desk crossing must not "skip
/// the deliberation the desk exists for" — is only answered by convening it.
#[tokio::test]
async fn a_desk_crossing_convenes_the_far_desk_rather_than_asking_one_seat() {
    let far = Room::answering("The replica lag budget is 400ms, measured over the last quarter.");
    let (log, _) = run_with(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    let convened = far.convened();
    assert_eq!(convened.len(), 1, "one question, one room: {convened:?}");
    assert_eq!(convened[0].0, "platform", "the far desk was convened");
    assert_eq!(
        convened[0].1, "planner",
        "attributed to the seat that asked, which is who the question is from"
    );
    assert!(
        convened[0].2.contains("What is the replica lag budget?"),
        "and the room is handed the line that asked it:\n{}",
        convened[0].2
    );
    assert!(
        far.fell_back().is_empty(),
        "no seat was asked on the side — a room that answered has answered: {:?}",
        far.fell_back()
    );

    // **Credited to the desk, never to the seat the library happened to pick.**
    //
    // A room's conclusion is not any one member's line, and several of its
    // seats may have argued the other way. Naming `@sre` would put words in a
    // teammate's mouth on a desk where nobody reading this can check.
    let home = log.replies("eng");
    let returned = home
        .iter()
        .find(|(_, text)| text.contains("400ms"))
        .expect("the answer came home");
    assert_eq!(
        returned.0, HIVE_REFERRAL_AUTHOR,
        "carried by the room as every crossing answer is: {home:?}"
    );
    assert!(
        returned.1.contains("Platform"),
        "and it names the desk that answered: {}",
        returned.1
    );
    assert!(
        !returned.1.contains("@sre") && !returned.1.contains("@dba"),
        "but not a seat, which did not personally answer: {}",
        returned.1
    );
}

/// **A continuation that repeats the line it already committed is not a row.**
///
/// The asker's turn continues on the answer its crossing earned, which is the
/// point of the second pass. A seat handed the same prompt shape twice can
/// simply re-emit the line it can see at the bottom of the transcript, and
/// journaling that put the identical row on the desk back to back — the second
/// copy carrying the crossing, so the console showed one member saying the same
/// thing twice. Seen on `companies/retail_co`.
///
/// The prompt now says what a continuation is for; this pins the guarantee,
/// which does not depend on a model reading it.
#[tokio::test]
async fn a_continuation_that_repeats_the_committed_line_is_not_journaled() {
    let far = FarDesk::answering("The replica lag budget is 400ms.");
    let (log, _) = run(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            // The continuation, saying nothing the first pass did not.
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        Some(&far),
    )
    .await;

    let asked: Vec<_> = log
        .replies("eng")
        .into_iter()
        .filter(|(author, text)| author == "planner" && text.contains("replica lag budget?"))
        .collect();
    assert_eq!(
        asked.len(),
        1,
        "the question is one row, not two: {asked:?}"
    );
}

/// **No continuation on an answer this seat cannot read.**
///
/// `returns = false` is a legitimate policy: ask a peer desk, let the answer
/// stand on their transcript, and do not carry it back. `forward` still records
/// the question on the ledger, so counting successful far-desk turns reported an
/// answer the asking desk had never received — and the continuation ran with a
/// prompt announcing it, free to commit a decision on information its speaker
/// never saw (Codex, #2332).
#[tokio::test]
async fn a_crossing_whose_answer_never_came_home_grants_no_continuation() {
    const NO_RETURNS: &str = "hive = { turn_budget = 6, quorum = 2, blind_round = false, \
                              referral = { enabled = true, returns = false } }";
    let far = FarDesk::answering("The replica lag budget is 400ms.");
    let (log, _) = run(
        NO_RETURNS,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            // Would be the continuation, if one were granted.
            (
                "planner",
                "!evidence #lag ^2 platform came back with 400ms.",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        Some(&far),
    )
    .await;

    assert_eq!(far.asked().len(), 1, "the question was still put");
    let home = log.replies("eng");
    assert!(
        home.iter()
            .all(|(_, text)| !text.contains("The replica lag budget is 400ms")),
        "the far desk's answer stays on the far desk, which is what `returns = false` \
         asks for: {home:?}"
    );
    // **The structural tell, not the text.** A continuation makes the asker
    // speak twice in immediate succession — its question, then its second pass
    // on the answer. Denied one, the floor passes to somebody else. Asserted
    // this way because the room's ordinary rotation brings the asker back later
    // anyway, so counting its rows or matching its words cannot tell a granted
    // continuation from a scheduled turn.
    let asked_at = home
        .iter()
        .position(|(author, text)| {
            author == "planner" && text.contains("What is the replica lag budget?")
        })
        .expect("the question is on the desk");
    assert_ne!(
        home[asked_at + 1].0,
        "planner",
        "the asker must not speak again on an answer it never received: {home:?}"
    );
}

/// **A continuation that says something new IS a row.**
///
/// The guard above must not cost the feature it protects: the whole reason the
/// asker speaks twice is to use what came back, and a seat that does exactly
/// that has earned its second row.
#[tokio::test]
async fn a_continuation_that_reads_the_answer_is_journaled() {
    let far = FarDesk::answering("The replica lag budget is 400ms.");
    let (log, _) = run(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            (
                "planner",
                "!evidence #lag ^2 platform came back with 400ms, which the staged plan fits.",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        Some(&far),
    )
    .await;

    assert!(
        log.replies("eng")
            .iter()
            .any(|(author, text)| author == "planner"
                && text.contains("platform came back with 400ms")),
        "the asker's conclusion on its own crossing: {:?}",
        log.replies("eng")
    );
}

/// **A room's answer is not written onto the far desk a second time.**
///
/// The single-seat path has to journal the answer there — the far desk's only
/// row IS that seat's turn, and nothing else records it. A room has already
/// written the question, every turn and the closing report, so repeating the
/// answer appended a verbatim copy of the report under the seat the library had
/// resolved, with `parent: None`. Three faults in one row: text already on the
/// desk, a conclusion attributed to one member who did not write it, and a
/// channel-level row no projection drops, reading as that member posting the
/// room's summary as their own message.
#[tokio::test]
async fn a_rooms_conclusion_is_not_echoed_onto_the_far_desk() {
    const CONCLUSION: &str = "the Platform desk settled on a 400ms budget";
    let far = Room::answering(CONCLUSION);
    let (log, _) = run_with(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    // Nothing at all on the far desk from this side: the `Room` fixture stands
    // in for the real episode, so every row there would be an echo.
    let far_rows = log.replies("platform");
    assert!(
        far_rows.is_empty(),
        "the room wrote its own transcript; this path must add nothing: {far_rows:?}"
    );
    // And the answer still comes home, once.
    let carried: Vec<_> = log
        .replies("eng")
        .into_iter()
        .filter(|(_, text)| text.contains(CONCLUSION))
        .collect();
    assert_eq!(carried.len(), 1, "exactly one copy comes home: {carried:?}");
    assert_eq!(carried[0].0, HIVE_REFERRAL_AUTHOR);
}

/// **`deliberates = false` is the single-seat arm, unchanged.**
///
/// Every measurement taken before the room existed was taken against this
/// path, so it has to stay reachable and stay identical — a knob that changed
/// the cheap arm too would make the comparison it exists for meaningless.
#[tokio::test]
async fn a_desk_that_turns_deliberation_off_still_asks_one_seat() {
    const SEAT_ONLY: &str = "hive = { turn_budget = 6, quorum = 2, blind_round = false, \
                             referral = { enabled = true, deliberates = false } }";
    let far = Room::answering("a room would have said this");
    let (log, _) = run_with(
        SEAT_ONLY,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    assert!(
        far.convened().is_empty(),
        "the desk said not to convene: {:?}",
        far.convened()
    );
    let seat = far.fell_back();
    assert_eq!(seat.len(), 1, "and one seat answered instead: {seat:?}");
    assert_eq!(seat[0].1, "sre", "the far desk's first eligible member");
    let returned = log
        .replies("eng")
        .into_iter()
        .find(|(_, text)| text.contains("one seat's opinion"))
        .expect("the seat's answer came home");
    assert!(
        returned.1.contains("@sre"),
        "attributed to the seat, because a seat is what answered: {}",
        returned.1
    );
}

/// **A far desk that cannot hold a room falls back to its seat.**
///
/// No `[hive]` block, one member, a quorum out of reach right now: all
/// ordinary shapes a company is allowed to have, and every one of them has a
/// correct answer that is not a failure. Refusing the crossing instead would
/// deny a company something it is entitled to do.
#[tokio::test]
async fn a_far_desk_that_cannot_deliberate_is_still_asked() {
    let far = Room::cannot();
    let (log, _) = run_with(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    assert_eq!(far.convened().len(), 1, "it was offered the room");
    let seat = far.fell_back();
    assert_eq!(seat.len(), 1, "and answered as a seat: {seat:?}");
    assert!(
        log.replies("eng")
            .iter()
            .any(|(author, text)| author == HIVE_REFERRAL_AUTHOR
                && text.contains("one seat's opinion")),
        "the answer still comes home"
    );
}

/// **A room that broke is a failed crossing, not a retry as one seat.**
///
/// Falling through to `refer` here would bill the far desk's turns and then
/// bill a turn again for the same question — and tell the asking room nothing
/// about the first attempt.
#[tokio::test]
async fn a_room_that_breaks_does_not_then_ask_a_seat() {
    let far = Room::broken();
    let (log, outcome) = run_with(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    assert_eq!(far.convened().len(), 1);
    assert!(
        far.fell_back().is_empty(),
        "no second attempt on the same question: {:?}",
        far.fell_back()
    );
    assert_eq!(outcome.referrals.failed, 1, "counted as a failed crossing");
    assert!(
        log.replies("eng")
            .iter()
            .any(|(author, text)| author == HIVE_REFERRAL_AUTHOR
                && text.contains("did not answer the question")),
        "and the room that spent the question is told it got nothing back: {:?}",
        log.replies("eng")
    );
}

/// **A room's prompt must not forbid the markers its protocol requires.**
///
/// `referral_prompt` closes with "Do not open your reply with a `!` marker:
/// this is not a deliberation turn, it is an answer to a colleague" — correct
/// for the single seat it was written for. Handed to a room as its task, it
/// contradicts `EpisodePrompt`'s own protocol ("Reply with ONE line only,
/// beginning with exactly one of these markers") inside the same prompt, and a
/// member that obeys the task deposits prose the fold counts for nothing.
/// Three of the first four live crossings ended `idle` with markerless turns.
#[test]
pub(super) fn the_prompt_a_room_is_handed_does_not_forbid_its_own_protocol() {
    let seat = referral::referral_prompt("planner", "Engineering", "What is the lag budget?");
    assert!(
        seat.contains("Do not open your reply with a `!` marker"),
        "the single-seat prompt still says so, which is right for a seat:\n{seat}"
    );

    let room = referral::referral_room_prompt("planner", "Engineering", "What is the lag budget?");
    assert!(
        !room.contains('`'),
        "a room is told nothing about markers — its protocol owns the shape of a turn:\n{room}"
    );
    assert!(room.contains("planner on the Engineering desk"));
    assert!(room.contains("What is the lag budget?"));
    // Recovered by the same reader, from either shape.
    assert_eq!(
        referral::asked_message(&room),
        "What is the lag budget?",
        "the room prompt puts the question last, with no footer to strip"
    );
}

/// **A question that quotes the footer is not truncated at it.**
///
/// The question is operator- and agent-authored, so it can contain the
/// footer's opening sentence; searching forwards treats that copy as the
/// generated footer and cuts the question there.
/// **A ROOM question quoting the footer survives too.**
///
/// `referral_room_prompt` deliberately has no footer, so the backwards split
/// that fixed the single-seat case found the question's own copy of that
/// sentence and truncated there instead — the same defect as the forward
/// split, in the other prompt shape (Codex, #2332). The footer is now taken
/// from `referral_prompt` whole and removed only as a suffix, so a shape that
/// does not end with it keeps its question intact.
#[test]
pub(super) fn a_room_question_containing_the_footers_words_is_not_truncated() {
    let question = "Answer in a few sentences. is what the customer wrote — can we refund it?";
    let room = referral::referral_room_prompt("planner", "Engineering", question);
    assert_eq!(
        referral::asked_message(&room),
        question,
        "a footerless prompt has nothing to strip"
    );
}

#[test]
pub(super) fn a_question_containing_the_footers_words_survives_unwrapping() {
    let question = "Answer in a few sentences. is what they told us — what is the lag budget?";
    let prompt = referral::referral_prompt("planner", "Engineering", question);
    assert_eq!(
        referral::asked_message(&prompt),
        question,
        "the generated footer is last, so only the last occurrence delimits it"
    );
}

/// **A room that could not answer is reported as the DESK failing, not a seat.**
///
/// `forward` returns from the `deliberate` arm before `refer` ever runs, so on
/// a room failure no seat was asked anything. Naming the seat the library
/// resolved credits a teammate with a refusal it never made, on a desk where
/// nobody reading the row can check — the same defect `room_note` removes on
/// the success path (CodeRabbit, #2332).
#[tokio::test]
async fn a_room_that_could_not_answer_is_not_blamed_on_a_seat() {
    let far = Room::broken();
    let (log, _) = run_with(
        REFERRING,
        &[
            (
                "planner",
                "!question #lag What is the replica lag budget? @#platform",
            ),
            ("scout", "!propose #stage Stage the rollout behind a flag."),
            ("critic", "!support #stage ^1 Staging fits the lag budget."),
            ("planner", "!commit #stage ^3 Recorded."),
        ],
        &far,
    )
    .await;

    let note = log
        .replies("eng")
        .into_iter()
        .find(|(_, text)| text.contains("did not answer the question"))
        .expect("the room that spent the question is told it got nothing back");
    assert_eq!(note.0, HIVE_REFERRAL_AUTHOR);
    assert!(
        note.1.contains("Platform"),
        "the desk that was asked is named: {}",
        note.1
    );
    assert!(
        !note.1.contains("@sre") && !note.1.contains("@dba"),
        "and no seat is blamed for a refusal it never made: {}",
        note.1
    );
}

#[test]
pub(super) fn a_rooms_failure_names_the_desk_and_no_seat_within_it() {
    assert_eq!(
        referral::room_unanswered_note("Platform"),
        "the Platform desk did not answer the question."
    );
    // The seat form is unchanged for a crossing that actually asked a seat.
    assert_eq!(
        referral::unanswered_note("sre", "Platform"),
        "@sre on the Platform desk did not answer the question."
    );
}

#[test]
pub(super) fn a_rooms_answer_names_the_desk_and_no_seat_within_it() {
    let note = referral::room_note("Platform", "  !support #stage It holds.  ");
    assert_eq!(
        note,
        "the Platform desk answered the question: support #stage It holds."
    );
}

#[test]
pub(super) fn a_returned_answer_names_its_author_and_desk_and_carries_no_marker() {
    let note = referral::returned_note("sre", "Platform", "  !support #stage It holds.  ");
    assert_eq!(
        note,
        "@sre on the Platform desk answered the question: support #stage It holds."
    );
}

#[test]
pub(super) fn the_referral_prompt_frames_a_colleagues_question_not_a_deliberation_turn() {
    let prompt = referral::referral_prompt("Planner", "Engineering", "What is the lag budget?");
    assert!(prompt.contains("Planner on the Engineering desk"));
    assert!(prompt.contains("What is the lag budget?"));
    assert!(
        prompt.contains("Do not open your reply with a `!` marker"),
        "the far teammate is answering a colleague, not depositing a move on its own \
         desk's board:\n{prompt}"
    );
}

// ---------------------------------------------------------------------------
// The manifest
// ---------------------------------------------------------------------------

/// The referral block is parsed off `[[group_chat]].hive`, not invented here.
#[test]
pub(super) fn the_manifest_parses_a_referral_block() {
    let manifest = two_desks(
        "hive = { referral = { enabled = true, max_hops = 3, reach = \"channels\", \
         returns = false, peer_cap = 4 } }",
    );
    let desk = desk_of(&manifest, "eng").expect("a room");
    let referral = &desk.config.referral;
    assert!(referral.enabled());
    assert_eq!(referral.peer_cap(), 4);
    let policy = referral.policy();
    assert_eq!(policy.max_hops, 3);
    assert!(!policy.returns);
    assert!(policy.reach.crosses());
    assert!(
        !policy.reach.addresses_desks(),
        "`channels` lets a turn run elsewhere without making `@#desk` mean anything"
    );
}

/// Every check here catches a policy that would be *silently* inert. A desk
/// that asks nothing looks exactly like a desk whose members had nothing to
/// ask, so a typo has to be a validation error rather than a quiet no-op.
#[test]
pub(super) fn the_manifest_refuses_a_referral_policy_that_could_never_fire() {
    let problems = |hive: &str| record(&two_desks(hive)).manifest.validate();

    let found = problems("hive = { referral = { enabled = true, reach = \"everywhere\" } }");
    assert!(
        found.iter().any(|p| p.contains("hive.referral.reach")),
        "{found:?}"
    );

    for key in ["max_hops", "peer_cap"] {
        let found = problems(&format!(
            "hive = {{ referral = {{ enabled = true, {key} = 0 }} }}"
        ));
        assert!(
            found
                .iter()
                .any(|p| p.contains(&format!("hive.referral.{key} = 0"))),
            "{key}: {found:?}"
        );
    }

    // A round trip is two hops. One hop plus `returns` describes a question
    // whose answer is thrown away, which is worse than refusing it.
    let found = problems("hive = { referral = { enabled = true, max_hops = 1 } }");
    assert!(
        found.iter().any(|p| p.contains("a round trip is two hops")),
        "{found:?}"
    );
    // Declared one-way, it is a policy somebody meant.
    let found = problems("hive = { referral = { enabled = true, max_hops = 1, returns = false } }");
    assert!(!found.iter().any(|p| p.contains("round trip")), "{found:?}");

    // And the ordinary opted-in block is accepted.
    assert!(problems(REFERRING).is_empty(), "{:?}", problems(REFERRING));
}
