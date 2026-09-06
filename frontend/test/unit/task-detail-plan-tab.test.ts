import { describe, expect, it } from "vitest";

import type { Prerequisite, TaskPlan } from "@/api/tasks";
import { isTaskTab, readTaskFocus, TASK_TABS, taskTabHref } from "@/lib/task-output";
import { planTabCount } from "@/views/TaskDetailView";

/**
 * The Plan tab: addressable, and legible without colour.
 *
 * Two separate failures on one tab.
 *
 * **It was not a `TaskTab`,** so selecting it wrote nothing into the address.
 * Every other tab on this screen survives a reload, a copied link and a
 * Back/Forward; Plan silently did not, and dropped the operator onto Timeline —
 * on the one tab they were most likely to be reading closely, because it is the
 * tab that answers "why hasn't this started?".
 *
 * **Its count distinguished blocking from unresolved by colour alone.** The
 * rendered text was a bare number in `text-destructive` or
 * `text-status-blocked-text`, so "this card cannot start" and "this card will
 * pause to ask you something" were the same glyph to anyone not reading the
 * hue — and to every greyscale print of the screen.
 */

function prerequisite(status: Prerequisite["status"]): Prerequisite {
  return { kind: "credential", name: "an API key", status, note: "" };
}

function plan(...statuses: Prerequisite["status"][]): TaskPlan {
  return {
    description: "",
    steps: [],
    prerequisites: statuses.map(prerequisite),
    risks: [],
    verification: "",
    scope: "",
    plannedAtMillis: 0,
  };
}

describe("the Plan tab is addressable", () => {
  it("is a tab an address can name", () => {
    expect(TASK_TABS).toContain("plan");
    expect(isTaskTab("plan")).toBe(true);
  });

  it("survives a round trip through the hash", () => {
    // Selecting the tab writes it; reading the address back yields it. Before
    // this the write was dropped by `isTaskTab` and the read returned `{}`, so
    // a reload landed on Timeline with the URL still claiming Plan.
    const href = taskTabHref("#/tasks/t-1?host=local", "plan");
    expect(href).toBe("#/tasks/t-1?host=local&tab=plan");
    expect(readTaskFocus(href)).toEqual({ tab: "plan" });
  });
});

describe("the Plan tab's count carries a signal that is not its colour", () => {
  it("marks a blocking count with the brief's own blocked shape", () => {
    const badge = planTabCount(plan("missing", "missing", "satisfied"));
    expect(badge).not.toBeNull();
    expect(badge!.count).toBe(2);
    expect(badge!.label).toBe("2 blocking prerequisites");
    // The icon is what a reader who cannot use the colour sees; that it is a
    // *different* component from the unresolved one is the whole point.
    expect(badge!.Icon).not.toBe(planTabCount(plan("needsApproval"))!.Icon);
  });

  it("marks an unresolved count differently, and says which it is", () => {
    const badge = planTabCount(plan("needsApproval", "unknown"));
    expect(badge!.count).toBe(2);
    expect(badge!.label).toBe("2 unresolved prerequisites");
  });

  it("counts a single prerequisite in the singular", () => {
    expect(planTabCount(plan("missing"))!.label).toBe("1 blocking prerequisite");
    expect(planTabCount(plan("unknown"))!.label).toBe("1 unresolved prerequisite");
  });

  it("still reports blocking ahead of unresolved, and nothing at all when clear", () => {
    // Unchanged from #337 and re-pinned here because the shape of the return
    // grew: red is only ever `missing`, and a plan with nothing to report keeps
    // the trigger a plain word.
    expect(planTabCount(plan("missing", "needsApproval"))!.label).toBe(
      "1 blocking prerequisite",
    );
    expect(planTabCount(plan("satisfied", "satisfied"))).toBeNull();
    expect(planTabCount(plan())).toBeNull();
  });
});
