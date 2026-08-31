// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InflightRun, IrreversibleEffect, Task } from "@/api/tasks";
import { RESUME_BLOCKED_REASON } from "@/lib/task-approvals";

/**
 * The task detail's control bar, which is where the screen decides what the
 * operator is allowed to do to a card.
 *
 * Every claim below is one the source reads correctly for and the screen got
 * wrong, which is why these render rather than grep:
 *
 * - **Retry beside a pending approval.** A run parked on a sign-off is
 *   *finished* to the runs store, so the card leaves `GET …/tasks/inflight`,
 *   `inflight` goes null, and the bar renders its else-branch — Retry, enabled,
 *   directly above a row reading "Waiting on your approval". One click spends a
 *   second agent turn on work the operator is still being asked to authorise,
 *   and re-runs it from the start. The board card has been disabling this since
 *   #883 and the detail never did.
 * - **Retry on a Done card**, which re-ran a task somebody had already
 *   accepted.
 * - **Three names for one state.** "Not yet dispatched" in the header, "Retry"
 *   on the button, "Dispatched" in the toast. "Retry" is the false one.
 * - **A composed redirect outliving its run.** The composer survives `inflight`
 *   going null — deliberately, so the typed instruction is not destroyed — and
 *   for a while Send survived with it and silently did nothing.
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

/** A client that answers nothing: this bar reads on click, never on mount. */
const CLIENT = {
  scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
  get: async () => {
    throw new Error("the control bar must not read on mount");
  },
} as unknown as OpenCompanyClient;

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

let container: HTMLDivElement;
let root: Root;

async function render(
  over: {
    task?: Task;
    inflight?: InflightRun | null;
    irreversible?: IrreversibleEffect[];
    historyIncomplete?: boolean;
    blockedOnApproval?: boolean;
    neverStarted?: boolean;
    finished?: boolean;
  } = {},
) {
  await act(async () => {
    root.render(
      createElement(ControlBar, {
        task: over.task ?? task(),
        inflight: over.inflight ?? null,
        irreversible: over.irreversible ?? [],
        historyIncomplete: over.historyIncomplete ?? false,
        blockedOnApproval: over.blockedOnApproval ?? false,
        neverStarted: over.neverStarted ?? false,
        finished: over.finished ?? false,
        client: CLIENT,
        company: "acme",
        onChanged: () => {},
        onEdit: () => {},
      }),
    );
  });
}

function buttons(): HTMLButtonElement[] {
  return [...document.querySelectorAll("button")] as HTMLButtonElement[];
}

function button(label: string): HTMLButtonElement | undefined {
  return buttons().find((b) => b.textContent?.trim() === label);
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

describe("Retry/Resume while the card is blocked on an approval (#883)", () => {
  it("renders Retry disabled, with the reason a pointer can read", async () => {
    await render({ blockedOnApproval: true });
    const retry = button("Retry");
    expect(retry).toBeDefined();
    expect(retry!.disabled).toBe(true);
    expect(retry!.title).toBe(RESUME_BLOCKED_REASON);
  });

  it("keeps it disabled on the confirmation-gated branch too", async () => {
    // A card with irreversible history renders the dialog trigger rather than
    // the plain button. Blocked is blocked on both, or the fix would hold for
    // the cheap cards and lapse on exactly the expensive ones.
    await render({
      blockedOnApproval: true,
      irreversible: [
        { kind: "payment", atMillis: 1, amountUsd: 12 } as IrreversibleEffect,
      ],
    });
    const retry = button("Retry");
    expect(retry).toBeDefined();
    expect(retry!.disabled).toBe(true);
    expect(retry!.title).toBe(RESUME_BLOCKED_REASON);
  });

  it("leaves it live, and unexplained, when nothing is blocking", async () => {
    await render({ blockedOnApproval: false });
    const retry = button("Retry");
    expect(retry).toBeDefined();
    expect(retry!.disabled).toBe(false);
    // No tooltip when there is nothing to excuse: a permanent title on a live
    // button is a reason that has stopped meaning anything.
    expect(retry!.title).toBe("");
  });

  it("disables rather than hides it", async () => {
    // Hiding would leave the card looking like it has no next action at all,
    // which is the ambiguity being fixed — the same call the board card made.
    await render({ blockedOnApproval: true });
    expect(button("Retry")).toBeDefined();
  });
});

describe("Retry on a finished card", () => {
  it("is not offered at all", async () => {
    await render({ task: task({ column: "done", stage: "done" }), finished: true });
    expect(button("Retry")).toBeUndefined();
    expect(button("Resume")).toBeUndefined();
    expect(button("Dispatch")).toBeUndefined();
  });

  it("still offers the read-only controls, so the bar is not empty", async () => {
    await render({ task: task({ column: "done", stage: "done" }), finished: true });
    expect(button("Export")).toBeDefined();
    expect(button("Edit")).toBeDefined();
  });
});

describe("One word per state (issue #465)", () => {
  it("says Dispatch for a card nothing has ever run", async () => {
    await render({
      task: task({ column: "pending", stage: "pending" }),
      neverStarted: true,
    });
    expect(button("Dispatch")).toBeDefined();
    // "Retry" claimed an attempt that does not exist, three lines under a
    // header reading "Not yet dispatched".
    expect(button("Retry")).toBeUndefined();
  });

  it("says Retry once the card has been worked", async () => {
    await render({ neverStarted: false });
    expect(button("Retry")).toBeDefined();
    expect(button("Dispatch")).toBeUndefined();
  });

  it("says Resume for a paused card, whatever else is true of it", async () => {
    await render({ task: task({ stage: "paused" }), neverStarted: true });
    expect(button("Resume")).toBeDefined();
    expect(button("Dispatch")).toBeUndefined();
  });
});

describe("A composed redirect when the run settles", () => {
  /** Opens the composer on a live run and types into it. */
  async function compose(text: string) {
    await render({ inflight: RUNNING });
    await act(async () => {
      button("Redirect")!.click();
    });
    const input = container.querySelector("input") as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    )!.set!;
    await act(async () => {
      setter.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    return input;
  }

  it("keeps the typed instruction on screen", async () => {
    await compose("check the invoice total first");
    // The run settles: the next detail poll finds it gone from `…/inflight`.
    await render({ inflight: null });
    const input = container.querySelector("input") as HTMLInputElement | null;
    expect(input).not.toBeNull();
    expect(input!.value).toBe("check the invoice total first");
  });

  it("takes Send away rather than leaving it to do nothing", async () => {
    await compose("check the invoice total first");
    await render({ inflight: null });
    const send = button("Send");
    expect(send).toBeDefined();
    expect(send!.disabled).toBe(true);
  });

  it("says what became of the run", async () => {
    await compose("check the invoice total first");
    await render({ inflight: null });
    expect(container.textContent).toContain("settled before the redirect was sent");
  });

  it("still sends while the run is live", async () => {
    const input = await compose("check the invoice total first");
    expect(input.value).toBe("check the invoice total first");
    expect(button("Send")!.disabled).toBe(false);
  });

  it("offers a way out, so the composer is not stuck open", async () => {
    await compose("check the invoice total first");
    await render({ inflight: null });
    await act(async () => {
      button("Discard")!.click();
    });
    expect(container.querySelector("input")).toBeNull();
  });
});
