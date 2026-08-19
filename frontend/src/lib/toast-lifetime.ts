// A wall-clock ceiling on how long one toast may stay on screen.
//
// Issue #933. `sonner`'s auto-dismiss is not a deadline — it is a timer the
// library *pauses*, and it pauses on three conditions held in `Toaster` state:
// the toaster being `expanded`, a pointer `interacting` with it, and the
// document being hidden. Two of those have no path back on their own:
//
//   * `expanded` is set by `mouseenter`/`mousemove` **and by sonner's built-in
//     Alt+T hotkey**, which also focuses the toast list. It is cleared only by a
//     `mouseleave`, by Escape while focus is inside the list, or by the toast
//     array changing. Expand it with the hotkey while the pointer is elsewhere
//     and no `mouseleave` can ever arrive: with a single toast up, nothing
//     changes the array either, so the timer stays paused indefinitely.
//   * `interacting` is set on `pointerdown` and cleared on `pointerup` *on the
//     toaster*. Pointer capture covers the ordinary drag, but not a toast that
//     opted out of dismissal, where sonner skips the capture and keeps the flag.
//
// A latched toast then rides over every view until someone clicks its ×, which
// is what #933 reported: nine minutes and four navigations of "Starting the
// product tour.".
//
// So the console does not rely on that timer alone. This module is the pure
// half of the guard in `components/ui/sonner.tsx`: it accumulates each toast's
// *visible* time and names the ones that have outstayed their duration. The two
// intentional pauses are preserved — a toast under the pointer is being read,
// and a toast in a hidden tab has not been seen yet — so the ceiling only ever
// removes a toast nobody is looking at.

/** sonner's own default lifetime (`TOAST_LIFETIME`), which the console does not override. */
export const DEFAULT_TOAST_DURATION_MS = 4000;

/**
 * How far past its duration a toast may go before the guard steps in.
 *
 * sonner's timer should be what dismisses a toast in the normal case; this is a
 * backstop, and firing it early would race the exit animation and swallow
 * `onAutoClose`. Long enough to lose that race, short enough that a wedged
 * toast is a blip rather than a fixture.
 */
export const DISMISSAL_GRACE_MS = 2000;

/** One toast the guard is watching, with the visible time it has accumulated. */
export interface TrackedToast {
  id: string | number;
  /** Milliseconds this toast has been on screen *while the tab was visible*. */
  visibleMs: number;
  /** The toast's own duration, when it set one. */
  duration?: number;
}

/** What the guard can observe about the moment it is deciding in. */
export interface ToastEnvironment {
  /** Is the pointer actually over the toaster? Not what sonner believes — what is true. */
  hovered: boolean;
  /** Is the tab hidden, so the operator has not seen these toasts yet? */
  documentHidden: boolean;
}

export interface SweepOptions {
  defaultDurationMs?: number;
  graceMs?: number;
}

export interface SweepResult {
  /** The tracked set advanced by this tick. */
  next: TrackedToast[];
  /** Ids to dismiss, because their time is up and nobody is reading them. */
  overdue: (string | number)[];
}

/**
 * Advance the guard by one tick and say which toasts are overdue.
 *
 * The clock only runs while the tab is visible, so a toast raised against a
 * backgrounded console still gets its full life once the operator returns —
 * and a throttled background interval can only under-count, never over-count.
 */
export function sweepToasts(
  tracked: TrackedToast[],
  tickMs: number,
  env: ToastEnvironment,
  options: SweepOptions = {},
): SweepResult {
  const defaultDuration = options.defaultDurationMs ?? DEFAULT_TOAST_DURATION_MS;
  const grace = options.graceMs ?? DISMISSAL_GRACE_MS;

  // A hidden tab stops the clock rather than pausing the decision: resuming from
  // a half-spent life would dismiss a toast the operator has only just seen.
  if (env.documentHidden) return { next: tracked, overdue: [] };

  const next = tracked.map((t) => ({ ...t, visibleMs: t.visibleMs + tickMs }));

  // Hovering is the one pause the console keeps: the pointer being there is the
  // operator reading the toast, and yanking it mid-read is the opposite of the
  // fix. The ceiling applies the moment they move away.
  if (env.hovered) return { next, overdue: [] };

  const overdue = next
    .filter((t) => t.duration !== Number.POSITIVE_INFINITY)
    .filter((t) => t.visibleMs >= (t.duration ?? defaultDuration) + grace)
    .map((t) => t.id);

  return { next, overdue };
}

/**
 * Reconcile the tracked set against what is actually on screen.
 *
 * Accumulated visible time survives — a toast whose content is updated in place
 * keeps its age, so an updater cannot keep it alive by refreshing it.
 */
export function reconcileTracked(
  tracked: TrackedToast[],
  live: { id: string | number; duration?: number }[],
): TrackedToast[] {
  const seen = new Map(tracked.map((t) => [t.id, t]));
  return live.map((t) => ({
    id: t.id,
    visibleMs: seen.get(t.id)?.visibleMs ?? 0,
    duration: t.duration,
  }));
}
