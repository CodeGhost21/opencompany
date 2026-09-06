import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

/**
 * Two pieces of plumbing on the Task Detail screen that ran and did nothing.
 *
 * Both are absences, which is why they are pinned by reading the source rather
 * than by rendering: there is no behaviour left to observe, and the risk is
 * that a later change re-adds them because the shape looks like it is missing
 * something.
 *
 * **`onSaved`** was a required prop threaded through five components, and the
 * only caller — `TaskDetailRoute` — passed `() => {}`. It existed to reconcile
 * the task board rendered *beside* this screen, and issue #1140 deleted that
 * board: the board is the `tasks` ledger now, it re-reads from the host, and it
 * is not mounted while the detail is. `load()` is what actually refreshes this
 * screen and always was, so the second path was worse than no path — it read
 * like the one doing the work.
 *
 * **`steer()`'s `if (!inflight) return`** looked like a guard and behaved like a
 * silent failure. The redirect composer deliberately outlives the run it steers
 * (so a settling run does not destroy the typed instruction), so Send really
 * could be pressed with `inflight` null — and this swallowed it: no request, no
 * error, no toast. `steer` takes the run key now, which makes the absence of a
 * run something the call site has to answer for, and Send is disabled there.
 */
describe("the no-op plumbing is gone, and stays gone", () => {
  const view = read("views/TaskDetailView.tsx");
  const route = read("views/TaskDetailRoute.tsx");
  const proposal = read("views/TaskWorkflowProposalPanel.tsx");

  it("has no `onSaved` left on the detail screen or its children", () => {
    // Matched as code rather than as a word, for two reasons: `TaskEditDialog`
    // keeps an `onSaved` of its own (it has other callers, and the detail still
    // passes it a close-and-reload), and each of these files carries a comment
    // saying why the prop is gone — a bare substring search would hit both.
    for (const [name, src] of [
      ["TaskDetailView", view],
      ["TaskDetailRoute", route],
      ["TaskWorkflowProposalPanel", proposal],
    ] as const) {
      expect(src, `${name} declares one`).not.toContain("onSaved: (t: Task) => void;");
      expect(src, `${name} invokes one`).not.toContain("onSaved(saved)");
      expect(src, `${name} threads one`).not.toContain("onSaved={onSaved}");
    }
    // The route's `onSaved={() => {}}` — the no-op the whole prop existed for.
    expect(route).not.toContain("onSaved=");
    expect(proposal).not.toContain("onSaved=");
  });

  it("still refreshes, through the path that always did the work", () => {
    // The point is not that the calls were deleted; it is that `load()` is
    // what replaced them. A save with neither would be the real regression.
    expect(view).toContain("await onChanged();");
    expect(view).toContain("void load();");
  });

  it("does not swallow a steer with no run", () => {
    expect(view).not.toContain("if (!inflight) return;");
    // The key is a parameter, so a caller with no run cannot compile its way
    // into the silent branch.
    expect(view).toContain("async function steer(\n    key: string,");
  });
});
