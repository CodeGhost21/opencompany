/**
 * Interval polling that stops while the tab is hidden (issue #581).
 *
 * The console's background reads — company status, the task board, standing
 * permissions — were armed with a bare `setInterval` that never stopped. A
 * browser throttles a hidden tab's timers, it does not cancel them, so a
 * console left open in a background tab kept asking the host for the board
 * forever, and every extra tab multiplied the steady-state request volume of a
 * company nobody was looking at.
 *
 * The shape here is not new: `ArtifactsTab` and the task detail screen already
 * hand-rolled it. This is that pattern extracted so a fourth caller cannot get
 * it subtly wrong — the double-arm guard and the listener removal are the two
 * halves people forget, and forgetting either turns a fix into a leak.
 *
 * Deliberately framework-free: no React import, no hook rules, so it is
 * callable from an effect, from a class, or from a plain test with fake timers.
 *
 * ## It does NOT load on start
 *
 * Every caller already fetches once on mount — that is what fills the screen
 * before the first tick. Loading here as well would double every mount and
 * every company switch. What it *does* do is load once on the hidden → visible
 * transition, because that is the moment the view is known to be stale by up to
 * however long the tab sat in the background.
 *
 * @param load       Called on each tick, and once when the tab becomes visible.
 *                   Errors are the caller's to handle (all current callers
 *                   swallow transient failures to keep the last good view).
 * @param intervalMs The cadence, in milliseconds, while the tab is visible.
 * @returns A disposer that clears the timer **and** removes the
 *          `visibilitychange` listener. Call it from effect cleanup; not
 *          calling it leaks a listener per mount.
 */
export function startVisiblePolling(load: () => void, intervalMs: number): () => void {
  let timer: number | undefined;

  const stop = () => {
    if (timer !== undefined) {
      window.clearInterval(timer);
      timer = undefined;
    }
  };

  // Guarded against double-arming: `visibilitychange` can fire visible→visible
  // in some browsers, and arming twice would leak the first timer and double
  // the cadence with no way left to clear it.
  const start = () => {
    if (timer === undefined) {
      timer = window.setInterval(() => load(), intervalMs);
    }
  };

  const onVisibility = () => {
    if (document.visibilityState === "hidden") {
      stop();
    } else {
      load();
      start();
    }
  };

  // Started in a hidden tab — a console restored into a background tab, or a
  // route mounted behind one — arms nothing until it is actually looked at.
  if (document.visibilityState !== "hidden") start();
  document.addEventListener("visibilitychange", onVisibility);

  return () => {
    stop();
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
