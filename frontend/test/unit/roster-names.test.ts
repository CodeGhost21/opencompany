import { describe, expect, it } from "vitest";

import { rosterDisplayName, rosterNameMap } from "@/lib/roster-names";

/**
 * The shared roster id -> display name resolver (issue #973).
 *
 * #931 was the same class of bug — an operator-added teammate's id is a
 * minted internal string (a ULID for the eight teammates created before #686
 * started minting a readable slug), and a surface that prints it instead of
 * the name is telling the reader nothing. #939 fixed the two connections
 * surfaces; this is the resolver every id-bearing surface should route
 * through from here on, so a fifth surface cannot regress the same way.
 */

describe("rosterDisplayName", () => {
  it("resolves a known id to its name", () => {
    const names = rosterNameMap([
      { id: "019fa75dbc9b-000000000005", name: "Mark" },
      { id: "backend_engineer", name: "backend dev" },
    ]);
    expect(rosterDisplayName("019fa75dbc9b-000000000005", names)).toBe("Mark");
    expect(rosterDisplayName("backend_engineer", names)).toBe("backend dev");
  });

  it("falls back to the id itself for an id the roster does not carry", () => {
    // Roster not loaded yet, or an id that names something else entirely —
    // either way, the label must never go blank.
    const names = rosterNameMap([]);
    expect(rosterDisplayName("019fa75dbc9b-000000000005", names)).toBe(
      "019fa75dbc9b-000000000005",
    );
  });

  it("falls back to the id when the roster's name for it is empty", () => {
    // A genuinely nameless entry must still render something, not a blank
    // label — the id is the honest fallback everywhere else here.
    const names = rosterNameMap([{ id: "member-7", name: "" }]);
    expect(rosterDisplayName("member-7", names)).toBe("member-7");
  });
});

describe("rosterNameMap", () => {
  it("last write wins on a duplicate id", () => {
    const names = rosterNameMap([
      { id: "cmo", name: "First" },
      { id: "cmo", name: "Second" },
    ]);
    expect(rosterDisplayName("cmo", names)).toBe("Second");
  });
});
