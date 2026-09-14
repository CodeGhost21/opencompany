// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { InflightRun, IrreversibleEffect, Task } from "@/api/tasks";

/**
 * The redirect composer's Send — `steer(inflight.key, "redirect", {
 * instruction })` against the same `ScopedCompany` `steer_task` route Pause
 * and Cancel use. The composer's `Input` carries no `maxlength`, but the
 * host does cap a redirect at `MAX_REDIRECT_CHARS` (`src/company/steer.rs`)
 * and refuses a longer one outright rather than silently truncating it — so
 * the console's honest job is not enforcing a limit it does not display, it
 * is surfacing that refusal when it comes back. This drives Send both ways:
 * a live client, and the host's `422` for an over-long instruction.
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

function setValue(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Opens the composer and types an instruction into it. */
async function compose(text: string) {
  await act(async () => {
    button("Redirect")!.click();
  });
  const input = container.querySelector("input") as HTMLInputElement;
  await act(async () => {
    setValue(input, text);
  });
  return input;
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

describe("Redirect, by authority (steer_task's ScopedCompany write)", () => {
  it("offers a plain member client a live composer and Send, reading no role first", async () => {
    const { client } = fakeClient(async () => undefined);
    await render(client);

    const input = await compose("check the invoice total first");
    expect(input.value).toBe("check the invoice total first");
    expect(button("Send")!.disabled).toBe(false);
  });
});

describe("Redirect, a steer the host refuses", () => {
  it("shows the host's own over-length refusal rather than claiming it was sent", async () => {
    const { client, post } = fakeClient(async () => {
      throw new ApiError(
        422,
        "redirect_too_long",
        "redirect instruction is 2500 characters; the limit is 2000",
        true,
      );
    });
    const { onChanged } = await render(client);

    const input = await compose("a".repeat(2500));
    await act(async () => {
      button("Send")!.click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith(
      "redirect instruction is 2500 characters; the limit is 2000",
    );
    expect(toasts.success).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
    // Not silently cleared: the composer still holds what was typed, exactly
    // as the settled-run case (`task-detail-control-bar.test.ts`) keeps it.
    expect(input.value.length).toBe(2500);
  });

  it("re-enables Send rather than leaving the composer stuck busy on a network failure", async () => {
    const { client } = fakeClient(async () => {
      throw new ApiError(0, "network_error", "cannot reach the company host");
    });
    await render(client);

    await compose("check the invoice total first");
    await act(async () => {
      button("Send")!.click();
    });

    expect(toasts.error).toHaveBeenCalledWith("cannot reach the company host");
    expect(button("Send")!.disabled).toBe(false);
  });
});
