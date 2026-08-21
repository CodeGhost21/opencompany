import { describe, expect, it } from "vitest";
import { EVENTS, STATUS } from "react-joyride";

import {
  EVENT_TOUR_END,
  STATUS_FINISHED,
  STATUS_SKIPPED,
  terminalOutcome,
} from "@/tour/events";

/**
 * Issue #1408 — the tour recorded nothing when it ended.
 *
 * Two separate things had to be true for that bug, and this file covers the
 * half a fast test can reach. The other half — that the handler is registered
 * on the RUN-level `onEvent` prop rather than the per-step `options.after`
 * hook — is only observable in a browser, and is pinned by
 * `test/e2e/tour-completion-persists.spec.ts`. Neither file is sufficient
 * alone: this one would have passed on the broken build, and the Playwright
 * spec cannot say *why* it broke.
 *
 * What earns its place here is the copied-constant risk. `src/tour/events.ts`
 * spells joyride's enum values out as string literals so the controller can
 * keep lazy-loading the package, and a copied constant is exactly the kind of
 * thing a minor version bump renames without anyone noticing — the failure
 * being, again, silence: `terminalOutcome` simply never matches and the tour
 * goes back to recording nothing. So the assertions below compare against the
 * real runtime enums, which this test may import freely because it is not
 * shipped in the bundle.
 */

describe("the literals copied out of react-joyride", () => {
  it("still match the package's own EVENTS.TOUR_END", () => {
    expect(EVENT_TOUR_END).toBe(EVENTS.TOUR_END);
  });

  it("still match the package's own terminal statuses", () => {
    expect(STATUS_FINISHED).toBe(STATUS.FINISHED);
    expect(STATUS_SKIPPED).toBe(STATUS.SKIPPED);
  });
});

describe("terminalOutcome", () => {
  it("records a completed run when the operator presses Done on the last stop", () => {
    expect(terminalOutcome({ type: EVENTS.TOUR_END, status: STATUS.FINISHED })).toBe("completed");
  });

  it("records a skipped run when the operator presses Skip tour mid-run", () => {
    expect(terminalOutcome({ type: EVENTS.TOUR_END, status: STATUS.SKIPPED })).toBe("skipped");
  });

  it("keeps the two outcomes distinguishable", () => {
    // They mean the same thing to `tourSeen` (both silence the welcome), but
    // they are different facts about the operator and the record keeps them
    // apart. A fix that collapsed both to `{ skipped: true }` would still pass
    // the Playwright spec — it only asks that the welcome stays away.
    const done = terminalOutcome({ type: EVENTS.TOUR_END, status: STATUS.FINISHED });
    const bailed = terminalOutcome({ type: EVENTS.TOUR_END, status: STATUS.SKIPPED });
    expect(done).not.toBe(bailed);
  });

  it("ignores the tour ending's own aftermath", () => {
    // joyride calls `controls.reset()` immediately after TOUR_END for an
    // uncontrolled tour, which emits TOUR_STATUS with the status already moved
    // on to `ready`. Nothing to record there.
    expect(terminalOutcome({ type: EVENTS.TOUR_STATUS, status: STATUS.READY })).toBeNull();
  });

  it("ignores a terminal status carried on a non-terminal event", () => {
    // The event is the gate, not the status: the status is still `finished`
    // for as long as it takes joyride to reset, and a second write would be a
    // second `seenAt` for one run.
    expect(terminalOutcome({ type: EVENTS.STEP_AFTER, status: STATUS.FINISHED })).toBeNull();
  });

  it("ignores every step-level event of a running tour", () => {
    // The per-step traffic that `options.after` sees — and that a run-level
    // handler must walk straight past. This is the shape of the bug inverted:
    // `after` saw only these, so it never recorded anything.
    for (const type of [
      EVENTS.TOUR_START,
      EVENTS.STEP_BEFORE,
      EVENTS.TOOLTIP,
      EVENTS.STEP_AFTER,
    ]) {
      expect(terminalOutcome({ type, status: STATUS.RUNNING }), `${type}`).toBeNull();
    }
  });

  it("does not record a paused run", () => {
    // `controls.stop()` (what a company switch triggers through `run={false}`)
    // parks the tour at `paused` and emits no TOUR_END. Recording there would
    // mark a tour seen that the operator never dismissed.
    expect(terminalOutcome({ type: EVENTS.TOUR_STATUS, status: STATUS.PAUSED })).toBeNull();
  });
});
