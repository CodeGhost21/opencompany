// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type {
  WorkflowFixFromRun,
  WorkflowGraph,
  WorkflowRunOutcome,
  WorkflowRunsPage,
} from "@/api/workflows";

/**
 * Issue #1704: what a workflow or company switch has to leave behind.
 *
 * `WorkflowsView` has one cleanup effect on `[selectedId, company]`, and every
 * fix in its history has been the same shape — a piece of per-workflow state
 * that was not listed there, rendering under the NEXT workflow as though it
 * belonged to it (`result` and `runRefusal` in #528, `runFailure` in #1007,
 * `adoptedFromHistoryRef` in #863, `liveRanRef` in #1010, the run-input draft
 * in #1204). These are the four still missing, and two of them are worse than
 * merely stale:
 *
 *  * **`fixingRunSeq` / `fixReason` are keyed by run `seq`.** `seq` is a
 *    journal position allocated per COMPANY, not per workflow, so a leftover
 *    value does not go unread — it lands on whichever run of the newly selected
 *    workflow happens to share that number. And because `RunHistoryPanel`
 *    disables EVERY row's Fix button while `fixingRunSeq` is set (one fix at a
 *    time), a leaked one takes the affordance away from a workflow no fix was
 *    ever requested for.
 *
 *  * **Clearing them is only half the fix.** The switch happens while the fix
 *    request is still in flight, so the clear runs first and the reply writes
 *    the state straight back. The second test below is the one that pins that:
 *    it fails against a cleanup effect that clears both fields and nothing else.
 *
 *  * **`conflict`** is the last of the persistent banners to outlive a switch.
 *    A successful graph read clears it — but a graph read that FAILS does not,
 *    which is exactly when the operator is left staring at it.
 *
 *  * **`error`** renders outside the `detailOpen` gate, so a graph-load failure
 *    follows the operator all the way back to the index.
 *
 * These render the view, the way `workflow-run-failure.test.ts` and
 * `workflow-history-cross-company-race.test.ts` earn their exception to the
 * pure-function rule: the claim is about what is in the DOM after a switch,
 * which no pure helper can pin.
 */

vi.mock("sonner", () => {
  const noop = vi.fn();
  const toast = Object.assign(noop, {
    success: noop,
    error: noop,
    warning: noop,
    info: noop,
    message: noop,
  });
  return { toast };
});

vi.mock("next-themes", () => ({ useTheme: () => ({ resolvedTheme: "light" }) }));

// React Flow measures its container on mount; jsdom has no layout and no
// `ResizeObserver`, so these stubs are what let the view render at all. None is
// under test. (Same three as `workflow-history-cross-company-race.test.ts`.)
class NoopResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
Object.assign(globalThis, {
  ResizeObserver: NoopResizeObserver,
  DOMMatrixReadOnly: class {
    m22 = 1;
  },
});
Object.defineProperties(globalThis.HTMLElement.prototype, {
  offsetHeight: { get: () => 400 },
  offsetWidth: { get: () => 800 },
});

const { WorkflowsView } = await import("@/views/WorkflowsView");

const WF_A = "wf-a";
const WF_B = "wf-b";

/** The seq both workflows' failed runs share — the collision the bug needs. */
const SHARED_SEQ = 20;

function graph(id: string, name: string): WorkflowGraph {
  return {
    id,
    name,
    version: "v1",
    nodes: [{ id: "start", kind: "trigger", name: "Start" }],
    edges: [],
  };
}

const GRAPHS: Record<string, WorkflowGraph> = {
  [WF_A]: graph(WF_A, "Workflow A"),
  [WF_B]: graph(WF_B, "Workflow B"),
};

/** A failed run — `error` is what puts the Fix affordance on the row. */
function failedRun(workflowId: string, seq: number): WorkflowRunOutcome {
  return {
    seq,
    atMillis: seq * 1_000,
    workflowId,
    scheduled: false,
    runId: `${workflowId}-r${seq}`,
    deliveries: [],
    pendingApprovals: [],
    error: `${workflowId} blew up`,
  };
}

const EMPTY: WorkflowRunsPage = { runs: [], hasMore: false };

/** A resolver the test controls, so a fetch can be held open across renders. */
function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function makeClient(script: {
  /** Held-open answer to `POST …/fix-from-run`, if the test drives one. */
  fix?: Promise<WorkflowFixFromRun>;
  /** Workflow ids whose graph read rejects, so nothing can clear state for us. */
  graphFails?: string[];
  /** Rejection for `DELETE …/workflows/{id}`, if the test drives one. */
  del?: () => Promise<never>;
}): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async (path: string) => {
      if (path.endsWith("/workflows")) {
        return [
          { id: WF_A, name: GRAPHS[WF_A].name },
          { id: WF_B, name: GRAPHS[WF_B].name },
        ];
      }
      if (path.includes("/workflows/tool-slugs")) return { slugs: [], unwired: [] };
      if (path.includes("/workflows/wired-channels")) return { channels: [] };
      if (path.includes("/workflows/runs")) {
        const url = new URL(path, "http://test");
        const workflow = url.searchParams.get("workflow");
        // The company-wide index fetch is inert here — the view stays on a
        // detail page throughout.
        if (workflow !== WF_A && workflow !== WF_B) return EMPTY;
        return { runs: [failedRun(workflow, SHARED_SEQ)], hasMore: false };
      }
      const detail = /\/workflows\/([^/?]+)$/.exec(path);
      if (detail) {
        const id = detail[1];
        if (script.graphFails?.includes(id)) {
          throw new Error(`could not load ${id}`);
        }
        return GRAPHS[id] ?? null;
      }
      return null;
    },
    post: async (path: string) => {
      if (path.includes("/fix-from-run") && script.fix) return script.fix;
      return {};
    },
    del: async () => {
      if (script.del) return script.del();
      return undefined;
    },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

/** The dialogs are portaled out of the view's own subtree. */
function inDocument<T extends Element>(testId: string): T | null {
  return document.querySelector<T>(`[data-testid="${testId}"]`);
}

function inView<T extends Element>(testId: string): T | null {
  return container.querySelector<T>(`[data-testid="${testId}"]`);
}

async function show(client: OpenCompanyClient, company: string, sub: string | null) {
  await act(async () => {
    root.render(createElement(WorkflowsView, { client, company, sub }));
  });
}

async function click(el: Element | null) {
  if (!el) throw new Error("nothing to click");
  await act(async () => {
    (el as HTMLButtonElement).click();
  });
}

async function openHistory() {
  await click(inView("workflow-history-toggle"));
}

function fixButton(): HTMLButtonElement | null {
  return inView<HTMLButtonElement>("workflow-run-fix-with-copilot");
}

describe("WorkflowsView leaves per-workflow state behind on a switch", () => {
  it("clears an in-flight copilot fix when the workflow changes", async () => {
    const fix = deferred<WorkflowFixFromRun>();
    const client = makeClient({ fix: fix.promise });

    await show(client, "acme", WF_A);
    await openHistory();
    await click(fixButton());

    // Workflow A's row is now the one being fixed.
    expect(fixButton()?.textContent).toContain("Fixing…");
    expect(fixButton()?.disabled).toBe(true);

    // The operator switches to workflow B, whose own failed run happens to
    // carry the same journal `seq`.
    await show(client, "acme", WF_B);

    // Pre-fix: `fixingRunSeq` is still 20, so B's unrelated run renders as
    // mid-fix and its Fix button is disabled by a request nobody made for it.
    expect(fixButton()?.textContent).toContain("Fix with copilot");
    expect(fixButton()?.disabled).toBe(false);

    // Let the abandoned request land so the test does not leave it dangling.
    await act(async () => {
      fix.resolve({ automatable: false, reason: "the trigger is misconfigured." });
    });
  });

  it("clears an in-flight copilot fix when the company changes", async () => {
    const fix = deferred<WorkflowFixFromRun>();
    const client = makeClient({ fix: fix.promise });

    await show(client, "acme", WF_A);
    await openHistory();
    await click(fixButton());
    expect(fixButton()?.disabled).toBe(true);

    // Same workflow id in another company — ids are unique only within a
    // company, so an identically seeded workflow is the ordinary case.
    await show(client, "beta", WF_A);

    expect(fixButton()?.textContent).toContain("Fix with copilot");
    expect(fixButton()?.disabled).toBe(false);

    await act(async () => {
      fix.resolve({ automatable: false, reason: "the trigger is misconfigured." });
    });
  });

  it("never lands the verdict for the workflow left behind on the new one", async () => {
    const fix = deferred<WorkflowFixFromRun>();
    const client = makeClient({ fix: fix.promise });

    await show(client, "acme", WF_A);
    await openHistory();
    await click(fixButton());

    // Switch away FIRST — the reply is still in flight, which is the whole
    // point: clearing `fixReason` on the switch cannot help if the request
    // that arrives afterwards writes it straight back.
    await show(client, "acme", WF_B);

    await act(async () => {
      fix.resolve({ automatable: false, reason: "the trigger is misconfigured." });
    });

    // Pre-fix: "The copilot couldn't fix this…" appears under workflow B's
    // run, about a failure of workflow A's.
    expect(inView("workflow-run-fix-not-automatable")).toBeNull();
    expect(container.textContent).not.toContain("the trigger is misconfigured.");
  });

  it("does not carry a version-conflict banner onto the next workflow", async () => {
    const conflict = new ApiError(409, "conflict", "This workflow changed since you loaded it.");
    // Workflow B's graph read fails, so nothing incidentally clears the banner:
    // only a successful read does, and that is precisely the case where an
    // operator would never see the leak.
    const client = makeClient({
      graphFails: [WF_B],
      del: () => Promise.reject(conflict),
    });

    await show(client, "acme", WF_A);
    await click(inView("workflow-delete"));
    await click(inDocument("workflow-delete-confirm"));
    expect(inView("workflow-conflict")).not.toBeNull();

    await show(client, "acme", WF_B);

    // Pre-fix: a banner claiming B's graph is stale, offering a Reload that
    // re-reads B — a false statement with a remedy for something else.
    expect(inView("workflow-conflict")).toBeNull();
  });

  it("does not carry a graph-load error back to the index", async () => {
    const client = makeClient({ graphFails: [WF_A] });

    await show(client, "acme", WF_A);
    expect(inView("workflow-load-error")).not.toBeNull();

    await click(inView("workflow-back-to-index"));

    // Pre-fix: "could not load the workflow graph" sits over an index that
    // loaded perfectly, about a workflow nobody is looking at.
    expect(inView("workflow-load-error")).toBeNull();
  });
});
