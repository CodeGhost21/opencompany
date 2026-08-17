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
 * The shape here is not new: `ArtifactsTab` and the task detail screen
 * hand-rolled it first. Both now call this instead, so the pattern lives in one
 * place and the next caller cannot get it subtly wrong — the transition-edge
 * check and the listener removal are the two halves people forget, and
 * forgetting either turns a fix into a leak.
 *
 * Deliberately framework-free: no React import, no hook rules, so it is
 * callable from an effect, from a class, or from a plain test with fake timers.
 *
 * ## It does NOT load on start
 *
 * Every caller already fetches once on mount — that is what fills the screen
 * before the first tick. Loading here as well would double every mount and
 * every company switch. What it *does* do is load once on the hidden → visible
 * *transition*, because that is the moment the view is known to be stale by up
 * to however long the tab sat in the background.
 *
 * The transition, not the state: `visibilitychange` can fire visible→visible in
 * some browsers, and a handler that only reads the live `visibilityState` would
 * treat each of those as a return from the background and pay for a full
 * refresh — two requests, for the callers that read two things. So the previous
 * state is remembered and only a genuine edge does any work.
 *
 * @param load       Called on each tick, and once on each hidden → visible
 *                   transition. Errors are the caller's to handle (all current
 *                   callers swallow transient failures to keep the last good
 *                   view).
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

  // Guarded against double-arming anyway: arming twice would leak the first
  // timer and double the cadence with no way left to clear it, and belt and
  // braces is cheap for something with no way to observe it went wrong.
  const start = () => {
    if (timer === undefined) {
      timer = window.setInterval(() => load(), intervalMs);
    }
  };

  // The remembered previous state is what makes this an edge and not a level.
  // It also carries the started-in-a-hidden-tab case: a console restored into a
  // background tab, or a route mounted behind one, arms nothing until the first
  // real transition to visible.
  let wasHidden = document.visibilityState === "hidden";

  const onVisibility = () => {
    const hidden = document.visibilityState === "hidden";
    if (hidden) {
      stop();
    } else if (wasHidden) {
      load();
      start();
    }
    wasHidden = hidden;
  };

  if (!wasHidden) start();
  document.addEventListener("visibilitychange", onVisibility);

  return () => {
    stop();
    document.removeEventListener("visibilitychange", onVisibility);
  };
}
