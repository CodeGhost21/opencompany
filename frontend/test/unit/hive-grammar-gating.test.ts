import { describe, expect, it } from "vitest";

import {
  derivedQuorum,
  derivedTurnBudget,
  effectiveQuorum,
  effectiveTurnBudget,
  eligibleSupporters,
  grammarProblems,
  isGoverned,
  may,
  movesFor,
} from "@/lib/hive/grammar";

/**
 * The per-seat move table, and the numbers a desk runs on when nobody named one.
 *
 * The gating rules all fail **open** — a mistake here hands a seat every move and
 * the desk quietly goes back to voting, which is the exact symptom the table
 * exists to fix and is invisible in the transcript.
 */

describe("what a seat may open a line with", () => {
  it("gives every move to a seat the table does not name", () => {
    // An omitted table is a no-op for every manifest written before it existed.
    expect(movesFor({ solver: ["propose"] }, "archivist")).toHaveLength(9);
  });

  it("gives every move to a seat named with an empty list", () => {
    // An empty list is a table somebody started and never filled in far more
    // often than it is a vow of silence, and the other reading hands a seat the
    // floor with nothing legal to say.
    expect(movesFor({ archivist: [] }, "archivist")).toHaveLength(9);
  });

  it("never takes away commit, question or defer", () => {
    // A desk that could bar a seat from recording a decision reaches quorum and
    // then hands the Commit floor to somebody with nothing legal to say — which
    // is how a six-member desk reported itself exhausted on an answer it had
    // already carried.
    const narrow = movesFor({ skeptic: ["object"] }, "skeptic");
    expect(narrow).toContain("commit");
    expect(narrow).toContain("question");
    expect(narrow).toContain("defer");
    expect(narrow).toContain("object");
    expect(narrow).not.toContain("propose");
  });

  it("returns kinds in the canonical order, so console and prompt agree", () => {
    expect(movesFor({ s: ["commit", "propose", "evidence"] }, "s")).toEqual([
      "propose",
      "evidence",
      "question",
      "defer",
      "commit",
    ]);
  });

  it("ignores an unknown kind rather than inventing one", () => {
    expect(movesFor({ s: ["propose", "shrug"] }, "s")).not.toContain("shrug");
  });

  it("distinguishes an ungoverned seat from one narrowed to everything", () => {
    // Invisible from the move list alone, and only one of them is a decision
    // somebody made.
    expect(isGoverned({ s: ["propose"] }, "other")).toBe(false);
    expect(isGoverned({ s: [] }, "s")).toBe(false);
    expect(isGoverned({ s: ["propose"] }, "s")).toBe(true);
  });

  it("answers `may` from the resolved list", () => {
    expect(may({ s: ["object"] }, "s", "commit")).toBe(true);
    expect(may({ s: ["object"] }, "s", "propose")).toBe(false);
  });
});

describe("the numbers derived from membership", () => {
  it("budgets three turns per member", () => {
    // An opening position, a reply to the room, and a commit.
    expect(derivedTurnBudget(3)).toBe(9);
    expect(derivedTurnBudget(6)).toBe(18);
  });

  it("sets a majority that still leaves somebody outside it", () => {
    // So a decision is never contingent on the whole room agreeing.
    expect(derivedQuorum(2)).toBe(1);
    expect(derivedQuorum(3)).toBe(2);
    expect(derivedQuorum(6)).toBe(4);
  });

  it("clamps an operator's quorum into 1..=members rather than refusing it", () => {
    expect(effectiveQuorum(99, 3)).toBe(3);
    expect(effectiveQuorum(1, 3)).toBe(1);
  });

  it("honours a declared budget and falls back only when unset", () => {
    expect(effectiveTurnBudget(4, 6)).toBe(4);
    expect(effectiveTurnBudget(undefined, 6)).toBe(18);
  });

  it("distinguishes a declared number from the identical derived one", () => {
    // A declared 9 that happens to equal the derived 9 behaves differently the
    // moment somebody joins the desk, which is why the editor shows the source.
    expect(effectiveTurnBudget(9, 3)).toBe(derivedTurnBudget(3));
    expect(effectiveTurnBudget(9, 4)).not.toBe(derivedTurnBudget(4));
  });
});

describe("who can actually carry a topic", () => {
  it("counts a proposer as a supporter", () => {
    // In tinyhivemind a proposal already counts as its own author's support, so a
    // seat that may only propose can still carry a topic. Missing this
    // under-counts eligibility and refuses a table that in fact works.
    expect(eligibleSupporters({ a: ["propose"], b: ["object"] }, ["a", "b"])).toBe(1);
  });

  it("counts an ungoverned seat, which holds everything", () => {
    expect(eligibleSupporters({ b: ["object"] }, ["a", "b"])).toBe(1);
  });
});

describe("the refusals the editor mirrors", () => {
  const members = ["a", "b", "c"];

  it("accepts a workable table", () => {
    expect(
      grammarProblems({ moves: { a: ["propose"], b: ["support"] } }, members),
    ).toEqual([]);
  });

  it("refuses a table that can never reach quorum", () => {
    // The load-bearing check: every seat agreeing changes nothing if the barred
    // seats cannot deposit a supporter.
    const problems = grammarProblems(
      { quorum: 2, moves: { a: ["propose"], b: ["object"], c: ["object"] } },
      members,
    );
    expect(problems).toHaveLength(1);
    expect(problems[0].field).toBe("quorum");
    expect(problems[0].message).toContain("can never carry");
  });

  it("does not run the quorum check on a desk that will not deliberate", () => {
    expect(
      grammarProblems(
        { enabled: false, quorum: 3, moves: { a: ["object"], b: ["object"] } },
        members,
      ),
    ).toEqual([]);
  });

  it("names a seat that is not on the desk", () => {
    // The one that happens in practice: retire a seat, the table keeps its id.
    const problems = grammarProblems({ moves: { ghost: ["propose"] } }, members);
    expect(problems.some((p) => p.field === "moves.ghost")).toBe(true);
  });

  it("names an unknown move kind", () => {
    const problems = grammarProblems({ moves: { a: ["shrug"] } }, members);
    expect(problems.some((p) => p.message.includes("is not a move"))).toBe(true);
  });

  it("refuses every zero rather than clamping it", () => {
    // An operator who wrote a number meant it; substituting a different one is
    // how a desk behaves in a way its manifest does not describe.
    expect(grammarProblems({ quorum: 0 }, members).some((p) => p.field === "quorum")).toBe(
      true,
    );
    expect(
      grammarProblems({ turnBudget: 0 }, members).some((p) => p.field === "turnBudget"),
    ).toBe(true);
    expect(
      grammarProblems({ refutationCap: 0 }, members).some(
        (p) => p.field === "refutationCap",
      ),
    ).toBe(true);
  });
});

describe("the grammar hive_math_lab actually ships", () => {
  // Read off `companies/hive_math_lab/company.toml` — the one company in the
  // repo with a real installed table, and the one every live run used.
  const members = ["theorist", "programmer", "verifier", "skeptic", "brute_forcer", "archivist"];
  const moves = {
    theorist: ["support", "object", "evidence", "question", "pin", "defer"],
    programmer: ["propose", "evidence", "support", "commit", "defer"],
    verifier: ["support", "object", "evidence", "commit", "defer"],
    skeptic: ["question", "object", "evidence", "defer"],
    brute_forcer: ["support", "evidence", "object", "defer"],
    archivist: ["evidence", "question", "pin", "defer"],
  };

  it("validates against its own declared quorum of 3", () => {
    expect(grammarProblems({ quorum: 3, turnBudget: 18, moves }, members)).toEqual([]);
  });

  it("seats exactly four supporters, one clear of its quorum", () => {
    // theorist, programmer, verifier, brute_forcer. The skeptic and the archivist
    // deliberately cannot carry anything — that is the whole design.
    expect(eligibleSupporters(moves, members)).toBe(4);
  });

  it("leaves commit with the two seats that hold it, plus everyone by default", () => {
    // The fix after a live run: commit is ungated, so the skeptic can record a
    // decision it was not allowed to argue for.
    expect(may(moves, "skeptic", "commit")).toBe(true);
    expect(may(moves, "skeptic", "propose")).toBe(false);
  });
});
