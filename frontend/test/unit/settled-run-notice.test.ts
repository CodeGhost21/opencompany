import { describe, expect, it } from "vitest";

import type { WorkflowRunVerdict } from "@/api/workflows";
import { VERDICT_TONE, settledRunNotice } from "@/views/workflows/run-health";

/**
 * B-039: what the console says the moment a run an operator started settles.
 *
 * It used to say one thing — `toast.success("Workflow ran.")` — for every run
 * whose body came back, which is every run that did not throw. A run the
 * operator had just pressed **Stop** on got it too, so one screen reported a
 * single run three contradictory ways at once: "stopped" on the row, "Workflow
 * ran." in the toast, and an output-shape error in the drawer.
 *
 * The reading now comes from the host's `verdict` (issue #981), the same word
 * every other reader of a run's outcome already mirrors.
 */
describe("settledRunNotice", () => {
  it("never tells an operator a run they stopped ran", () => {
    const notice = settledRunNotice("stopped");
    expect(notice.message).toBe("Run stopped.");
    // Idle, not a fault, and not a success — the reading `VERDICT_TONE` gives
    // a stop somebody asked for.
    expect(notice.tone).toBe("info");
  });

  it("never tells an operator a run that failed ran", () => {
    expect(settledRunNotice("failed")).toEqual({
      tone: "error",
      message: "The workflow run failed.",
    });
  });

  it("says a parked run is waiting on the operator rather than done", () => {
    for (const verdict of ["blocked", "awaiting-approval"] as const) {
      const notice = settledRunNotice(verdict);
      expect(notice.tone).toBe("info");
      expect(notice.message).toContain("waiting on your approval");
    }
  });

  it("keeps the plain sentence only for a clean run", () => {
    expect(settledRunNotice("ok")).toEqual({ tone: "success", message: "Workflow ran." });
  });

  it("keeps it for a host that sends no verdict at all, rather than inventing one", () => {
    // A host predating issue #981 ships no `verdict` key. Guessing a reading
    // for it is the habit this whole change removes.
    expect(settledRunNotice(undefined).message).toBe("Workflow ran.");
  });

  /**
   * The guard that keeps this honest as the verdict vocabulary grows: a word
   * the host can send and this function has never heard of would silently fall
   * to the success arm, which is precisely the false green B-039 is about.
   */
  it("greets exactly one verdict with a success tick", () => {
    const cheerful = (Object.keys(VERDICT_TONE) as WorkflowRunVerdict[]).filter(
      (verdict) => settledRunNotice(verdict).tone === "success",
    );
    expect(cheerful).toEqual(["ok"]);
  });
});
