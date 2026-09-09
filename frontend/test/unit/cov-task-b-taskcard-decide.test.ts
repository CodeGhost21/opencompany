// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { Task } from "@/api/tasks";
import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { taskApprovalRows } from "@/lib/task-approvals";
import { TaskItem } from "@/views/TaskCard";
import type { DecidedApproval } from "@/views/chat/model";

/**
 * Deciding a blocked approval from the board card.
 *
 * `task-blocked-card.test.ts` already drives Approve/Decline clicks through to
 * a recording `onDecide`, so "a real decide from the card" in the ordinary
 * sense is covered. Two edges are not:
 *
 * - **The read-only gate.** `ApprovalRow`'s own doc: "a surface with no
 *   handler renders no decide controls rather than live buttons that do
 *   nothing. Every board in this console is handed one" — so `onDecide` being
 *   absent is the actual authority axis here, not admin-vs-member (the host
 *   asks nothing about role for a decide either). No test mounts the card
 *   without one.
 * - **A decide that failed.** `failed` is threaded through every render in
 *   `task-blocked-card.test.ts` as `{}`; nothing there ever puts an id in it
 *   and checks the card still says so, with live buttons to retry.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function card(): Task {
  return {
    id: "task-1",
    title: "Triage the release blockers",
    column: "working",
    stage: "paused",
    priority: "high",
    assignee: "qa",
    updatedAt: T0,
  } as Task;
}

function parked(id: string): ApprovalSummary {
  return {
    id,
    kind: "web_fetch",
    amount_usd: null,
    at_millis: T0,
    agent: "qa",
    task: { link: "task", id: "task-1" },
    payload: { url: "https://example.com/a" },
    batch: "turn-1",
  };
}

let container: HTMLDivElement;
let root: Root;

async function render(opts: {
  approvals?: ApprovalSummary[];
  onDecide?:
    | ((approval: ApprovalSummary, verdict: Verdict, scope: GrantScope) => void)
    | undefined;
  failed?: Record<string, string>;
  decided?: Record<string, DecidedApproval>;
  deciding?: ReadonlyMap<string, Verdict>;
}) {
  const approvals = opts.approvals ?? [];
  await act(async () => {
    root.render(
      createElement(TaskItem, {
        task: card(),
        dragging: false,
        rows: taskApprovalRows(approvals, opts.decided ?? {}, "task-1"),
        now: T0 + 60_000,
        askerNames: new Map([["qa", "QA Engineer"]]),
        deciding: opts.deciding ?? new Map<string, Verdict>(),
        failed: opts.failed ?? {},
        onDecide: opts.onDecide,
        onOpen: () => {},
        onResume: () => {},
      }),
    );
  });
}

function blockedRow(): HTMLElement | null {
  return container.querySelector<HTMLElement>('[data-approval-inline="card"]');
}

function button(label: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(label),
  ) as HTMLButtonElement | undefined;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("Decide from the card — gated on whether a decide handler was wired at all", () => {
  it("offers Approve/Decline when a handler is wired — the ordinary board", async () => {
    await render({ approvals: [parked("a1")], onDecide: () => {} });
    expect(blockedRow()).not.toBeNull();
    expect(button("Approve")).toBeDefined();
    expect(button("Decline")).toBeDefined();
  });

  it("renders no decide controls at all on a card with no handler, rather than live buttons wired to nothing", async () => {
    await render({ approvals: [parked("a1")], onDecide: undefined });
    expect(blockedRow()).toBeNull();
    expect(button("Approve")).toBeUndefined();
    expect(button("Decline")).toBeUndefined();
  });
});

describe("A decide that did not land", () => {
  it("says so on the card, and leaves live buttons to retry rather than hiding or freezing them", async () => {
    const a1 = parked("a1");
    await render({
      approvals: [a1],
      onDecide: () => {},
      failed: { a1: "network error" },
    });

    expect(blockedRow()?.textContent).toContain("Not recorded — try again");
    // The item is still pending, not silently marked decided — the operator
    // can press Approve again.
    const approve = button("Approve");
    expect(approve).toBeDefined();
    expect(approve!.disabled).toBe(false);
  });

  it("names how many of a batch failed, not just that something did", async () => {
    const a1 = parked("a1");
    const a2 = { ...parked("a2"), payload: { url: "https://example.com/b" } };
    await render({
      approvals: [a1, a2],
      onDecide: () => {},
      failed: { a1: "network error" },
    });

    expect(blockedRow()?.textContent).toContain("1 of 2 weren't recorded — try again");
  });
});
