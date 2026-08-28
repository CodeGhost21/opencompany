// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { Task } from "@/api/tasks";
import type { TeamMemberDto } from "@/api/types";
import type { TaskColumn } from "@/lib/board-columns";

/**
 * The Working filter cannot strand an operator in an empty roster.
 *
 * The filter above the roster hides every teammate whose current workload is
 * not "working", and the switch is disabled while the workload has not been
 * read. That pair is fine while the workload is readable, and fine while it is
 * null from the start — the toggle cannot be turned on. The failure is the
 * transition in between: an operator turns the filter on, then a later
 * workload refresh fails (`loadWorkload` lands on `null`), and every member now
 * reads as not working while the disabled switch offers no way back. The
 * roster disappears behind a filter nothing can turn off.
 *
 * # Why this is a component test and not a pure-function test
 *
 * The reset lives in `TeamView` state — `workingOnly` must follow `workload`
 * back to `false` when the read fails. That is a claim about what the DOM does
 * across a prop change, not something a helper can pin, so this renders the
 * view the way `ledger-retire-confirm.test.ts` does for the same reason. The
 * e2e suite pins the filter's happy path and the disabled-switch guard; this
 * pins the transition the mock cannot drive (the reload needs a `refreshKey`
 * bump, which only a setup completion — a live-host flow — produces).
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

const api = vi.hoisted(() => ({
  listTasks: vi.fn(),
  fetchBoardColumns: vi.fn(),
  fetchMe: vi.fn(),
  listPeople: vi.fn(),
  setInboxEnabled: vi.fn(),
}));

vi.mock("@/api/tasks", () => ({ listTasks: api.listTasks }));
vi.mock("@/lib/board-columns", () => ({
  fetchBoardColumns: api.fetchBoardColumns,
  // `team-workload.ts` reads this beside the fetch — the "working" status it
  // derives is what the Working filter judges against.
  IN_FLIGHT_COLUMNS: ["planning", "in_progress"],
}));
vi.mock("@/api/auth", () => ({ me: api.fetchMe, listPeople: api.listPeople }));
vi.mock("@/api/inbox", () => ({ setInboxEnabled: api.setInboxEnabled }));

const { TeamView } = await import("@/views/TeamView");

/** Ravi and Priya carry no global flag in the e2e fixture; keep the same mix. */
const ROSTER: TeamMemberDto[] = [
  {
    id: "maya",
    name: "Maya",
    role: "Research Lead",
    description: "Tracks competitor moves and drafts the weekly brief.",
  },
  {
    id: "ravi",
    name: "Ravi",
    role: "Analyst",
    description: "Digs through the numbers.",
  },
  { id: "priya", name: "Priya", role: "Writer", description: "Turns findings into words." },
];

const TASKS: Task[] = [
  {
    id: "t1",
    title: "Scan competitor pricing",
    column: "working",
    stage: "in_progress",
    priority: "high",
    assignee: "maya",
    updatedAt: 0,
  },
  {
    id: "t2",
    title: "Draft the weekly brief",
    column: "pending",
    priority: "medium",
    assignee: "maya",
    updatedAt: 0,
  },
  {
    id: "t3",
    title: "Q3 cohort numbers",
    column: "done",
    priority: "medium",
    assignee: "ravi",
    updatedAt: 0,
  },
];

const COLUMNS: TaskColumn[] = [
  { id: "pending", label: "Pending", closed: false },
  { id: "working", label: "Working", closed: false },
  { id: "done", label: "Done", closed: true },
];

function fakeClient(): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    listTeam: async () => ROSTER,
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

function card(name: string): HTMLElement | null {
  return (
    Array.from(document.querySelectorAll<HTMLElement>('[data-testid="team-card"]')).find((el) =>
      el.textContent?.includes(name),
    ) ?? null
  );
}

function workingSwitch(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[data-testid="team-roster-working"]');
}

function render(client: OpenCompanyClient, refreshKey: number) {
  return act(async () => {
    root.render(
      createElement(TeamView, {
        client,
        company: "acme",
        sub: null,
        onOpenAgent: vi.fn(),
        refreshKey,
        onRunSetup: vi.fn(),
        onManageDesks: vi.fn(),
        onNavigateToDesk: vi.fn(),
      }),
    );
  });
}

describe("the Working filter survives a workload outage (issue #1436)", () => {
  it("resets the filter when a workload refresh fails, so the roster stays reachable", async () => {
    api.listTasks.mockResolvedValue(TASKS);
    api.fetchBoardColumns.mockResolvedValue(COLUMNS);
    api.fetchMe.mockResolvedValue({ role: "admin" });
    api.listPeople.mockResolvedValue([]);

    // One client for the whole scenario: only `refreshKey` changes between
    // renders, which is exactly the setup-completion reload the bug needs.
    const client = fakeClient();
    await render(client, 0);

    expect(card("Maya")).not.toBeNull();
    const first = workingSwitch();
    expect(first).not.toBeNull();
    // The workload has been read, so the filter is live and off.
    expect(first?.getAttribute("data-disabled")).toBeNull();
    expect(first?.getAttribute("data-checked")).toBeNull();

    // Turn it on: only Maya — the one with an in-flight card — remains.
    await act(async () => {
      first?.click();
    });
    expect(workingSwitch()?.getAttribute("data-checked")).not.toBeNull();
    expect(card("Maya")).not.toBeNull();
    expect(card("Ravi")).toBeNull();
    expect(card("Priya")).toBeNull();

    // The next workload read fails (a re-run setup on a dropped network).
    api.listTasks.mockRejectedValue(new Error("drop"));
    api.fetchBoardColumns.mockRejectedValue(new Error("drop"));
    await render(client, 1);

    // The filter resets rather than hiding the roster behind a switch that is
    // now disabled and can no longer be turned off.
    const after = workingSwitch();
    expect(after?.getAttribute("data-disabled")).not.toBeNull();
    expect(after?.getAttribute("data-checked")).toBeNull();
    expect(card("Maya")).not.toBeNull();
    expect(card("Ravi")).not.toBeNull();
    expect(card("Priya")).not.toBeNull();
  });

  it("clears a stale workload before a re-read, so the previous map cannot filter the refreshed roster", async () => {
    api.listTasks.mockResolvedValue(TASKS);
    api.fetchBoardColumns.mockResolvedValue(COLUMNS);
    api.fetchMe.mockResolvedValue({ role: "admin" });
    api.listPeople.mockResolvedValue([]);

    const client = fakeClient();
    await render(client, 0);

    // Turn the filter on: only Maya — the one with an in-flight card — remains.
    await act(async () => {
      workingSwitch()?.click();
    });
    expect(card("Maya")).not.toBeNull();
    expect(card("Ravi")).toBeNull();

    // The next re-read's workload requests hang (a stalled network). The
    // roster read still lands, so the refreshed roster renders — and the
    // previous read's workload map must not silently filter it.
    api.listTasks.mockReturnValue(new Promise<Task[]>(() => {}));
    api.fetchBoardColumns.mockReturnValue(new Promise<TaskColumn[]>(() => {}));
    await render(client, 1);

    // Everyone is visible, and the switch is disabled because the new workload
    // is not known yet — the stale map is gone rather than still filtering.
    expect(card("Maya")).not.toBeNull();
    expect(card("Ravi")).not.toBeNull();
    expect(card("Priya")).not.toBeNull();
    const after = workingSwitch();
    expect(after?.getAttribute("data-disabled")).not.toBeNull();
    expect(after?.getAttribute("data-checked")).toBeNull();
  });

  it("ignores a superseded workload read, so a later read's map is the one that filters", async () => {
    api.fetchMe.mockResolvedValue({ role: "admin" });
    api.listPeople.mockResolvedValue([]);
    api.fetchBoardColumns.mockResolvedValue(COLUMNS);

    // Two task reads, resolved out of order: the first starts earlier but lands
    // after the second. The stale one must not overwrite the newer map.
    const resolveTask: Array<(tasks: Task[]) => void> = [];
    api.listTasks.mockImplementation(
      () =>
        new Promise<Task[]>((resolve) => {
          resolveTask.push(resolve);
        }),
    );

    const client = fakeClient();
    await render(client, 0); // read 1 (older) starts, stays in flight
    await render(client, 1); // read 2 (newer) starts, read 1 still in flight
    expect(resolveTask).toHaveLength(2);

    // The newer read lands first: Maya working.
    await act(async () => {
      resolveTask[1](TASKS);
    });
    // The older read lands second with nobody working: it must be ignored.
    await act(async () => {
      resolveTask[0]([]);
    });

    // With the filter on, only Maya — the newer read's answer — remains. If
    // the stale read had overwritten the map, Maya would read as idle and the
    // roster would empty out behind the filter.
    await act(async () => {
      workingSwitch()?.click();
    });
    expect(card("Maya")).not.toBeNull();
    expect(card("Ravi")).toBeNull();
    expect(card("Priya")).toBeNull();
  });
});
