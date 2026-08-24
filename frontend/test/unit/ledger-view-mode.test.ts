import { describe, expect, it } from "vitest";

import { readLedgerViewMode } from "@/hooks/use-ledger-view-mode";

describe("ledger view mode route state", () => {
  it("selects List only when the hash explicitly asks for it", () => {
    expect(readLedgerViewMode("#/ledgers/tasks?view=list")).toBe("list");
    expect(readLedgerViewMode("#/ledgers/tasks?view=board")).toBe("board");
    expect(readLedgerViewMode("#/ledgers/tasks?new")).toBe("board");
  });

  it("defaults to the ledger's own mode when the hash names no view", () => {
    // A declared ledger opens as rows; the tasks ledger opens as a board —
    // that is `defaultLedgerMode`, passed in as the fallback (issue #1351).
    expect(readLedgerViewMode("#/ledgers/goals", "list")).toBe("list");
    // An explicit `?view=list` keeps winning over the fallback.
    expect(readLedgerViewMode("#/ledgers/tasks?view=list", "board")).toBe("list");
  });
});
