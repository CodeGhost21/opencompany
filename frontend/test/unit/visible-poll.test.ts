// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { startVisiblePolling } from "@/lib/visible-poll";

/**
 * The visibility-gated poller (issue #581).
 *
 * The property under test is not "it polls" — every `setInterval` does that.
 * It is that a tab nobody is looking at costs the host **nothing**, and that
 * coming back to it is worth exactly one read rather than a doubled cadence
 * that never stops again. Both halves are easy to get wrong in a way no
 * screenshot would ever show, which is why they are asserted here instead of
 * in a browser walk.
 */

let visibility: DocumentVisibilityState = "visible";

/** Flips the document's visibility and fires the event the browser would. */
function setVisibility(next: DocumentVisibilityState) {
  visibility = next;
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  visibility = "visible";
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => visibility,
  });
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("startVisiblePolling", () => {
  it("polls on cadence while the tab is visible", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    // Nothing on start: the caller owns its own mount fetch, and loading here
    // as well would double every mount.
    expect(load).toHaveBeenCalledTimes(0);

    vi.advanceTimersByTime(3000);
    expect(load).toHaveBeenCalledTimes(3);

    dispose();
  });

  it("stops polling while the tab is hidden", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    vi.advanceTimersByTime(1000);
    expect(load).toHaveBeenCalledTimes(1);

    setVisibility("hidden");
    load.mockClear();

    // THE regression. A browser throttles a hidden tab's timers, it does not
    // cancel them, so without the gate this is where the background tab keeps
    // asking the host for a board nobody is reading.
    vi.advanceTimersByTime(60_000);
    expect(load).toHaveBeenCalledTimes(0);

    dispose();
  });

  it("loads exactly once on becoming visible, then resumes the same cadence", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    setVisibility("hidden");
    vi.advanceTimersByTime(10_000);
    load.mockClear();

    setVisibility("visible");
    // One read, because the view is stale by however long the tab sat in the
    // background — not two, which is what an un-guarded re-arm plus a catch-up
    // fetch would produce.
    expect(load).toHaveBeenCalledTimes(1);

    // And the cadence is the original one. A leaked timer from the first arm
    // would show up here as two calls per interval instead of one.
    vi.advanceTimersByTime(3000);
    expect(load).toHaveBeenCalledTimes(4);

    dispose();
  });

  it("ignores redundant visible → visible events entirely", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    load.mockClear();

    // A visible → visible event, which some browsers do emit. Nothing about the
    // tab changed, so nothing is owed: not a refresh (the callers that read two
    // things would pay twice per stray event, which is the waste #581 exists to
    // remove) and not a second timer (nobody would hold a handle to it).
    setVisibility("visible");
    setVisibility("visible");
    expect(load).toHaveBeenCalledTimes(0);

    // Cadence unchanged — a leaked second timer would show as four here.
    vi.advanceTimersByTime(2000);
    expect(load).toHaveBeenCalledTimes(2);

    dispose();
  });

  it("loads once per genuine hidden → visible edge", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    load.mockClear();

    // Two real round trips through the background. Each one, and only each one,
    // is worth exactly one catch-up read.
    setVisibility("hidden");
    setVisibility("visible");
    expect(load).toHaveBeenCalledTimes(1);

    setVisibility("hidden");
    setVisibility("visible");
    expect(load).toHaveBeenCalledTimes(2);

    dispose();
  });

  it("stops the timer and drops the listener when disposed", () => {
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    dispose();

    vi.advanceTimersByTime(10_000);
    expect(load).toHaveBeenCalledTimes(0);

    // The listener half, asserted by behaviour rather than by spying: a
    // disposer that cleared the timer but left the listener attached would
    // come back to life on the next visibility flip — a leak per mount, and
    // the console remounts these on every company switch.
    setVisibility("hidden");
    setVisibility("visible");
    vi.advanceTimersByTime(10_000);
    expect(load).toHaveBeenCalledTimes(0);
  });

  it("arms nothing when started in a hidden tab", () => {
    visibility = "hidden";
    const load = vi.fn();
    const dispose = startVisiblePolling(load, 1000);

    vi.advanceTimersByTime(10_000);
    expect(load).toHaveBeenCalledTimes(0);

    // …and comes up on the first look, rather than waiting a full interval.
    setVisibility("visible");
    expect(load).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1000);
    expect(load).toHaveBeenCalledTimes(2);

    dispose();
  });
});
