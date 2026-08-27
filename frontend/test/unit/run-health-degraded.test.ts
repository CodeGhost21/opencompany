import { describe, expect, it } from "vitest";

import type { WorkflowRunOutcome } from "@/api/workflows";
import { VERDICT_TONE, runTone, verdictOf } from "@/views/workflows/run-health";

/**
 * Issue #1865: the console's newest verdict — a run whose node errored under
 * `on_error: continue|route`, or whose agent turn truncated at the iteration
 * cap, kept going and settled with nothing else wrong. Before this the host
 * never sent this word, so the console's own reading of such a run reached
 * "ok" the same way `WorkflowRunVerdict::of` did before its own `degraded`
 * arm — the false-success half of the issue, mirrored client-side.
 *
 * The host is the source of truth for `degraded` (there is no client-side
 * fallback ladder for it, unlike the older verdicts — see `verdictOf`'s own
 * comment on why a pre-#981 host still needs one and a pre-#1865 host does
 * not need this one badly enough to earn a second ladder), so these tests
 * pin the READING side only: once `run.verdict === "degraded"` arrives, the
 * console must render it as its own amber tone, not fold it into `ok`.
 */

function baseRun(over: Partial<WorkflowRunOutcome> = {}): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_700_000_000_000,
    workflowId: "daily_digest",
    scheduled: false,
    deliveries: [],
    pendingApprovals: [],
    ...over,
  };
}

describe("the degraded verdict", () => {
  it("is a token VERDICT_TONE recognises, with its own amber dot", () => {
    expect(VERDICT_TONE.degraded).toBeDefined();
    expect(VERDICT_TONE.degraded.label).toBe("degraded");
    // Amber — the same shape `blocked`/`awaiting-approval` wear, not the red
    // `failed`/`undelivered` share. The run's own config asked for the
    // branch to survive the error, and it did.
    expect(VERDICT_TONE.degraded.dot).toBe(VERDICT_TONE.blocked.dot);
    expect(VERDICT_TONE.degraded.dot).not.toBe(VERDICT_TONE.failed.dot);
  });

  it("verdictOf trusts the host's word rather than folding it into ok", () => {
    const run = baseRun({ verdict: "degraded" });
    expect(verdictOf(run)).toBe("degraded");
    expect(runTone(run)).toEqual(VERDICT_TONE.degraded);
  });

  it("does not shadow a host word it does not recognise, so a genuinely ok run stays ok", () => {
    const run = baseRun({ verdict: "ok" });
    expect(verdictOf(run)).toBe("ok");
  });
});
