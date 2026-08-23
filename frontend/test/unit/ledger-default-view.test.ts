import { describe, expect, it } from "vitest";

import type { LedgerSummary } from "@/api/ledgers";
import { defaultLedgerMode } from "@/views/LedgersView";

/** The native task ledger dispatches from its board; declared ledgers are read. */
const TASKS = {
  slug: "tasks",
  source: "native",
} as unknown as LedgerSummary;

const GOALS = {
  slug: "goals",
  source: "events",
} as unknown as LedgerSummary;

describe("a ledger's initial view", () => {
  it("keeps the native Tasks ledger on its dispatch board", () => {
    expect(defaultLedgerMode(TASKS)).toBe("board");
  });

  it("opens agent-written ledgers as readable rows", () => {
    expect(defaultLedgerMode(GOALS)).toBe("list");
  });

  it("does not give other native ledgers the Tasks board", () => {
    expect(defaultLedgerMode({ ...TASKS, slug: "inbox" })).toBe("list");
  });

  it("does not choose a board before the selected ledger is known", () => {
    expect(defaultLedgerMode(null)).toBe("list");
  });
});
