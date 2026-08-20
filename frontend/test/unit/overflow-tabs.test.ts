import { describe, expect, it } from "vitest";

import { chooseVisibleCount } from "@/lib/overflow-tabs";

describe("chooseVisibleCount", () => {
  it("shows everything, and pays nothing for More, when it all fits", () => {
    expect(chooseVisibleCount([80, 80, 80], 60, 300)).toBe(3);
    // Exactly at the edge: still no More needed.
    expect(chooseVisibleCount([100, 100, 100], 60, 300)).toBe(3);
  });

  it("reserves the More trigger's width once anything overflows", () => {
    // 4 tabs at 80px = 320, available 300. Naively 3 fit (240), but the
    // trigger itself costs 60, so the true budget is 240 — exactly 3 fit.
    expect(chooseVisibleCount([80, 80, 80, 80], 60, 300)).toBe(3);
  });

  it("shows fewer once the More trigger's cost is accounted for", () => {
    // 3 tabs at 100 = 300, available exactly 300 with nothing spare for
    // More — but total (300) <= available (300), so nothing overflows and
    // no trigger is needed at all.
    expect(chooseVisibleCount([100, 100, 100], 0, 300)).toBe(3);
    // Now a 4th tab makes it overflow, and the trigger needs room.
    expect(chooseVisibleCount([100, 100, 100, 100], 40, 300)).toBe(2);
  });

  it("always shows at least one tab, even if it alone does not fit", () => {
    expect(chooseVisibleCount([500], 0, 100)).toBe(1);
    expect(chooseVisibleCount([500, 80], 60, 100)).toBe(1);
  });

  it("returns 0 for an empty strip", () => {
    expect(chooseVisibleCount([], 60, 300)).toBe(0);
  });

  it("degrades gracefully at a narrow width — the 12-declared-list cap case", () => {
    // 15 lists (tasks + 14 more, roughly the built-in + declared cap) at a
    // uniform 90px each, a narrow 375px viewport's worth of strip.
    const widths = Array<number>(15).fill(90);
    const visible = chooseVisibleCount(widths, 70, 375);
    expect(visible).toBeGreaterThanOrEqual(1);
    expect(visible).toBeLessThan(15);
    // What's shown, plus the trigger, must actually fit.
    const used = widths.slice(0, visible).reduce((a, b) => a + b, 0);
    expect(used + 70).toBeLessThanOrEqual(375);
  });
});
