import { describe, expect, it } from "vitest";

import { computeWindow, rowOffsets, totalHeight } from "@/components/virtual-window";

describe("rowOffsets / totalHeight", () => {
  it("accumulates heights plus the inter-row gap", () => {
    const offsets = rowOffsets([100, 100, 100], 10);
    expect(offsets).toEqual([0, 110, 220, 330]);
    // Total drops the trailing gap: three 100px rows + two 10px gaps.
    expect(totalHeight(offsets, 10)).toBe(320);
  });

  it("is zero-length safe", () => {
    expect(rowOffsets([], 10)).toEqual([0]);
    expect(totalHeight([0], 10)).toBe(0);
  });
});

describe("computeWindow", () => {
  const uniform = (count: number, h: number, gap: number) =>
    rowOffsets(new Array<number>(count).fill(h), gap);

  it("renders only the rows intersecting the viewport, never the whole set", () => {
    const count = 1000;
    const offsets = uniform(count, 100, 0);
    const win = computeWindow({
      scrollTop: 0,
      viewportHeight: 500,
      offsets,
      count,
      overscan: 2,
    });
    // 5 rows fit in 500px + 2 overscan below; the window is a tiny slice of 1000.
    expect(win.startIndex).toBe(0);
    expect(win.endIndex).toBe(7);
    expect(win.endIndex - win.startIndex + 1).toBeLessThan(count);
  });

  it("tracks the viewport as it scrolls and pads by overscan on both sides", () => {
    const count = 1000;
    const offsets = uniform(count, 100, 0);
    const win = computeWindow({
      scrollTop: 10_000,
      viewportHeight: 500,
      offsets,
      count,
      overscan: 3,
    });
    // Rows 100..105 span the viewport; overscan widens to 97..108.
    expect(win.startIndex).toBe(97);
    expect(win.endIndex).toBe(108);
  });

  it("clamps to the population at the ends", () => {
    const count = 10;
    const offsets = uniform(count, 100, 0);
    const win = computeWindow({
      scrollTop: 5000,
      viewportHeight: 500,
      offsets,
      count,
      overscan: 4,
    });
    expect(win.startIndex).toBeGreaterThanOrEqual(0);
    expect(win.endIndex).toBe(9);
  });

  it("handles a single item", () => {
    const win = computeWindow({
      scrollTop: 0,
      viewportHeight: 500,
      offsets: rowOffsets([80], 0),
      count: 1,
      overscan: 4,
    });
    expect(win).toEqual({ startIndex: 0, endIndex: 0 });
  });

  it("returns an empty window for an empty list", () => {
    const win = computeWindow({
      scrollTop: 0,
      viewportHeight: 500,
      offsets: [0],
      count: 0,
      overscan: 4,
    });
    expect(win.endIndex).toBeLessThan(win.startIndex);
  });

  it("respects variable row heights when choosing the first visible row", () => {
    // A tall first row pushes the viewport start past row 0 by itself.
    const offsets = rowOffsets([1000, 100, 100, 100, 100], 0);
    const win = computeWindow({
      scrollTop: 1050,
      viewportHeight: 100,
      offsets,
      count: 5,
      overscan: 0,
    });
    // Offset 1050 lands inside row 1 (1000..1100); bottom 1150 lands in row 2.
    expect(win.startIndex).toBe(1);
    expect(win.endIndex).toBe(2);
  });
});
