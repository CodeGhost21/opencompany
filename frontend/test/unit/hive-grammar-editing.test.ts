import { describe, expect, it } from "vitest";

import {
  GATEABLE_KINDS,
  eligibleSupporters,
  grammarProblems,
  isGoverned,
  movesFor,
  type MoveKind,
} from "@/lib/hive/grammar";

/**
 * Editing a table, as the matrix does it.
 *
 * The editor's one non-obvious operation is **materialising** a row: a seat the
 * table does not name holds every move, and ticking a box for it has to write
 * the whole row rather than a single kind — otherwise the first click silently
 * takes eight moves away, which is the opposite of what the operator did.
 */

/** The matrix's toggle, extracted so it can be tested without a DOM. */
function toggle(
  moves: Record<string, string[]>,
  agentId: string,
  kind: MoveKind,
  next: boolean,
): Record<string, string[]> {
  const table = { ...moves };
  const current = new Set<string>(table[agentId] ?? GATEABLE_KINDS);
  if (next) current.add(kind);
  else current.delete(kind);
  table[agentId] = GATEABLE_KINDS.filter((k) => current.has(k));
  return table;
}

describe("materialising an ungoverned seat", () => {
  it("takes away one move, not eight, on the first click", () => {
    // The trap: seeding the new row from `[]` instead of "everything" would
    // read as "narrow this seat to nothing" when the operator unticked one box.
    const next = toggle({}, "scout", "propose", false);
    expect(next.scout).toEqual(["support", "object", "refute", "evidence", "pin"]);
    expect(isGoverned(next, "scout")).toBe(true);
  });

  it("keeps the ungated three after narrowing", () => {
    const next = toggle({}, "scout", "propose", false);
    const held = movesFor(next, "scout");
    expect(held).toContain("commit");
    expect(held).toContain("question");
    expect(held).toContain("defer");
    expect(held).not.toContain("propose");
  });

  it("never writes an ungated kind into the table", () => {
    // `commit`/`question`/`defer` are accepted and ignored by the host, so
    // storing them would be noise that reads as a decision.
    const next = toggle({ scout: ["propose"] }, "scout", "support", true);
    expect(next.scout).toEqual(["propose", "support"]);
  });
});

describe("the refusal the editor must reproduce", () => {
  const members = ["theorist", "programmer", "verifier", "skeptic", "brute_forcer", "archivist"];

  it("goes from valid to unreachable as support is removed", () => {
    // Exactly the interaction the panel was driven through: hive_math_lab's own
    // table seats four supporters against a quorum of three; take three of them
    // away and the room can never decide anything.
    let moves: Record<string, string[]> = {
      theorist: ["support", "object", "evidence", "pin"],
      programmer: ["propose", "evidence", "support"],
      verifier: ["support", "object", "evidence"],
      skeptic: ["object", "evidence"],
      brute_forcer: ["support", "evidence", "object"],
      archivist: ["evidence", "pin"],
    };
    expect(eligibleSupporters(moves, members)).toBe(4);
    expect(grammarProblems({ quorum: 3, moves }, members)).toEqual([]);

    for (const seat of ["theorist", "verifier", "brute_forcer"]) {
      moves = toggle(moves, seat, "support", false);
    }
    expect(eligibleSupporters(moves, members)).toBe(1);
    const problems = grammarProblems({ quorum: 3, moves }, members);
    expect(problems).toHaveLength(1);
    expect(problems[0].message).toContain("can never carry");
  });
});
