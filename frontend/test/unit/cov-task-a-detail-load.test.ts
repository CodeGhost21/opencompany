// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { TaskDetail } from "@/api/tasks";

/**
 * The Task Detail screen's own read (`GET …/tasks/{id}`, `task_detail`), which
 * — like every other task route — sits behind `ScopedCompany`: any signed-in
 * company member, not an admin-only guard. There is no role check anywhere in
 * `load()` to get wrong, so what this file pins is that absence staying true
 * (a member sees exactly what an admin would), and that a load failure is an
 * honest, recoverable state rather than a blank pane or a permanent spinner.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function detail(over: Partial<TaskDetail> = {}): TaskDetail {
  return {
    task: {
      id: "task-1",
      title: "Reconcile the ledger",
      column: "working",
      stage: "in_review",
      priority: "medium",
      assignee: "finance",
      updatedAt: T0,
    },
    timeline: [],
    durations: {
      workedMillis: 60_000,
      workedLive: false,
      waitingMillis: 0,
      waitingLive: false,
      asOfMillis: T0,
    },
    approvals: [],
    irreversibleEffects: [],
    historyIncomplete: false,
    discussion: [],
    discussionHasMore: false,
    lineage: { children: [] },
    runs: [],
    ...over,
  } as TaskDetail;
}

/**
 * A member-shaped client: no `/auth/me` handler at all. If `load()` ever asked
 * this screen's own role before rendering the card, this client has no answer
 * for it and the read would reject — which is exactly the failure a stray
 * `useCanManage` call would produce here.
 */
function client(reads: Array<() => Promise<unknown>>): {
  client: OpenCompanyClient;
  paths: string[];
} {
  let n = 0;
  const paths: string[] = [];
  const c = {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    listTeam: async () => [],
    get: async (path: string) => {
      paths.push(path);
      if (path.endsWith("/tasks/inflight")) return [];
      if (path.includes("/tasks/task-1")) {
        const read = reads[Math.min(n, reads.length - 1)];
        n += 1;
        return read();
      }
      if (path.includes("/ledgers")) return { ledgers: [] };
      return {};
    },
  } as unknown as OpenCompanyClient;
  return { client: c, paths };
}

let container: HTMLDivElement;
let root: Root;

const { TaskDetailView } = await import("@/views/TaskDetailView");

async function render(c: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(TaskDetailView, {
        client: c,
        company: "acme",
        taskId: "task-1",
        onBack: () => {},
        onNavigate: () => {},
        onDeleted: () => {},
      }),
    );
  });
}

function button(label: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === label,
  ) as HTMLButtonElement | undefined;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe("Task Detail load, by authority", () => {
  it("loads and renders the card for a plain member client, asking no role question first", async () => {
    const { client: c, paths } = client([async () => detail()]);
    await render(c);

    expect(container.textContent).toContain("Reconcile the ledger");
    expect(paths.some((p) => p.includes("/auth/me"))).toBe(false);
  });
});

describe("Task Detail load, a rejected read", () => {
  it("shows the host's own message on a network failure, not a blank pane", async () => {
    const { client: c } = client([
      async () =>
        Promise.reject(new ApiError(0, "network_error", "cannot reach the company host")),
    ]);
    await render(c);

    expect(container.textContent).toContain("cannot reach the company host");
    expect(button("Try again")).toBeDefined();
  });

  it("shows the host's own message on a host-side rejection, not a blank pane", async () => {
    const { client: c } = client([
      async () =>
        Promise.reject(
          new ApiError(503, "unavailable", "the task store is temporarily unavailable", true),
        ),
    ]);
    await render(c);

    expect(container.textContent).toContain("the task store is temporarily unavailable");
    expect(button("Try again")).toBeDefined();
  });

  it("never leaves the card unnamed and spinning forever — the retry recovers it", async () => {
    const reads: Array<() => Promise<unknown>> = [
      async () => Promise.reject(new ApiError(0, "network_error", "cannot reach the company host")),
      async () => detail(),
    ];
    const { client: c } = client(reads);
    await render(c);

    expect(container.textContent).not.toContain("Reconcile the ledger");
    expect(container.querySelector(".animate-spin")).toBeNull();

    await act(async () => {
      button("Try again")!.click();
    });

    expect(container.textContent).toContain("Reconcile the ledger");
    expect(button("Try again")).toBeUndefined();
  });
});
