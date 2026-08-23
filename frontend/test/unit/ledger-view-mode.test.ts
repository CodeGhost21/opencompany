import { describe, expect, it } from "vitest";

import { readLedgerViewMode } from "@/hooks/use-ledger-view-mode";

describe("ledger view mode route state", () => {
  it("selects List only when the hash explicitly asks for it", () => {
    expect(readLedgerViewMode("#/ledgers/tasks?view=list")).toBe("list");
    expect(readLedgerViewMode("#/ledgers/tasks?view=board")).toBe("board");
    expect(readLedgerViewMode("#/ledgers/tasks?new")).toBe("board");
  });
});
