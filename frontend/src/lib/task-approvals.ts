import type { TaskApproval } from "@/api/tasks";

/**
 * What the task card says about approvals (issue #468).
 *
 * `null` when the card is not waiting on anything — the caller renders nothing
 * at all rather than a "no approvals" line, which is the clutter the removed
 * Approvals tab was made of.
 */
export interface PendingApprovalWait {
  /** How many of this task's approvals are still parked. */
  count: number;
  /** Epoch-millis the *oldest* parked one arrived. */
  since: number;
  /** How long the card has been stopped, measured to the caller's clock. */
  waited: number;
}

/**
 * Derives the card's one approval line from this task's approvals.
 *
 * Lives here rather than inline in the view for the reason every other
 * derivation in `lib/` does: getting it wrong produces a card that looks
 * completely normal while saying the wrong thing. A card that reports the
 * *newest* park reads as freshly blocked no matter how long it has actually sat
 * there, and nothing about the rendering would look broken.
 *
 * Three decisions worth stating, because each is a way this could be silently
 * wrong:
 *
 * - **Only `pending` counts.** `approved`, `denied` and `expired` are
 *   resolutions; they are already on the timeline with their own waited span,
 *   and counting them here would rebuild the removed tab one row at a time.
 * - **The oldest park wins**, not the newest. It is how long this card has
 *   really been stopped. With the newest, a second effect parking behind the
 *   first would reset a clock that should be climbing.
 * - **`waited` is clamped at zero.** `atMillis` comes from the host and `now`
 *   from the browser; a clock skew of a few seconds must not render "waiting
 *   for -3s".
 */
export function pendingApprovalWait(
  approvals: readonly TaskApproval[],
  now: number,
): PendingApprovalWait | null {
  let count = 0;
  let since = Number.POSITIVE_INFINITY;
  for (const a of approvals) {
    if (a.status !== "pending") continue;
    count += 1;
    if (a.atMillis < since) since = a.atMillis;
  }
  if (count === 0) return null;
  return { count, since, waited: Math.max(0, now - since) };
}
