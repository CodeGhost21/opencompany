// The console's half of the ledger rules.
//
// These are the predicates the Ledgers screen renders from, and each one exists
// so the UI meets a host rule *before* it bites rather than reporting a refused
// save afterwards.

import { describe, expect, it } from "vitest";

import {
  byline,
  composableFields,
  idField,
  isClosingStatus,
  isWritable,
  statusField,
  statusNeedsReason,
  titleField,
  type LedgerSummary,
} from "@/api/ledgers";

function ledger(overrides: Partial<LedgerSummary> = {}): LedgerSummary {
  return {
    slug: "risks",
    title: "Risks",
    purpose: "What could go wrong.",
    source: "events",
    derived: "derived/RISKS.md",
    writtenBy: "`record_entry` to add or amend a row",
    builtin: false,
    fields: [
      { name: "id", role: "id" },
      { name: "risk", role: "title" },
      { name: "status", role: "status" },
      { name: "mitigation", role: "prose" },
      { name: "reason", role: "prose" },
    ],
    statuses: [
      { name: "open" },
      { name: "closed", closed: true, needsReason: true },
      { name: "superseded", closed: true },
    ],
    sections: [],
    open: 0,
    closed: 0,
    ...overrides,
  };
}

describe("reading a ledger's declared shape", () => {
  it("finds the id, title and status fields by role rather than by name", () => {
    const held = ledger();
    expect(idField(held)?.name).toBe("id");
    expect(titleField(held)?.name).toBe("risk");
    expect(statusField(held)?.name).toBe("status");
  });

  // A ledger a teammate declared may name its fields anything. Reading by role
  // is what lets the screen render one it has never seen.
  it("works on a ledger whose fields are named nothing familiar", () => {
    const held = ledger({
      fields: [
        { name: "ref", role: "id" },
        { name: "headline", role: "title" },
        { name: "stance", role: "status" },
      ],
    });
    expect(idField(held)?.name).toBe("ref");
    expect(titleField(held)?.name).toBe("headline");
    expect(statusField(held)?.name).toBe("stance");
  });

  it("offers every field but the id and the status to the compose form", () => {
    const names = composableFields(ledger()).map((field) => field.name);
    expect(names).toEqual(["risk", "mitigation", "reason"]);
  });

  it("tolerates a ledger that declares no status at all", () => {
    const held = ledger({
      fields: [
        { name: "id", role: "id" },
        { name: "what", role: "title" },
      ],
    });
    expect(statusField(held)).toBeUndefined();
    expect(isClosingStatus(held, "open")).toBe(false);
  });
});

describe("what closes a row", () => {
  it("reads closedness off the declaration, not off a status name", () => {
    const held = ledger();
    expect(isClosingStatus(held, "closed")).toBe(true);
    expect(isClosingStatus(held, "superseded")).toBe(true);
    expect(isClosingStatus(held, "open")).toBe(false);
    // A word that *sounds* terminal but is not declared closed is not closed.
    expect(isClosingStatus(held, "done")).toBe(false);
  });

  it("matches a status case-insensitively and ignores surrounding space", () => {
    expect(isClosingStatus(ledger(), " CLOSED ")).toBe(true);
  });

  // The form asks for the reason so the save does not fail for want of it.
  it("knows which closing statuses demand a reason", () => {
    const held = ledger();
    expect(statusNeedsReason(held, "closed")).toBe(true);
    expect(statusNeedsReason(held, "superseded")).toBe(false);
    expect(statusNeedsReason(held, "open")).toBe(false);
  });
});

describe("what this surface may write", () => {
  it("treats an events ledger as writable and a native one as not", () => {
    expect(isWritable(ledger())).toBe(true);
    // The board's rows fire dispatch and planning passes, so they are written
    // on the board. Offering a compose box here would produce a refused save.
    expect(isWritable(ledger({ source: "native" }))).toBe(false);
  });
});

describe("bylines", () => {
  it("prefers the label, falls back to the id, and always names the kind", () => {
    expect(byline({ kind: "human", id: "u-1", label: "Dana" })).toBe(
      "Dana (human)",
    );
    expect(byline({ kind: "agent", id: "ceo" })).toBe("ceo (agent)");
    expect(byline({ kind: "system" })).toBe("system");
    // A blank label must not render as " ()".
    expect(byline({ kind: "agent", id: "ceo", label: "   " })).toBe(
      "ceo (agent)",
    );
  });
});
