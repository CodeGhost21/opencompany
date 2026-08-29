import { describe, expect, it } from "vitest";

import { hasOtherOpenTurns, type OpenTurn } from "@/lib/live-reply";

/**
 * The guard on clearing a thread's live tool rows (PR #1904 review).
 *
 * Two failures sit on either side of it, and the id-exclusion is what avoids
 * both: clearing while another turn runs erases the rows of work in flight,
 * and skipping the clear because the *settled* turn still appears in a stale
 * snapshot leaves a finished turn's rows on screen with nothing to remove
 * them — a failed turn has no `agent_reply` to fall back on.
 */
describe("deciding whether a thread still has work in flight", () => {
  const turn = (turnId: string): OpenTurn => ({ turnId, queued: false });

  it("ignores the turn that just settled, even on a snapshot that still lists it", () => {
    // The race: `getChatHistory` resolves before React commits the
    // `setOpenTurns` that dropped this turn, so the ref still holds it.
    const stale = { eng: [turn("t1")] };
    expect(hasOtherOpenTurns(stale, "eng", "t1")).toBe(false);
  });

  it("says so when a newer turn is still running", () => {
    const withQueued = { eng: [turn("t1"), turn("t2")] };
    expect(hasOtherOpenTurns(withQueued, "eng", "t1")).toBe(true);
  });

  it("agrees once the snapshot has caught up", () => {
    expect(hasOtherOpenTurns({}, "eng", "t1")).toBe(false);
    expect(hasOtherOpenTurns({ eng: [] }, "eng", "t1")).toBe(false);
  });

  it("excludes nothing when no turn settled — the mention re-read's shape", () => {
    expect(hasOtherOpenTurns({ eng: [turn("t1")] }, "eng")).toBe(true);
  });

  it("is per thread", () => {
    expect(hasOtherOpenTurns({ other: [turn("t9")] }, "eng", "t1")).toBe(false);
  });
});
