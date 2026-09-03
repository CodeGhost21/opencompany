// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { WorkflowGraph } from "@/api/workflows";

/**
 * B-121: deleting a workflow with a run in flight stops that run, and the
 * confirmation and the toast both have to say so — this is the tinysweeper
 * "untested-behavior" gap for PR #2053: the backend's `stop_runs_of_workflow`
 * was tested, but none of the three presentation changes this PR made were —
 * the confirmation dialog's wording varies on whether a run is watching, the
 * success toast varies on the same thing, and the view drops `activeRunId`
 * so the Stop control does not survive the workflow it belonged to.
 *
 * Mounting the view (rather than testing a pure helper) is required here for
 * the same reason `workflow-run-failure.test.ts` and `workflow-index-first.test.ts`
 * mount it: what is under test is what ends up on screen, not a return value.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

vi.mock("next-themes", () => ({ useTheme: () => ({ resolvedTheme: "light" }) }));

// React Flow measures its container on mount; jsdom has no layout and no
// `ResizeObserver`. None of these three is under test.
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

const GRAPH: WorkflowGraph = {
  id: "digest",
  name: "Weekly digest",
  version: "v1",
  nodes: [
    { id: "start", kind: "trigger", name: "Monday morning" },
    { id: "n_3", kind: "agent", name: "Draft the digest", agent: "writer" },
  ],
  edges: [{ from: "start", to: "n_3" }],
};

/**
 * A client whose run POST always accepts and detaches (issue #383's shape),
 * so `activeRunId` is set the simple way — directly from the response — with
 * no need to fake the run-history polling that would otherwise derive it.
 */
function fakeClient(): { client: OpenCompanyClient; deletes: string[] } {
  const deletes: string[] = [];
  const client = {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async (path: string) => {
      if (path.endsWith("/workflows")) return [{ id: GRAPH.id, name: GRAPH.name }];
      if (path.includes("/workflows/runs")) return { runs: [], hasMore: false };
      return GRAPH;
    },
    post: async (path: string) => {
      if (path.includes("/run")) return { runId: "r-live", detached: true };
      return {};
    },
    del: async (path: string) => {
      const id = path.match(/\/workflows\/([^/?]+)/)?.[1];
      if (id) deletes.push(decodeURIComponent(id));
    },
    // Issue #1845: the week-1 nudge banner polls this on mount; an empty
    // feed keeps it a no-op for every test in this file, which is not about
    // the nudge.
    notifications: async () => ({ notifications: [], unread: 0 }),
    markNotificationsRead: async () => ({ unread: 0 }),
  } as unknown as OpenCompanyClient;
  return { client, deletes };
}

let container: HTMLDivElement;
let root: Root;

function button(label: string, scope: ParentNode = container): HTMLButtonElement {
  const found = Array.from(scope.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no “${label}” button in:\n${(scope as Element).innerHTML}`);
  return found as HTMLButtonElement;
}

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

/** Mount on the workflow's own detail page, where Run/Delete live (issue #1110). */
async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(WorkflowsView, { client, company: "acme", sub: GRAPH.id }));
  });
}

/** Opens the delete confirmation. The dialog itself portals to `document.body`,
 * outside `container` — the same reach `workflow-index-first.test.ts` uses for
 * `workflow-delete-confirm`. */
function openDeleteConfirm() {
  button("Delete").click();
}

describe("deleting a workflow with a run in flight", () => {
  it("warns about the run in the confirmation, stops it, and says so in the toast", async () => {
    const { client, deletes } = fakeClient();
    await mount(client);

    // Dispatch a run and let it detach — `activeRunId` is set directly from
    // the accepted response (WorkflowsView.tsx's `isDetached` branch).
    await act(async () => {
      button("Run").click();
    });
    expect(container.querySelector('[data-testid="workflow-cancel-run"]')).not.toBeNull();

    await act(async () => {
      openDeleteConfirm();
    });
    const consequence = document.querySelector('[data-testid="workflow-delete-consequence"]');
    expect(consequence?.textContent).toContain("A run of this workflow is going right now");
    expect(consequence?.textContent).toContain("stops that run where it is");

    await act(async () => {
      button("Delete workflow", document.body).click();
    });

    expect(deletes).toEqual(["digest"]);
    // The success toast names the extra consequence the plain delete does not.
    expect(toasts.success).toHaveBeenCalledWith(
      expect.stringContaining("and stopped the run in flight."),
    );
    // `setActiveRunId(null)` (B-121): the Stop control cannot survive the
    // workflow it belonged to — asserted by the DOM, since that state is not
    // otherwise observable from outside the view.
    expect(container.querySelector('[data-testid="workflow-cancel-run"]')).toBeNull();
  });
});

describe("deleting a workflow with no run in flight", () => {
  it("keeps the plain warning and the plain toast", async () => {
    const { client, deletes } = fakeClient();
    await mount(client);

    await act(async () => {
      openDeleteConfirm();
    });
    const consequence = document.querySelector('[data-testid="workflow-delete-consequence"]');
    expect(consequence?.textContent).toBe(
      "This removes the workflow and stops it running on its schedule. Past runs stay in the run history. This can't be undone.",
    );

    await act(async () => {
      button("Delete workflow", document.body).click();
    });

    expect(deletes).toEqual(["digest"]);
    expect(toasts.success).toHaveBeenCalledWith(`Deleted “${GRAPH.name}”.`);
    // Never claims a run was stopped when there was none to stop.
    expect(toasts.success).not.toHaveBeenCalledWith(expect.stringContaining("stopped the run"));
  });
});
