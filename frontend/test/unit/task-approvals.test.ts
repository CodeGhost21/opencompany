import { describe, expect, it } from "vitest";

import { pendingApprovalWait } from "@/lib/task-approvals";
import type { TaskApproval, TaskApprovalStatus } from "@/api/tasks";

/**
 * The task card's one approval line (issue #468).
 *
 * #468 removed the card's Approvals tab, which could list every sign-off but
 * could not decide any of them. What had to survive the removal is the
 * *signal*: a card stalled behind an approval must still say so, or the screen
 * that exists to answer "why is this stuck" quietly stops answering it.
 *
 * This derivation is the whole of that signal, which is why it is tested rather
 * than left inline — each way it can be wrong produces a card that looks
 * entirely normal while misreporting.
 */

function approval(over: Partial<TaskApproval> = {}): TaskApproval {
  return { id: "appr-1", atMillis: 1_000, status: "pending", ...over };
}

describe("pendingApprovalWait", () => {
  it("reports nothing when the task has no approvals at all", () => {
    expect(pendingApprovalWait([], 5_000)).toBeNull();
  });

  /**
   * The removed tab rendered a "No approvals for this task" empty state on
   * every card that never parked one. Returning `null` rather than a zero
   * count is what lets the caller render nothing instead of reintroducing it.
   */
  it.each<TaskApprovalStatus>(["approved", "denied", "expired"])(
    "reports nothing when the only approval is %s",
    (status) => {
      expect(pendingApprovalWait([approval({ status })], 5_000)).toBeNull();
    },
  );

  it("reports a single pending approval and how long it has waited", () => {
    const wait = pendingApprovalWait([approval({ atMillis: 1_000 })], 4_500);
    expect(wait).toEqual({ count: 1, since: 1_000, waited: 3_500 });
  });

  /**
   * The one that matters most. A card with two parked effects has been stopped
   * since the *first* of them; measuring from the newest would reset a clock
   * that should be climbing, every time another effect parks behind it — and
   * the card would look freshly blocked no matter how long it had really sat.
   */
  it("measures from the oldest park, not the newest", () => {
    const wait = pendingApprovalWait(
      [
        approval({ id: "new", atMillis: 9_000 }),
        approval({ id: "old", atMillis: 2_000 }),
        approval({ id: "mid", atMillis: 5_000 }),
      ],
      10_000,
    );
    expect(wait).toEqual({ count: 3, since: 2_000, waited: 8_000 });
  });

  it("counts only the pending ones when the task has a mixed history", () => {
    const wait = pendingApprovalWait(
      [
        // Resolved long before the pending one, and must not be what the
        // clock is measured from.
        approval({ id: "done", atMillis: 100, status: "approved" }),
        approval({ id: "gone", atMillis: 200, status: "expired" }),
        approval({ id: "live", atMillis: 6_000, status: "pending" }),
      ],
      6_750,
    );
    expect(wait).toEqual({ count: 1, since: 6_000, waited: 750 });
  });

  /**
   * `atMillis` is the host's clock and `now` is the browser's. A few seconds of
   * skew is ordinary and must not render as "waiting for -3s".
   */
  it("clamps a negative wait to zero when the browser clock lags the host", () => {
    const wait = pendingApprovalWait([approval({ atMillis: 10_000 })], 7_000);
    expect(wait).toEqual({ count: 1, since: 10_000, waited: 0 });
  });
});
