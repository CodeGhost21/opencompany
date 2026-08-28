// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { NotificationDto } from "@/api/types";
import type { WorkflowGraph } from "@/api/workflows";
import { WEEK1_NUDGE_KIND } from "@/lib/week1-nudge";

/**
 * Issue #1845 (review: PR #1878, two findings on `WorkflowsView`'s week-1
 * nudge banner). Neither is reachable from `week1-nudge.test.ts`, which only
 * covers the pure `pickActiveNudge` helper, and the feature's only other
 * coverage — `test/e2e/workflow-week1-nudge.spec.ts` — is Playwright and
 * needs a live host build this suite cannot start, so its assertions stay
 * unproven here rather than double-counted as existing coverage:
 *
 *  1. A `refreshNudge` fetch already in flight when this session's own
 *     `handleCreated` fires can resolve AFTER it, carrying a row the
 *     scheduler filed before the create — `clearNudge` could not have marked
 *     it read at that moment, because it did not yet know the row's id.
 *  2. The host files this nudge off a scheduler tick with no SSE frame, so a
 *     tab left open across that tick never learns a nudge landed until the
 *     next reload or company switch.
 */

vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { success: vi.fn(), error: vi.fn(), warning: vi.fn(), info: vi.fn() }),
}));
vi.mock("next-themes", () => ({ useTheme: () => ({ resolvedTheme: "light" }) }));

// React Flow measures its container on mount; jsdom has no layout and no
// `ResizeObserver`. Neither is under test here — same setup as
// `workflow-index-first.test.ts`.
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

const STUB_GRAPH: WorkflowGraph = {
  id: "wf-1",
  name: "My first workflow",
  description: "",
  nodes: [],
  edges: [],
  editable: true,
  enabled: true,
  version: "v1",
};

// The real `WorkflowCreateDialog` drives a whole copilot chat flow; stubbed
// here to isolate `WorkflowsView`'s own nudge-clearing wiring, which is what
// both findings are about. The create-mode dialog (no `workflow` prop) gets a
// one-click "confirm" trigger; the edit-mode instance renders nothing, since
// neither finding touches it.
vi.mock("@/views/WorkflowCreateDialog", () => ({
  WorkflowCreateDialog: (props: {
    open: boolean;
    workflow?: unknown;
    onCreated?: (graph: WorkflowGraph) => void;
  }) => {
    if (!props.open || props.workflow) return null;
    return createElement(
      "button",
      {
        "data-testid": "week1-nudge-test-confirm-create",
        onClick: () => props.onCreated?.(STUB_GRAPH),
      },
      "confirm create",
    );
  },
}));

const { WorkflowsView } = await import("@/views/WorkflowsView");

function nudgeRow(id: string): NotificationDto {
  return {
    id,
    kind: WEEK1_NUDGE_KIND,
    subjectKind: "workflow",
    subjectId: "week1-first-workflow",
    title: "Save your first workflow",
    createdAt: 1,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
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
  window.location.hash = "";
});

const named = (label: string) =>
  Array.from(container.querySelectorAll("button")).find((b) => b.textContent?.trim() === label) ?? null;
const banner = () => container.querySelector('[data-testid="workflow-week1-nudge"]');

function baseClient(overrides: Partial<OpenCompanyClient>): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async (path: string) => {
      if (path.endsWith("/workflows")) return [];
      if (path.includes("/workflows/tool-slugs")) return { slugs: [], unwired: [] };
      if (path.includes("/workflows/wired-channels")) return { channels: [] };
      if (path.includes("/workflows/runs")) return { runs: [], hasMore: false };
      return null;
    },
    post: async () => ({}),
    del: async () => {},
    markNotificationsRead: async () => ({ unread: 0 }),
    ...overrides,
  } as unknown as OpenCompanyClient;
}

describe("the week-1 nudge banner (PR #1878 review)", () => {
  it("does not resurrect a nudge off a fetch that was already stale when this session created a workflow", async () => {
    const gate = deferred<{ notifications: NotificationDto[]; unread: number }>();
    const markedRead: string[] = [];
    const client = baseClient({
      notifications: () => gate.promise,
      markNotificationsRead: async (ids: string[]) => {
        markedRead.push(...ids);
        return { unread: 0 };
      },
    });

    window.location.hash = "#/workflows";
    await act(async () => {
      root.render(createElement(WorkflowsView, { client, company: "acme", sub: null }));
    });
    // The mount effect's `refreshNudge` call is now in flight against `gate`.

    // The operator creates a workflow through the ordinary "New workflow"
    // flow before that fetch resolves.
    await act(async () => {
      named("New workflow")?.click();
    });
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="week1-nudge-test-confirm-create"]')?.click();
    });

    // The stale fetch now resolves, carrying the row the scheduler filed
    // before this create.
    await act(async () => {
      gate.resolve({ notifications: [nudgeRow("n1")], unread: 1 });
      await gate.promise;
    });

    // `handleCreated` also navigates to the new workflow's detail view
    // (issue #1110), where the banner never renders regardless of this fix —
    // that alone would not distinguish pre-fix from post-fix. Returning to
    // the index is what actually surfaces whatever the stale fetch left in
    // `nudge` state, with no reload involved.
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="workflow-back-to-index"]')?.click();
    });

    expect(banner()).toBeNull();
    expect(markedRead).toContain("n1");
  });

  it("still shows a fetch that resolves stale when no local create happened", async () => {
    // Control: the same stale-response shape as above, but without a local
    // create in between — the banner must still show, so the fix above is
    // not simply suppressing the banner unconditionally.
    const gate = deferred<{ notifications: NotificationDto[]; unread: number }>();
    const client = baseClient({ notifications: () => gate.promise });

    window.location.hash = "#/workflows";
    await act(async () => {
      root.render(createElement(WorkflowsView, { client, company: "acme", sub: null }));
    });
    await act(async () => {
      gate.resolve({ notifications: [nudgeRow("n1")], unread: 1 });
      await gate.promise;
    });

    expect(banner()).not.toBeNull();
  });

  it("picks up a nudge filed after mount once the host's own poll cadence ticks", async () => {
    let feed: NotificationDto[] = [];
    const client = baseClient({
      notifications: async () => ({ notifications: feed, unread: feed.length }),
    });

    window.location.hash = "#/workflows";
    await act(async () => {
      root.render(
        createElement(WorkflowsView, { client, company: "acme", sub: null, approvalsNow: 1_000 }),
      );
    });
    expect(banner()).toBeNull();

    // The scheduler's daily tick files a nudge server-side. Nothing about
    // this tab changes yet — no SSE frame exists for it.
    feed = [nudgeRow("n1")];

    // The host's own poll cadence (the same `feed.now` tick the mention badge
    // piggybacks on in `app-shell.tsx`) advances, re-rendering with a new
    // `approvalsNow`.
    await act(async () => {
      root.render(
        createElement(WorkflowsView, { client, company: "acme", sub: null, approvalsNow: 2_000 }),
      );
    });

    expect(banner()).not.toBeNull();
  });
});
