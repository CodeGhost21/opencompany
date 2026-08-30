import { describe, expect, it } from "vitest";

import { batchPositions } from "@/lib/approval-wording";

/**
 * "N of M from the same turn" on the Approvals page (#1289).
 *
 * The line used to hardcode its numerator: every sibling of a batch read
 * "1 of M", so a two-card turn showed two "1 of 2"s and never a "2 of 2".
 * These pin the numerator to the card's real place in the batch, and pin index
 * and total to one walk of one list so a focus-narrowed view cannot count them
 * over different sets.
 */

type Row = { id: string; batch?: string | null };

describe("batchPositions (#1289)", () => {
  it("numbers each card 1..M within its turn's batch, not 1 every time", () => {
    const rows: Row[] = [
      { id: "a1", batch: "turn-1" },
      { id: "a2", batch: "turn-1" },
    ];

    const pos = batchPositions(rows);

    // The bug: both were "1 of 2". Now the second is "2 of 2".
    expect(pos.get("a1")).toEqual({ index: 1, total: 2 });
    expect(pos.get("a2")).toEqual({ index: 2, total: 2 });
  });

  it("counts each turn's batch independently and interleaved", () => {
    const rows: Row[] = [
      { id: "a1", batch: "turn-1" },
      { id: "b1", batch: "turn-2" },
      { id: "a2", batch: "turn-1" },
      { id: "a3", batch: "turn-1" },
      { id: "b2", batch: "turn-2" },
    ];

    const pos = batchPositions(rows);

    expect(pos.get("a1")).toEqual({ index: 1, total: 3 });
    expect(pos.get("a2")).toEqual({ index: 2, total: 3 });
    expect(pos.get("a3")).toEqual({ index: 3, total: 3 });
    expect(pos.get("b1")).toEqual({ index: 1, total: 2 });
    expect(pos.get("b2")).toEqual({ index: 2, total: 2 });
  });

  it("omits approvals with no batch — their line is not shown", () => {
    const rows: Row[] = [
      { id: "solo" },
      { id: "none", batch: null },
      { id: "a1", batch: "turn-1" },
    ];

    const pos = batchPositions(rows);

    expect(pos.has("solo")).toBe(false);
    expect(pos.has("none")).toBe(false);
    // A batch of one is still a real position — the caller only renders the
    // line when total > 1, so a lone batched row carries { index: 1, total: 1 }.
    expect(pos.get("a1")).toEqual({ index: 1, total: 1 });
  });

  it("index and total share the pending list, so both shrink together", () => {
    // #842: the caller passes only what is still pending, so a decided sibling
    // is simply absent — the survivors renumber over the smaller set rather
    // than one half counting the decided row and the other not.
    const afterOneDecided: Row[] = [
      { id: "a2", batch: "turn-1" },
      { id: "a3", batch: "turn-1" },
    ];

    const pos = batchPositions(afterOneDecided);

    expect(pos.get("a2")).toEqual({ index: 1, total: 2 });
    expect(pos.get("a3")).toEqual({ index: 2, total: 2 });
  });
});
