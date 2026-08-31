// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TaskApproval, TaskDetail } from "@/api/tasks";
import type { ApprovalSummary } from "@/api/types";

/**
 * Three states the whole Task Detail screen could reach and could not get out
 * of, or got wrong — pinned at the screen rather than at a helper, because in
 * every one of them the helper was already right.
 *
 * - **A failed read was a dead end.** A non-404 leaves `detail` null and sets
 *   `error`, and the render ternary fell through to `null`: a back bar, a red
 *   alert, and nothing else. The 4s poll does recover on its own — the in-file
 *   comment claiming "nothing retries" was wrong about that — but a pane with
 *   no control in it cannot say so, and an operator who cannot tell a slow
 *   failure from a permanent one leaves.
 * - **A `?tab=plan` on a card with no plan** would select a tab with no trigger
 *   and no panel, now that Plan is addressable at all.
 * - **Retry beside a pending approval.** The run parked, so it is gone from
 *   `…/tasks/inflight`, so the bar renders the branch that offers Retry —
 *   enabled, above a row saying the card is waiting on the operator.
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

const { TaskDetailView } = await import("@/views/TaskDetailView");
const { RESUME_BLOCKED_REASON } = await import("@/lib/task-approvals");

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

const PENDING: TaskApproval = {
  id: "ap-1",
  kind: "web_fetch",
  status: "pending",
  atMillis: T0,
} as TaskApproval;

const QUEUED: ApprovalSummary = {
  id: "ap-1",
  kind: "web_fetch",
  amount_usd: null,
  at_millis: T0,
  agent: "finance",
  task: { link: "task", id: "task-1" },
  payload: { url: "https://example.com/rates" },
} as ApprovalSummary;

/**
 * A client whose `…/tasks/{id}` read is scripted call by call.
 *
 * Everything else answers something harmless: the screen also reads the ledger
 * list (for column labels) and the in-flight list, and neither is what any of
 * these tests is about.
 */
function client(reads: Array<() => Promise<unknown>>): OpenCompanyClient {
  let n = 0;
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    // The blocked row names whoever asked, which is a roster read.
    listTeam: async () => [],
    get: async (path: string) => {
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
}

let container: HTMLDivElement;
let root: Root;

async function render(
  c: OpenCompanyClient,
  over: {
    focus?: { tab?: "timeline" | "attempts" | "plan" | "artifacts" | "discussion" };
    parked?: ApprovalSummary[];
  } = {},
) {
  await act(async () => {
    root.render(
      createElement(TaskDetailView, {
        client: c,
        company: "acme",
        taskId: "task-1",
        focus: over.focus,
        parked: over.parked,
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
  vi.clearAllMocks();
});

describe("a read that failed with something other than a 404", () => {
  it("offers a way out of the pane rather than leaving it empty", async () => {
    await render(client([async () => Promise.reject(new Error("gateway is down"))]));
    expect(container.textContent).toContain("gateway is down");
    expect(button("Try again")).toBeDefined();
  });

  it("recovers the card when the retry succeeds", async () => {
    const reads: Array<() => Promise<unknown>> = [
      async () => Promise.reject(new Error("gateway is down")),
      async () => detail(),
    ];
    await render(client(reads));
    expect(container.textContent).not.toContain("Reconcile the ledger");
    await act(async () => {
      button("Try again")!.click();
    });
    expect(container.textContent).toContain("Reconcile the ledger");
    expect(button("Try again")).toBeUndefined();
  });
});

describe("an address naming the Plan tab", () => {
  it("falls back to Timeline on a card that has no plan", async () => {
    await render(client([async () => detail()]), { focus: { tab: "plan" } });
    // Not a blank panel under a tab bar: the default tab's own content.
    expect(container.textContent).toContain("Nothing has happened yet");
  });
});

describe("the screen's heading outline", () => {
  it("puts an h2 between the card's h1 and the h3s its panels render", async () => {
    await render(client([async () => detail()]));
    expect(container.querySelector("h1")?.textContent).toBe("Reconcile the ledger");
    // The Artifacts panel renders an `h3` per artifact, so with only the card
    // title above it the outline jumped h1 → h3 and a reader navigating by
    // heading could not tell which section they had landed in. Each panel now
    // opens with its own `h2`, `sr-only` because the tab bar is already the
    // visible label.
    const h2s = [...container.querySelectorAll("h2")].map((h) => h.textContent);
    expect(h2s).toContain("Timeline");
  });
});

describe("a card parked on an approval, with no run in flight", () => {
  it("disables Retry and says why", async () => {
    await render(client([async () => detail({ approvals: [PENDING] })]), {
      parked: [QUEUED],
    });
    // The row that explains it, and the button that must not fire beneath it.
    expect(container.textContent).toContain("Waiting on your approval");
    const retry = button("Retry");
    expect(retry).toBeDefined();
    expect(retry!.disabled).toBe(true);
    expect(retry!.title).toBe(RESUME_BLOCKED_REASON);
  });

  it("disables it on the host's word alone, before the queue has delivered the row", async () => {
    // The two reads land separately. `approvals` is the host's own ownership
    // answer and arrives first; a Retry that stayed live for that poll is the
    // whole bug, just four seconds narrower.
    await render(client([async () => detail({ approvals: [PENDING] })]), {
      parked: [],
    });
    expect(button("Retry")!.disabled).toBe(true);
  });

  /**
   * The other direction, and it is **deliberate** (#1953 review).
   *
   * A queue row this card owns, with the card's own `approvals` read not yet
   * carrying it, still takes Retry down. Intersecting the queue with
   * `detail.approvals` here — the rule `AwaitingApprovalRow` applies to its
   * *rows* — would collapse this union into `awaitingApproval` alone and hand
   * the operator a live Retry beside a genuinely parked approval.
   *
   * The two are not the same claim. Over-claiming a **row** offers a resolve
   * button for a request this card may not own; over-claiming the **block**
   * only greys a button for a poll. The host ranks them the same way in
   * `pending_approvals_resolved`: when it cannot resolve an approval's owning
   * attempt it keeps the parked link rather than dropping it, because "a
   * dropped blocker is a card that lies about being free".
   *
   * The stale-attribution worry that motivates intersecting was fixed at the
   * source by #1891: `ApprovalSummary.task` is the host's `approval_owner`
   * answer, not the park's stamp, so a queue row reaching here already belongs
   * to this card.
   */
  it("disables it on the queue's word alone, before the card's own read carries it", async () => {
    await render(client([async () => detail({ approvals: [] })]), {
      parked: [QUEUED],
    });
    expect(button("Retry")!.disabled).toBe(true);
    expect(button("Retry")!.title).toBe(RESUME_BLOCKED_REASON);
  });

  it("leaves it live on a card with nothing parked", async () => {
    await render(client([async () => detail()]));
    expect(button("Retry")!.disabled).toBe(false);
  });
});
