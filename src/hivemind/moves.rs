//! The per-member move grammar: which trace kinds a seat may open a line with.
//!
//! A room whose members may all make every move is a room that votes. The live
//! evidence is unambiguous: on a nine-episode run of `companies/hive_math_lab`
//! every episode was three independent `!propose`s of the same number followed
//! by a `!commit`, and quorum carried because in `tinyhivemind` a `!propose`
//! counts as its own author's support. No `!support`, `!object`, `!evidence`,
//! `!question` or `!pin` was ever deposited. Three agents agreeing in parallel
//! is not deliberation; it is three answers with a quorum rule stapled on.
//!
//! The repair is structural rather than exhortative. A desk assigns moves —
//! one proposer, one challenger who may only object or refute, an archivist who
//! may only supply evidence — and the driver **enforces** the assignment, so a
//! member that cannot propose has to do something with the floor other than
//! restate the answer.
//!
//! Enforcement is deliberately two-stage and never fatal: a first violation is
//! a one-line correction and a second attempt, and a second violation is
//! journaled with its leading `!` removed. A demoted line still says whatever
//! the member wanted to say — it simply deposits no trace, because
//! `tinyhivemind_hive::trace::resolve` only reads a marker at the start of a
//! line. It can therefore never be folded as support for anything, which is the
//! one outcome that would let a barred move still carry a topic.

use std::collections::BTreeMap;

/// Every kind a `[group_chat.hive].moves` entry may name.
///
/// `pin` covers `!unpin` as well: both write the same desk pinboard, and a
/// member entitled to put something on the board is entitled to take it off
/// again. Splitting them would be a permission nobody could use correctly.
pub const MOVE_KINDS: &[&str] = &[
    "propose", "support", "object", "refute", "evidence", "question", "defer", "commit", "pin",
];

/// The permission kind a marker line asks for, if it asks for one.
///
/// `None` for a line with no leading marker, and for a marker this host does
/// not police — a line the fold itself discards is not a move anybody made, and
/// demoting it would be a correction with no subject.
#[must_use]
pub fn line_kind(line: &str) -> Option<&'static str> {
    let word = line.trim_start().strip_prefix('!')?;
    let word = word.split_whitespace().next()?;
    match word {
        "unpin" => Some("pin"),
        other => MOVE_KINDS.iter().copied().find(|kind| *kind == other),
    }
}

/// The moves `member` may open a line with, in [`MOVE_KINDS`] order.
///
/// A member the map does not name may make every move, which is what makes an
/// omitted `moves` table a no-op for every manifest written before it existed.
/// An entry naming no kind at all is read as "every move" for the same reason a
/// silent member is: an empty list is far more likely to be a table written and
/// never filled in than a deliberate vow of silence, and the alternative reading
/// hands somebody the floor with nothing legal to say.
#[must_use]
pub fn allowed_for(moves: &BTreeMap<String, Vec<String>>, member: &str) -> Vec<&'static str> {
    match moves.get(member) {
        Some(declared) if !declared.is_empty() => MOVE_KINDS
            .iter()
            .copied()
            .filter(|kind| declared.iter().any(|named| named == kind))
            .collect(),
        _ => MOVE_KINDS.to_vec(),
    }
}

/// The one-line correction a member gets after its first barred move.
///
/// One line, because the whole prompt already told it what its moves are and a
/// second paragraph explaining the rule again is how a correction turns into a
/// context-window tax. It names the move it may not make first: a model that
/// reads only the head of a correction still learns the thing it got wrong.
#[must_use]
pub fn correction(attempted: &str, allowed: &[&str]) -> String {
    let moves = allowed
        .iter()
        .map(|kind| format!("!{kind}"))
        .collect::<Vec<_>>()
        .join(", ");
    let moves = if moves.is_empty() {
        "none on this turn".to_owned()
    } else {
        moves
    };
    format!(
        "You may not `!{attempted}` on this desk; your moves are {moves}. Reply again with ONE \
         line beginning with one of those markers."
    )
}

/// The same line, stripped of the marker that made it a move.
///
/// The leading `!` and nothing else: the member's own words are kept verbatim,
/// so the transcript records what it wanted to say and a reader can see the
/// attempt. What it loses is the only thing at stake — `resolve` reads a
/// marker at the start of a line and nowhere else, so a demoted line folds to
/// no trace and cannot support, object to, or commit anything.
///
#[must_use]
pub fn demote(line: &str) -> String {
    line.trim_start().strip_prefix('!').map_or_else(
        || line.trim().to_owned(),
        |rest| {
            let demoted = rest.trim_start();
            if demoted.is_empty() {
                "(no answer)".to_owned()
            } else {
                demoted.to_owned()
            }
        },
    )
}

/// One line a member deposited that its seat is not entitled to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveViolation {
    /// The member that made it, twice.
    pub agent_id: String,
    /// The kind it reached for, without the `!`.
    pub attempted: String,
}
