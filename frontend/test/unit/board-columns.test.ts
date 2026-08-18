// The board's columns, now that the console reads them instead of keeping them.
//
// The list these replace was a hand-maintained copy of the host's, and the only
// thing that could catch it drifting was an end-to-end test against a live
// host. What is left to test here is the *reading*: that a host label wins, that
// the fallback is readable rather than a raw wire word, and that declaration
// order survives — because a console that sorted the columns itself would put
// Done beside To-do the first time somebody added one.

import { describe, expect, it } from "vitest";

import {
  BOARD_LEDGER,
  columnOf,
  columnsOf,
  humanizeStatus,
  labelFor,
} from "@/lib/board-columns";
import type { LedgerSummary } from "@/api/ledgers";

function board(): LedgerSummary {
  return {
    slug: BOARD_LEDGER,
    title: "Tasks",
    purpose: "The company's work board.",
    source: "native",
    derived: "derived/TASKS.md",
    writtenBy: "the board",
    builtin: true,
    fields: [
      { name: "id", role: "id" },
      { name: "title", role: "title" },
      { name: "column", role: "status" },
    ],
    statuses: [
      { name: "todo", label: "To-do" },
      { name: "planning", label: "Planning" },
      { name: "in_progress", label: "In progress" },
      { name: "paused", label: "Paused" },
      { name: "in_review", label: "In review" },
      { name: "done", label: "Done", closed: true },
    ],
    sections: [],
    open: 0,
    closed: 0,
  };
}

describe("reading the board's columns off the ledger", () => {
  it("keeps declaration order, which is board order", () => {
    const ids = columnsOf(board()).map((column) => column.id);
    expect(ids).toEqual([
      "todo",
      "planning",
      "in_progress",
      "paused",
      "in_review",
      "done",
    ]);
  });

  it("takes the host's label rather than deriving one", () => {
    const columns = columnsOf(board());
    expect(labelFor(columns, "todo")).toBe("To-do");
    expect(labelFor(columns, "in_progress")).toBe("In progress");
  });

  it("carries which column ends a card's life", () => {
    const columns = columnsOf(board());
    expect(columns.filter((column) => column.closed).map((c) => c.id)).toEqual([
      "done",
    ]);
  });

  // The reason the host sends labels at all: deriving them gets the very first
  // column wrong, and a board headed "Todo" is a board that looks broken.
  it("does not fall back to humanising when the host sent a label", () => {
    expect(humanizeStatus("todo")).toBe("Todo");
    expect(labelFor(columnsOf(board()), "todo")).toBe("To-do");
  });
});

describe("statuses the host sent no label for", () => {
  it("humanises a declared ledger's own statuses", () => {
    expect(columnOf({ name: "at_risk" }).label).toBe("At risk");
    expect(columnOf({ name: "open" }).label).toBe("Open");
    expect(columnOf({ name: "not-started" }).label).toBe("Not started");
  });

  it("treats a blank label as no label", () => {
    expect(columnOf({ name: "open", label: "   " }).label).toBe("Open");
  });

  it("leaves an unreadable id alone rather than mangling it", () => {
    expect(humanizeStatus("")).toBe("");
    expect(humanizeStatus("   ")).toBe("   ");
  });
});

describe("labelling a column nothing declares", () => {
  // A stored card can carry a column this build has never heard of — a rollback,
  // a column removed. It must still read as words rather than as `in_review`.
  it("humanises rather than printing the wire word", () => {
    expect(labelFor(columnsOf(board()), "in_triage")).toBe("In triage");
  });

  // The board renders before its column read lands. Labelling has to work
  // against an empty list without showing a flash of raw ids.
  it("works before any columns have loaded", () => {
    expect(labelFor([], "in_progress")).toBe("In progress");
    expect(labelFor([], "done")).toBe("Done");
  });
});
