// How a run reads at a glance: its terminal status, its delivery counts, and
// how long ago it happened.
//
// Extracted from `WorkflowsView.tsx` (issue #303) because the workflow cards now
// need the SAME reading the history rows and the last-run chip use. Two
// implementations of "is this run healthy?" would drift, and the card grid is
// precisely the surface where a wrong green dot is most costly — it is the one
// an operator scans instead of opening anything.

import type { DeliveryReport, WorkflowRunOutcome } from "@/api/workflows";

/**
 * The `pending` delivery status — a report parked for an operator's approval —
 * is added to `DeliveryStatus` by issue #227. It is typed `string` rather than
 * written as a literal so these comparisons compile both before and after that
 * lands: against today's union TypeScript would reject the literal as a
 * no-overlap comparison, and once the union widens this keeps behaving
 * identically. The runtime check is what matters — the host can already send a
 * status this console's type doesn't name yet.
 */
const PENDING_STATUS: string = "pending";

/** Reports that did NOT reach their destination **and will not without a
 * change** — the number worth acting on. `pending` is excluded on purpose: it
 * is a report parked for an operator's approval, so counting it here would
 * badge a working approvals queue as a failure. */
export function undeliveredCount(deliveries: DeliveryReport[]): number {
  return deliveries.filter((d) => d.status !== "sent" && d.status !== PENDING_STATUS).length;
}

/** Reports waiting on an operator's verdict rather than on a fix. */
export function pendingCount(deliveries: DeliveryReport[]): number {
  return deliveries.filter((d) => d.status === PENDING_STATUS).length;
}

/** A compact "N minutes ago" for a run timestamp — enough to tell last night's
 * scheduled run from the one just clicked, without a date library. */
export function relativeTime(atMillis: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - atMillis) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

/** The status dot for a whole run.
 *
 * Every arm returns one of the console's five run states, so a dot here means
 * exactly what the same dot means on the task board and in the runs table:
 * running, blocked on a human, done, failed, or idle. See
 * docs/design-system/color.md.
 *
 * **A run still in flight is checked FIRST**, ahead of every terminal reading.
 * A running run has no `error`, no `cancelled` and no deliveries yet, so
 * without this arm it falls all the way through to the green "ok" — and every
 * caller that trusts this function (the last-run chip, the history rows, the
 * cards) paints a run that has not finished as one that succeeded. That is a
 * claim the host has not made.
 *
 * The label is not decoration: it ships with the dot at every call site,
 * because roughly 1 in 12 men cannot separate the red/green pair and a bare
 * dot puts the whole signal on hue.
 */
export function runTone(run: WorkflowRunOutcome): { dot: string; label: string } {
  if (isRunning(run)) return { dot: "animate-pulse bg-status-running", label: "running" };
  if (run.error) return { dot: "bg-status-failed", label: "failed" };
  // Issue #383: checked before the delivery reads, and deliberately NOT red. A
  // stop somebody asked for is not a fault, and a cancelled run has no
  // deliveries to weigh anyway — so without this arm it would fall through to
  // the green "ok" and read as a clean success. Idle is the state for "nothing
  // is happening and nothing went wrong".
  if (run.cancelled) return { dot: "bg-status-idle", label: "stopped" };
  if (undeliveredCount(run.deliveries) > 0)
    return { dot: "bg-status-failed", label: "not delivered" };
  // Blocked, not running. This was the running colour, which said "the machine
  // is working on it" about the one state that means the opposite: it is
  // parked until a human decides. Amber is the colour that gets looked at.
  if (pendingCount(run.deliveries) > 0)
    return { dot: "bg-status-blocked", label: "awaiting approval" };
  return { dot: "bg-status-done", label: "ok" };
}

/**
 * A run that is still walking its graph.
 *
 * Its own reading, ahead of {@link runTone}: an in-flight run has not failed and
 * has not succeeded, and painting it with either colour is a claim the host has
 * not made yet.
 */
export function isRunning(run: WorkflowRunOutcome): boolean {
  return run.running === true;
}
