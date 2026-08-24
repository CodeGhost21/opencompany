// The three formatters the step timeline and every surface that renders one
// share (issue #1573).
//
// They lived inside `TaskDetailView.tsx` while the timeline had exactly one
// caller. It now has two — a card's attempts and a teammate's run history — and
// a leaf module is what keeps the second one from importing the first, which
// would be a cycle: `TaskDetailView` renders the shared timeline, so the shared
// timeline cannot reach back into it.

/** `1h 04m 09s` / `4m 09s` / `9s`. */
export function formatDuration(millis: number): string {
  const s = Math.floor(millis / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0)
    return `${h}h ${String(m).padStart(2, "0")}m ${String(sec).padStart(2, "0")}s`;
  if (m > 0) return `${m}m ${String(sec).padStart(2, "0")}s`;
  return `${sec}s`;
}

/** A wall-clock time of day, in the viewer's locale. */
export function timeOf(at: number): string {
  return new Date(at).toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  });
}

/**
 * The pixel height of a waiting band for a span of `millis` (#305).
 *
 * Sub-linear on purpose. The point of the band is that a four-hour wait and a
 * four-second wait must not look alike, but a linear scale makes the short one
 * invisible and the long one taller than the screen. A log curve with a floor
 * and a cap keeps both on the page: ~14px at four seconds, ~112px at four
 * hours. Past the cap the printed duration inside the band carries the
 * precision the height no longer can.
 */
export function waitingBandHeight(millis: number): number {
  const minutes = Math.max(0, millis) / 60_000;
  const raw = 12 + 26 * Math.log2(1 + minutes);
  return Math.round(Math.min(112, Math.max(12, raw)));
}
