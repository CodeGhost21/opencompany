// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { IrreversibleEffect, TaskDetail } from "@/api/tasks";
import { dispatchNeedsConfirm } from "@/lib/task-edit";

/**
 * The edit dialog's dispatch guard, wired from the detail screen's own read.
 *
 * `TaskEditDialog`'s Column select emits `{column: "working"}` — the identical
 * write the Retry button on this screen makes — so it sits behind the same
 * #351 confirmation. That gate treats an unread journal as **"cannot say"** and
 * confirms unconditionally, on purpose: a caller that has not read the journal
 * cannot claim a card is clean, and defaulting to clean would leave the guard
 * dead in a way indistinguishable from working.
 *
 * Which makes this call site's wiring the whole behaviour. Unwired, every
 * column change on every card asks the operator to confirm something that never
 * happened; wired, a card whose journal records nothing irreversible changes
 * column in one click and a card that already sent a payment still stops.
 *
 * The dialog is stubbed rather than driven, because what is being pinned is
 * what this screen *hands* it — the props are read straight into
 * `dispatchNeedsConfirm`, so its answer on those props is the observable
 * consequence, and driving a `<Select>` to reach the same assertion would be
 * testing Base UI.
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

const seen = vi.hoisted(() => ({ props: null as Record<string, unknown> | null }));

vi.mock("@/views/TaskEditDialog", () => ({
  TaskEditDialog: (props: Record<string, unknown>) => {
    seen.props = props;
    return null;
  },
}));

const { TaskDetailView } = await import("@/views/TaskDetailView");

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

const PAYMENT: IrreversibleEffect = {
  kind: "payment",
  atMillis: T0,
  amountUsd: 42,
} as IrreversibleEffect;

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

function client(d: TaskDetail): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    listTeam: async () => [],
    get: async (path: string) => {
      if (path.endsWith("/tasks/inflight")) return [];
      if (path.includes("/tasks/task-1")) return d;
      if (path.includes("/ledgers")) return { ledgers: [] };
      return {};
    },
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function render(d: TaskDetail) {
  await act(async () => {
    root.render(
      createElement(TaskDetailView, {
        client: client(d),
        company: "acme",
        taskId: "task-1",
        onBack: () => {},
        onNavigate: () => {},
        onDeleted: () => {},
      }),
    );
  });
}

/** What the gate would decide from the props this screen handed the dialog. */
function confirmsOnDispatch(): boolean {
  return dispatchNeedsConfirm(
    { column: "working" },
    {
      irreversible: seen.props!.irreversible as IrreversibleEffect[] | undefined,
      historyIncomplete: seen.props!.historyIncomplete as boolean | undefined,
    },
  );
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  seen.props = null;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("the detail screen wires the edit dialog's dispatch guard", () => {
  it("stops asking about a card whose journal records nothing irreversible", async () => {
    await render(detail());
    expect(seen.props).not.toBeNull();
    expect(confirmsOnDispatch()).toBe(false);
  });

  it("still stops on a card that already did something that cannot be undone", async () => {
    await render(detail({ irreversibleEffects: [PAYMENT] }));
    expect(confirmsOnDispatch()).toBe(true);
  });

  it("still stops when the journal admits it cannot describe its own history", async () => {
    await render(detail({ historyIncomplete: true }));
    expect(confirmsOnDispatch()).toBe(true);
  });

  it("hands over the in-flight read, which is what withholds Delete", async () => {
    // Delete is refused with a `409` while a run holds the card (#984), so the
    // dialog must not offer it — and it can only know from this screen.
    await render(detail());
    expect(seen.props!).toHaveProperty("inflight");
  });
});
