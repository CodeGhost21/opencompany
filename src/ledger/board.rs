//! The board's column vocabulary, declared once.
//!
//! The task board's columns existed in three places that could not check each
//! other: a `[&str; 6]` on the port, a `match` mapping each id to a label
//! beside it, and a hand-maintained `TASK_COLUMNS` in the console — whose own
//! comment admitted the drift it could not prevent, since *"a Rust test cannot
//! see the TS list, so a column added on one side and not the other keeps this
//! green."*
//!
//! This is that one place. [`COLUMNS`] carries every column's id, its human
//! label, whether it ends a card's life, and where it renders in the ledger
//! file; [`crate::ports::tasks`] derives its list and its labels from it, the
//! `tasks` ledger declaration builds its statuses and sections from it, and the
//! console reads the labels off the ledger rather than keeping a copy. Adding a
//! column is one edit here.
//!
//! # What is deliberately still a plain const
//!
//! The individual ids — [`COLUMN_IN_PROGRESS`](crate::ports::tasks::COLUMN_IN_PROGRESS)
//! and its siblings — stay leaf constants on the port, and this table refers to
//! them. They are load-bearing in a way a label is not: entering `in_progress`
//! *dispatches the card*, and the edge keys off that exact literal. So the
//! **identity** of a column is a compile-time constant a `match` arm can name,
//! and only its presentation and its grouping live here.
//!
//! # And why the table is not itself declarable
//!
//! A company may declare any ledger it likes, but not this one: a column here
//! is a lifecycle state that spends money. `planning` fires a model call,
//! `in_progress` opens an attempt, and `done` is reachable only through a human
//! verdict. A company that could add a seventh column from a JSON file would
//! have a state the runtime has no edge for, and a card that entered it would
//! sit there forever with nothing to say why.

use crate::ports::tasks::{
    COLUMN_DONE, COLUMN_IN_PROGRESS, COLUMN_IN_REVIEW, COLUMN_PAUSED, COLUMN_PLANNING, COLUMN_TODO,
};

/// One column of the board, and everything any surface needs to render it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardColumn {
    /// The wire word a card stores and the dispatch edge matches on.
    pub id: &'static str,
    /// What a person reads. Never derived from the id: `in_review` humanises to
    /// "In review" by luck and `todo` does not.
    pub label: &'static str,
    /// Whether a card here is finished.
    ///
    /// Exactly one column is, and it is not a presentation detail: it decides
    /// which cards an index files as archive and which the ledger counts as
    /// outstanding. A card in review or paused is **stopped, not finished** —
    /// calling either closed would make "what is still open" answer wrong.
    pub closed: bool,
    /// The heading this column's cards render under in `derived/tasks.md`.
    ///
    /// Several columns share one heading on purpose: a reader opening the file
    /// wants *in flight* and *waiting on a person*, not six lists of one. The
    /// board itself still shows a column per id — the grouping is the file's,
    /// not the board's.
    pub section: &'static str,
    /// The sentence under that heading, taken from the first column in it.
    pub blurb: &'static str,
}

/// Every column, in board order.
///
/// The order is the reading order and the drag order, and it is asserted
/// against a literal in [`crate::ports::tasks`] so a reorder is a deliberate
/// two-line change rather than a side effect of editing this table.
pub const COLUMNS: [BoardColumn; 6] = [
    BoardColumn {
        id: COLUMN_TODO,
        label: "To-do",
        closed: false,
        section: "Not started",
        blurb: "Waiting for a person to say the work should start — entered by hand, or returned \
                by a pass that could not finish. Most recently added or returned first.",
    },
    BoardColumn {
        id: COLUMN_PLANNING,
        label: "Planning",
        closed: false,
        section: "In flight",
        blurb: "Work a teammate is on, or that a planning pass is turning into a brief.",
    },
    BoardColumn {
        id: COLUMN_IN_PROGRESS,
        label: "In progress",
        closed: false,
        section: "In flight",
        blurb: "Work a teammate is on, or that a planning pass is turning into a brief.",
    },
    BoardColumn {
        id: COLUMN_PAUSED,
        label: "Paused",
        closed: false,
        section: "Waiting on a person",
        blurb: "Stopped, not finished. Each of these needs somebody to decide something before it \
                can move.",
    },
    BoardColumn {
        id: COLUMN_IN_REVIEW,
        label: "In review",
        closed: false,
        section: "Waiting on a person",
        blurb: "Stopped, not finished. Each of these needs somebody to decide something before it \
                can move.",
    },
    BoardColumn {
        id: COLUMN_DONE,
        label: "Done",
        closed: true,
        section: "Recently done",
        blurb: "The most recently finished. Kept, because a company that cannot see what it \
                already did repeats it.",
    },
];

/// Every column id, in board order.
///
/// A `const fn` so [`crate::ports::tasks::BOARD_COLUMNS`] stays a genuine
/// constant — usable in a `match`, in an array length, and in a `const` context
/// — rather than becoming a lazily-built `Vec` every caller has to unwrap. The
/// alternative was a second literal list, which is the thing this module
/// exists to delete.
pub const fn ids() -> [&'static str; COLUMNS.len()] {
    let mut out = [""; COLUMNS.len()];
    let mut index = 0;
    while index < COLUMNS.len() {
        out[index] = COLUMNS[index].id;
        index += 1;
    }
    out
}

/// The column `id` names, if it names one.
pub fn column(id: &str) -> Option<&'static BoardColumn> {
    COLUMNS.iter().find(|column| column.id == id)
}

/// The distinct section headings, in the order their first column appears.
pub fn sections() -> Vec<&'static BoardColumn> {
    let mut out: Vec<&'static BoardColumn> = Vec::new();
    for column in &COLUMNS {
        if !out.iter().any(|held| held.section == column.section) {
            out.push(column);
        }
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;

    /// The ids are the dispatch edge's, so they are pinned against a literal:
    /// a rename here would silently stop `in_progress` dispatching.
    #[test]
    fn the_table_carries_the_boards_ids_in_board_order() {
        assert_eq!(
            ids(),
            [
                "todo",
                "planning",
                "in_progress",
                "paused",
                "in_review",
                "done"
            ]
        );
    }

    /// The labels, pinned.
    ///
    /// This assertion used to be impossible to make here. The console kept its
    /// own copy of these strings, so the only thing that could join the two
    /// lists was an end-to-end test driving a rendered board against a live
    /// host — `test/e2e/board-columns.spec.ts`, whose comment said as much:
    /// *"a Rust test cannot see the TS list."* Now the console reads these off
    /// the `tasks` ledger, so there is one list and this is it.
    #[test]
    fn the_labels_are_the_ones_every_surface_renders() {
        let labels: Vec<&str> = COLUMNS.iter().map(|column| column.label).collect();
        assert_eq!(
            labels,
            [
                "To-do",
                "Planning",
                "In progress",
                "Paused",
                "In review",
                "Done"
            ]
        );
    }

    #[test]
    fn every_column_has_a_label_that_is_not_its_wire_word() {
        for column in &COLUMNS {
            assert!(!column.label.is_empty(), "{} has no label", column.id);
            assert!(
                !column.label.contains('_'),
                "{} still reads as a wire word: {}",
                column.id,
                column.label
            );
        }
    }

    /// Done is the only finished column, and it is reached only by a person's
    /// verdict. Calling review or paused closed would make "what is still
    /// outstanding" answer wrong on every surface at once.
    #[test]
    fn exactly_one_column_is_closed_and_it_is_done() {
        let closed: Vec<&str> = COLUMNS
            .iter()
            .filter(|column| column.closed)
            .map(|column| column.id)
            .collect();
        assert_eq!(closed, [COLUMN_DONE]);
    }

    #[test]
    fn columns_sharing_a_section_share_its_blurb() {
        for column in &COLUMNS {
            let first = COLUMNS
                .iter()
                .find(|held| held.section == column.section)
                .expect("its own section exists");
            assert_eq!(
                column.blurb, first.blurb,
                "`{}` disagrees with the rest of `{}` about what the section is for",
                column.id, column.section
            );
        }
    }

    #[test]
    fn the_sections_are_the_distinct_headings_in_first_appearance_order() {
        let headings: Vec<&str> = sections().iter().map(|column| column.section).collect();
        assert_eq!(
            headings,
            [
                "Not started",
                "In flight",
                "Waiting on a person",
                "Recently done"
            ]
        );
    }

    #[test]
    fn a_column_is_found_by_id_and_an_invented_one_is_not() {
        assert_eq!(column("done").map(|held| held.label), Some("Done"));
        assert!(column("in-progress").is_none());
        assert!(column("").is_none());
    }
}
