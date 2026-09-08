// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { Task } from "@/api/tasks";

/**
 * `ControlBar`'s Dispatch/Retry/Resume (`patchColumn`, `column: "working"`) —
 * `patch_task` is `ScopedCompany`, so a plain member dispatching a card is
 * correct, not a gap, and the click has to really send the `PATCH`. Also pins
 * that a finished card genuinely drops the control from the DOM rather than
 * merely disabling it — `task-detail-control-bar.test.ts` covers that shape for
 * blocked-on-approval; this is the same claim for `finished`.
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
const { ApiError } = await import("@/api/types");

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

function clientAs(patch: (path: string, body: unknown) => Promise<Task>): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async (path: string) => {
      if (path.endsWith("/auth/me")) {
        return { id: "u1", email: "a@b.c", role: "member", company: "acme" };
      }
      throw new Error("the control bar must not read on mount");
    },
    patch,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

function buttons(): HTMLButtonElement[] {
  return [...document.querySelectorAll("button")] as HTMLButtonElement[];
}

function button(label: string): HTMLButtonElement | undefined {
  return buttons().find((b) => b.textContent?.trim() === label);
}

async function render(client: OpenCompanyClient, over: Partial<Task> = {}, finished = false) {
  const onChanged = vi.fn();
  await act(async () => {
    root.render(
      createElement(ControlBar, {
        task: task(over),
        inflight: null,
        irreversible: [],
        historyIncomplete: false,
        blockedOnApproval: false,
        neverStarted: true,
        finished,
        client,
        company: "acme",
        onChanged,
        onEdit: () => {},
      }),
    );
  });
  return { onChanged };
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

describe("Dispatch, for a plain member session", () => {
  it("is offered, and clicking it really sends the PATCH", async () => {
    const patch = vi.fn(async (_path: string, _body: unknown) => task({ column: "working" }));
    const { onChanged } = await render(clientAs(patch));

    await act(async () => {
      button("Dispatch")!.click();
    });

    expect(patch).toHaveBeenCalledTimes(1);
    const [, body] = patch.mock.calls[0] as [string, { column?: string }];
    expect(body.column).toBe("working");
    expect(onChanged).toHaveBeenCalled();
    expect(toasts.success).toHaveBeenCalled();
  });

  it("drops the control entirely once the card is finished", async () => {
    const patch = vi.fn(async () => task());
    await render(clientAs(patch), { column: "done", stage: "done" }, true);
    expect(button("Dispatch")).toBeUndefined();
    expect(button("Retry")).toBeUndefined();
    expect(button("Resume")).toBeUndefined();
  });
});

describe("Dispatch, when the host refuses the write", () => {
  it("says so and stays put — never a false 'Dispatched'", async () => {
    const patch = vi.fn(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    const { onChanged } = await render(clientAs(patch));

    await act(async () => {
      button("Dispatch")!.click();
    });

    expect(patch).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith(
      expect.stringContaining("cannot reach the company host"),
    );
    expect(toasts.success).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
    // Still offered — a failed dispatch is retryable, not a dead end.
    expect(button("Dispatch")).toBeDefined();
  });
});
