import { describe, expect, it } from "vitest";

import {
  DEFAULT_TOAST_DURATION_MS,
  DISMISSAL_GRACE_MS,
  reconcileTracked,
  sweepToasts,
  type ToastEnvironment,
  type TrackedToast,
} from "@/lib/toast-lifetime";

/**
 * The console's ceiling on a toast's life (issue #933).
 *
 * The bug was a toast that never left: sonner's auto-dismiss is a timer it
 * *pauses* on `expanded` / `interacting` / hidden-document, and the first two can
 * latch with no way back — sonner's Alt+T hotkey expands the toaster and focuses
 * it, and `expanded` then waits for a `mouseleave` that a pointer which was
 * never there cannot deliver. The reported toast sat over four views for nine
 * minutes with only its × working.
 *
 * These tests pin the guard's decision, which is the whole of the fix worth
 * pinning fast: *when* a toast is overdue, and — just as important — the three
 * cases where it must be left alone. The browser half (the real hotkey, a real
 * hover) is `test/e2e/toast-dismissal.spec.ts`.
 */

const AWAKE: ToastEnvironment = { hovered: false, documentHidden: false };
const TICK = 500;

/** Run the guard until it reports something overdue, or give up. */
function runUntilOverdue(
  tracked: TrackedToast[],
  env: ToastEnvironment,
  maxMs = 60_000,
): { elapsed: number; overdue: (string | number)[] } {
  let current = tracked;
  for (let elapsed = TICK; elapsed <= maxMs; elapsed += TICK) {
    const { next, overdue } = sweepToasts(current, TICK, env);
    current = next;
    if (overdue.length > 0) return { elapsed, overdue };
  }
  return { elapsed: maxMs, overdue: [] };
}

describe("sweepToasts", () => {
  it("dismisses a toast once its duration plus the grace period is up", () => {
    const { elapsed, overdue } = runUntilOverdue([{ id: "a", visibleMs: 0 }], AWAKE);
    expect(overdue).toEqual(["a"]);
    expect(elapsed).toBe(DEFAULT_TOAST_DURATION_MS + DISMISSAL_GRACE_MS);
  });

  it("leaves a toast alone until then, so sonner's own timer is what normally fires", () => {
    // The guard is a backstop. Reporting overdue before the grace has elapsed
    // would race sonner's exit animation and swallow its `onAutoClose`.
    const { overdue } = sweepToasts(
      [{ id: "a", visibleMs: DEFAULT_TOAST_DURATION_MS }],
      TICK,
      AWAKE,
    );
    expect(overdue).toEqual([]);
  });

  it("honours a toast's own longer duration", () => {
    const { elapsed } = runUntilOverdue([{ id: "a", visibleMs: 0, duration: 20_000 }], AWAKE);
    expect(elapsed).toBe(20_000 + DISMISSAL_GRACE_MS);
  });

  it("never dismisses a toast that asked to be permanent", () => {
    const { overdue } = runUntilOverdue(
      [{ id: "a", visibleMs: 0, duration: Number.POSITIVE_INFINITY }],
      AWAKE,
    );
    expect(overdue).toEqual([]);
  });

  it("holds a hovered toast for as long as the pointer is on it", () => {
    // The one pause the console keeps: the pointer being there is somebody
    // reading, and this fix must not yank a toast out from under them.
    const hovered = { hovered: true, documentHidden: false };
    const { overdue } = runUntilOverdue([{ id: "a", visibleMs: 0 }], hovered);
    expect(overdue).toEqual([]);
  });

  it("still ages a hovered toast, so it goes the moment the pointer leaves", () => {
    const hovered = { hovered: true, documentHidden: false };
    let tracked: TrackedToast[] = [{ id: "a", visibleMs: 0 }];
    for (let i = 0; i < 40; i++) tracked = sweepToasts(tracked, TICK, hovered).next;
    // 20s of hovering: nothing dismissed, but the clock ran.
    expect(tracked[0].visibleMs).toBe(20_000);
    expect(sweepToasts(tracked, TICK, AWAKE).overdue).toEqual(["a"]);
  });

  it("stops the clock while the tab is hidden, so an unseen toast keeps its full life", () => {
    let tracked: TrackedToast[] = [{ id: "a", visibleMs: 0 }];
    const hidden = { hovered: false, documentHidden: true };
    for (let i = 0; i < 200; i++) tracked = sweepToasts(tracked, TICK, hidden).next;
    expect(tracked[0].visibleMs).toBe(0);

    // Back in the foreground it gets the whole window, not the remainder of one
    // it spent in a tab nobody was looking at.
    const { elapsed } = runUntilOverdue(tracked, AWAKE);
    expect(elapsed).toBe(DEFAULT_TOAST_DURATION_MS + DISMISSAL_GRACE_MS);
  });

  it("reports every overdue toast, not just the front one", () => {
    const overdue = sweepToasts(
      [
        { id: "old", visibleMs: 30_000 },
        { id: "older", visibleMs: 90_000 },
        { id: "fresh", visibleMs: 0 },
      ],
      TICK,
      AWAKE,
    ).overdue;
    expect(overdue).toEqual(["old", "older"]);
  });
});

describe("reconcileTracked", () => {
  it("starts new toasts at zero and forgets ones that have left", () => {
    const tracked = reconcileTracked([{ id: "gone", visibleMs: 3000 }], [{ id: "new" }]);
    expect(tracked).toEqual([{ id: "new", visibleMs: 0, duration: undefined }]);
  });

  it("keeps the accumulated age of a toast that is updated in place", () => {
    // sonner's update path replaces the toast object for the same id. If that
    // reset the age, anything refreshing a toast would keep it alive forever —
    // the very shape of bug this guard exists to close.
    const tracked = reconcileTracked(
      [{ id: "a", visibleMs: 3500 }],
      [{ id: "a", duration: 8000 }],
    );
    expect(tracked).toEqual([{ id: "a", visibleMs: 3500, duration: 8000 }]);
  });
});
