// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { IrreversibleEffect, Task } from "@/api/tasks";

/**
 * "Plan first" — `planFirst()` on the detail screen's control bar, the
 * `PATCH { column: "planning" }` that buys one planning pass before any work
 * is dispatched. Offered only for a card still in `pending`,
 * and otherwise wired exactly like Dispatch: the same `patch_task` route,
 * the same `ScopedCompany` authority (any company member), and the same
 * try/catch that had never actually been driven to a rejection.
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
    column: "pending",
    stage: "pending",
    priority: "medium",
    assignee: "finance",
    ...over,
  } as Task;
}

/** A member-shaped client that reads nothing and asks no role question. */
function fakeClient(patchImpl: () => Promise<Task>) {
  const patch = vi.fn(patchImpl);
  const client = {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async () => {
      throw new Error("the control bar must not read on mount");
    },
    patch,
  } as unknown as OpenCompanyClient;
  return { client, patch };
}

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient, onChanged = vi.fn()) {
  await act(async () => {
    root.render(
      createElement(ControlBar, {
        task: task(),
        inflight: null,
        irreversible: [] as IrreversibleEffect[],
        historyIncomplete: false,
        blockedOnApproval: false,
        neverStarted: true,
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

describe("Plan first, by authority (patch_task's ScopedCompany write)", () => {
  it("offers a plain member client a live Plan first, reading no role first", async () => {
    const { client } = fakeClient(async () => task({ column: "planning" }));
    await render(client);

    const planFirst = button("Plan first");
    expect(planFirst).toBeDefined();
    expect(planFirst!.disabled).toBe(false);
  });
});

describe("Plan first, a PATCH the host refuses", () => {
  it("reports the host's own message rather than claiming a brief is being written", async () => {
    const { client, patch } = fakeClient(async () => {
      throw new ApiError(422, "invalid_column", "planning is not open for this card", true);
    });
    const { onChanged } = await render(client);

    await act(async () => {
      button("Plan first")!.click();
    });

    expect(patch).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith("planning is not open for this card");
    expect(toasts.success).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("re-enables Plan first rather than leaving it stuck busy on a network failure", async () => {
    const { client } = fakeClient(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    await render(client);

    await act(async () => {
      button("Plan first")!.click();
    });

    expect(toasts.error).toHaveBeenCalledWith("cannot reach the company host");
    expect(button("Plan first")!.disabled).toBe(false);
  });
});
