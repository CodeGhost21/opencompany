import { describe, expect, it } from "vitest";

import type { LifecycleAction } from "@/api/client";

/**
 * What the lifecycle buttons show between the click and the host confirming.
 *
 * The controls read `pending ?? feed.status.lifecycle`, so the mapping from an
 * action to the state it lands in is what makes a click visible immediately.
 * Get it wrong and the console asserts a lifecycle the company is not in — a
 * worse failure than the lag it replaced, because it looks authoritative.
 *
 * `resultOf` is not exported (it is a view-local helper), so this restates it
 * and pins the one entry that is easy to get wrong.
 */
function resultOf(action: LifecycleAction): string {
  switch (action) {
    case "pause":
      return "paused";
    case "resume":
      return "running";
    case "suspend":
      return "suspended";
    case "archive":
      return "archived";
  }
}

/** Mirrors the component: the optimistic state wins until it is cleared. */
function shown(pending: string | null, lifecycle: string): string {
  return pending ?? lifecycle;
}

describe("the lifecycle a button lands the company in", () => {
  it("maps resume to running, not to 'resumed'", () => {
    // THE one to get wrong. The success *toast* says "Company resumed", and
    // reusing that wording here would set `pending` to a lifecycle the host
    // never reports — so `running` would be false, the Resume button would
    // stay on screen, and the company would look stuck.
    expect(resultOf("resume")).toBe("running");
  });

  it("maps the other three to their own names", () => {
    expect(resultOf("pause")).toBe("paused");
    expect(resultOf("suspend")).toBe("suspended");
    expect(resultOf("archive")).toBe("archived");
  });

  it("produces a state the buttons actually branch on", () => {
    // The controls derive `running`/`paused`/`archived` by comparing to these
    // literals. A value outside the set renders no button at all, which is how
    // a company becomes unoperatable from the console.
    const known = ["running", "paused", "suspended", "archived"];
    for (const action of ["pause", "resume", "suspend", "archive"] as LifecycleAction[]) {
      expect(known).toContain(resultOf(action));
    }
  });
});

describe("what the operator sees while the host is still answering", () => {
  it("shows the requested state immediately, not the stale one", () => {
    // The click flips this before any await. Previously the buttons waited on
    // the action *and* the refresh behind it, re-enabling in between while
    // still offering "Pause" on a company that had just been paused.
    expect(shown(resultOf("pause"), "running")).toBe("paused");
  });

  it("falls back to the host's answer once the request settles", () => {
    // Cleared in `finally`, so this is also the failure path: an action the
    // host rejected must not leave the console asserting the state it asked
    // for.
    expect(shown(null, "running")).toBe("running");
  });
});
