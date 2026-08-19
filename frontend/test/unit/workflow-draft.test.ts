import { describe, expect, it } from "vitest";

import type { WorkflowDraftFromDescription, WorkflowGraph } from "@/api/workflows";
import { draftBanners, draftLanding } from "@/lib/workflow-draft";

/**
 * The create-time copilot's banner reducer (issue #813).
 *
 * The dialog only wires these three slots into `<Alert>`s, so the branch logic —
 * a drafted graph shows a summary and any host corrections; a one-off shows a
 * reason — is proved here rather than through a render.
 */

/** A minimal graph, enough to make a draft "automatable with a workflow". */
const GRAPH: WorkflowGraph = {
  id: "weekly-digest",
  name: "Weekly digest",
  version: null,
  nodes: [],
  edges: [],
};

function drafted(over: Partial<WorkflowDraftFromDescription>): WorkflowDraftFromDescription {
  return { automatable: true, ...over };
}

describe("draftBanners", () => {
  it("builds a summary line and passes host notes through for a drafted graph", () => {
    const banners = draftBanners(
      drafted({
        workflow: GRAPH,
        summary: "email the weekly digest",
        notes: ["Assigned the “Write” step to teammate `qa_engineer`."],
      }),
    );
    expect(banners.summary).toBe(
      "Drafted: email the weekly digest — review below, then Create.",
    );
    expect(banners.notes).toEqual([
      "Assigned the “Write” step to teammate `qa_engineer`.",
    ]);
    expect(banners.reason).toBeNull();
  });

  it("falls back to a bare summary line and no notes when the host sent none", () => {
    const banners = draftBanners(drafted({ workflow: GRAPH }));
    expect(banners.summary).toBe("Drafted — review below, then Create.");
    expect(banners.notes).toEqual([]);
    expect(banners.reason).toBeNull();
  });

  it("drops blank notes rather than rendering empty bullets", () => {
    const banners = draftBanners(
      drafted({ workflow: GRAPH, summary: "x", notes: ["  ", "kept"] }),
    );
    expect(banners.notes).toEqual(["kept"]);
  });

  it("surfaces the reason for a not-automatable answer, with a default", () => {
    const withReason = draftBanners({ automatable: false, reason: "this only runs once" });
    expect(withReason.summary).toBeNull();
    expect(withReason.notes).toEqual([]);
    expect(withReason.reason).toBe("this only runs once");

    const noReason = draftBanners({ automatable: false });
    expect(noReason.reason).toBe("This is better done once than built into a workflow.");
  });
});

/**
 * What a landed copilot draft may do to the form it came back to (issue #1052).
 *
 * The defect was a consent problem, not a missing guard: `runDraft` took its
 * `window.confirm` *before* a request that takes seconds, while the operator is
 * invited to keep typing through it. The answer therefore authorised replacing a
 * form that no longer existed by the time it arrived — and a draft abandoned by
 * cancelling could hydrate the *next* open.
 *
 * These pin the decision itself. Proving it through the dialog would need a slow
 * request, a rendered form and a stubbed `window.confirm`; the branch that
 * matters is three lines of reasoning about two epochs and a dirty flag.
 */
describe("draftLanding", () => {
  it("applies when nothing would be lost", () => {
    expect(draftLanding({ requestedEpoch: 3, currentEpoch: 3, dirtyNow: false })).toBe("apply");
  });

  /**
   * The bug's core case: the form was clean when the request went out and the
   * operator typed while it ran. Dirtiness is read at landing, so this asks.
   */
  it("asks when the operator has made the form dirty since the request", () => {
    expect(draftLanding({ requestedEpoch: 3, currentEpoch: 3, dirtyNow: true })).toBe("confirm");
  });

  /**
   * Cancel mid-draft, reopen: the reset effect bumped the epoch, so the
   * abandoned response must not fill in the fresh form.
   */
  it("drops a response whose dialog has moved on", () => {
    expect(draftLanding({ requestedEpoch: 3, currentEpoch: 4, dirtyNow: false })).toBe("drop");
  });

  /**
   * Staleness outranks dirtiness. A response for a form that is gone must not
   * raise a confirm about a form the operator is no longer looking at — a
   * question nobody can place is worse than silence.
   */
  it("drops rather than asking when the dialog moved on AND the form is dirty", () => {
    expect(draftLanding({ requestedEpoch: 3, currentEpoch: 4, dirtyNow: true })).toBe("drop");
  });

  /** Epochs only ever move forward, but the rule is inequality, not ordering. */
  it("treats any epoch change as stale, in either direction", () => {
    expect(draftLanding({ requestedEpoch: 4, currentEpoch: 3, dirtyNow: false })).toBe("drop");
  });
});
