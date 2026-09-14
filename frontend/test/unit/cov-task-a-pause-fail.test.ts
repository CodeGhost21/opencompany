// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { InflightRun, IrreversibleEffect, Task } from "@/api/tasks";

/**
 * Pause — the control bar's "Stop" button, `steer(inflight.key, "pause")`
 * against `POST …/tasks/{id}/steer` (`steer_task`, `src/server/ops/tasks.rs`).
 * That route is `ScopedCompany` like the rest of the task family, so pausing
 * a run is not, and must not become, admin-gated — a member's Pause really
 * does steer the run, same as `pause`/`resume` on the company itself
 * (`settings-general-admin-only.test.ts` documents the same shape for those).
 *
 * No e2e ever steered a real pause; this drives the click through `steer()`
 * both ways — a live client, and a client the host refuses.
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

const { ControlBar } = await import("@/views/TaskDetailView");

function task(over: Partial<Task> = {}): Task {
  return {
    id: "task-1",
    title: "Reconcile the ledger",
    column: "working",
    stage: "in_review",
    priority: "medium",
    assignee: "finance",
    ...over,
  } as Task;
}

const RUNNING: InflightRun = {
  key: "run-key-1",
  taskId: "task-1",
} as InflightRun;

/** A member-shaped client that reads nothing and asks no role question. */
function fakeClient(postImpl: (path: string, body: unknown) => Promise<void>) {
  const post = vi.fn(postImpl);
  const client = {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async () => {
      throw new Error("the control bar must not read on mount");
    },
    post,
  } as unknown as OpenCompanyClient;
  return { client, post };
}

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient, onChanged = vi.fn()) {
  await act(async () => {
    root.render(
      createElement(ControlBar, {
        task: task(),
        inflight: RUNNING,
        irreversible: [] as IrreversibleEffect[],
        historyIncomplete: false,
        blockedOnApproval: false,
        neverStarted: false,
        finished: false,
        client,
        company: "acme",
        onChanged,
        onEdit: () => {},
      }),
    );
  });
  return { onChanged };
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
  vi.clearAllMocks();
});

describe("Pause (Stop), by authority (steer_task's ScopedCompany write)", () => {
  it("offers a plain member client a live Stop on a running task, reading no role first", async () => {
    const { client } = fakeClient(async () => undefined);
    await render(client);

    const stop = button("Stop");
    expect(stop).toBeDefined();
    expect(stop!.disabled).toBe(false);
  });

  it("actually steers the run: pauses it and refreshes the screen", async () => {
    const { client, post } = fakeClient(async () => undefined);
    const { onChanged } = await render(client);

    await act(async () => {
      button("Stop")!.click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(post.mock.calls[0][0]).toContain("/tasks/run-key-1/steer");
    expect(post.mock.calls[0][1]).toEqual({ action: "pause" });
    expect(onChanged).toHaveBeenCalledTimes(1);
  });
});

describe("Pause, a steer the host refuses", () => {
  it("reports the host's own message rather than claiming the run stopped", async () => {
    const { client, post } = fakeClient(async () => {
      throw new ApiError(409, "conflict", "this run already settled", true);
    });
    const { onChanged } = await render(client);

    await act(async () => {
      button("Stop")!.click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith("this run already settled");
    expect(toasts.success).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("re-enables Stop rather than leaving it stuck busy on a network failure", async () => {
    const { client } = fakeClient(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    await render(client);

    await act(async () => {
      button("Stop")!.click();
    });

    expect(toasts.error).toHaveBeenCalledWith("cannot reach the company host");
    expect(button("Stop")!.disabled).toBe(false);
  });
});
