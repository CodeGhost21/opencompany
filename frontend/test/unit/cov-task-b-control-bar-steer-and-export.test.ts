// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InflightRun, Task } from "@/api/tasks";

/**
 * `ControlBar`'s Cancel and Export, exercised through the same client a
 * signed-in company member carries.
 *
 * Both routes are `ScopedCompany` (`server::ops::tasks`, `task_export.rs`) —
 * any member, not admin-only — and neither `ControlBar` nor `ExportButton`
 * ever reads a role before rendering: there is no `useCanManage` in this file.
 * `task-detail-control-bar.test.ts` pins the composer and the Retry/Resume
 * labels, but its `CLIENT` defines no `post` and no `getDocument` at all, so
 * nothing there ever presses Cancel or Export through to the client — the
 * confirm gate and the button text are unit-tested, and the request a real
 * click sends is not.
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
  taskId: "task-1",
  key: "run-key-1",
  kind: "task",
  title: "Reconcile the ledger",
  agentId: "finance",
  startedAt: 1_700_000_000_000,
  pendingAction: null,
};

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient, over: Partial<Task> = {}) {
  await act(async () => {
    root.render(
      createElement(ControlBar, {
        task: task(over),
        inflight: RUNNING,
        irreversible: [],
        historyIncomplete: false,
        blockedOnApproval: false,
        neverStarted: false,
        finished: false,
        client,
        company: "acme",
        onChanged: () => {},
        onEdit: () => {},
      }),
    );
  });
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(document.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no "${label}" button in:\n${document.body.innerHTML}`);
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
  await act(async () => root.unmount());
  container.remove();
});

describe("Cancel reaches a live run through an ordinary member client", () => {
  /** A client that would fail if `ControlBar` ever asked who the viewer is. */
  function memberClient(post: (path: string, body: unknown) => Promise<unknown>): OpenCompanyClient {
    return {
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      get: async () => {
        throw new Error("ControlBar must not read a role before offering Cancel");
      },
      post: async (path: string, body: unknown) => post(path, body),
    } as unknown as OpenCompanyClient;
  }

  it("opens the confirm gate, then steers by key with confirm:true and refreshes the card", async () => {
    const seen: Array<{ path: string; body: unknown }> = [];
    let changed = 0;
    await act(async () => {
      root.render(
        createElement(ControlBar, {
          task: task(),
          inflight: RUNNING,
          irreversible: [],
          historyIncomplete: false,
          blockedOnApproval: false,
          neverStarted: false,
          finished: false,
          client: memberClient(async (path, body) => {
            seen.push({ path, body });
            return undefined;
          }),
          company: "acme",
          onChanged: async () => {
            changed += 1;
          },
          onEdit: () => {},
        }),
      );
    });

    // The trigger alone must not steer anything — that is the confirm gate
    // `task-detail-control-bar.test.ts` already pins. What was never proven is
    // what pressing the dialog's own confirm button actually sends.
    await act(async () => {
      button("Cancel").click();
    });
    expect(seen).toHaveLength(0);

    await act(async () => {
      button("Cancel run").click();
    });

    expect(seen).toHaveLength(1);
    expect(seen[0].path).toContain("/tasks/run-key-1/steer");
    expect(seen[0].body).toEqual({ action: "cancel", confirm: true });
    expect(changed).toBe(1);
  });

  it("offers Stop, Redirect and Cancel with no role read at all — matching the route's member-open authority", async () => {
    await render(memberClient(async () => undefined));
    expect(button("Stop")).toBeDefined();
    expect(button("Redirect")).toBeDefined();
    expect(button("Cancel")).toBeDefined();
  });
});

describe("a cancel the host refuses", () => {
  it("says so honestly, and leaves the control pressable again — never a silent success", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      post: async () => {
        throw new Error("the host fell over");
      },
    } as unknown as OpenCompanyClient);

    await act(async () => {
      button("Cancel").click();
    });
    await act(async () => {
      button("Cancel run").click();
    });

    expect(toasts.error).toHaveBeenCalledTimes(1);
    expect(toasts.success).not.toHaveBeenCalled();
    // `busy` cleared in `finally`: the operator can try again rather than
    // finding every steer control dead after one failed request.
    expect(button("Stop").disabled).toBe(false);
  });
});

describe("Export offers no role gate either", () => {
  it("renders live for a client that would fail on any role read", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      get: async () => {
        throw new Error("ControlBar must not read a role before offering Export");
      },
    } as unknown as OpenCompanyClient);
    const exportButton = button("Export");
    expect(exportButton.disabled).toBe(false);
  });

  it("says so honestly when the document read fails, rather than downloading nothing silently", async () => {
    await render({
      scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
      getDocument: async () => {
        throw new Error("could not reach the host");
      },
    } as unknown as OpenCompanyClient);

    await act(async () => {
      button("Export").click();
    });

    expect(toasts.error).toHaveBeenCalledTimes(1);
    expect(toasts.success).not.toHaveBeenCalled();
    expect(button("Export").disabled).toBe(false);
  });
});
