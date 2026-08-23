// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunSummary } from "@/api/runs";
import { AgentRuns } from "@/views/team/AgentRuns";

/**
 * A teammate's run history (issue #1573).
 *
 * A unit render, earned for the reason the instructions-editor render is: the
 * claims under test are what reaches the operator's eye and no pure helper can
 * hold them. Three of them, and each is a way of being wrong that looks
 * completely normal on screen.
 *
 * 1. **The desk selector actually goes to the host.** An unrecognised query
 *    parameter is *ignored*, not refused, so a host predating `?agent=` answers
 *    with the whole company's newest attempts. Every row would be real and the
 *    page would still be a lie.
 * 2. **…and the answer is filtered anyway.** Which is what makes that host
 *    under-report rather than misattribute.
 * 3. **A failed source read does not cost the history.** The board and the
 *    workflow list are what turn a `taskId` into a title; when they fail, every
 *    attempt must still list.
 */

/** The list poll's cadence, mirrored from `AgentRuns` (it is module-private). */
const POLL_MS = 4000;

function run(over: Partial<RunSummary> = {}): RunSummary {
  return {
    id: "run-1",
    agentId: "engineer",
    attempt: 1,
    status: "succeeded",
    phase: "terminal",
    createdAtMillis: 1_700_000_000_000,
    startedAtMillis: 1_700_000_000_000,
    finishedAtMillis: 1_700_000_010_000,
    usage: { input: 100, output: 40, cachedInput: 0, costUsd: 0 },
    stepCount: 3,
    stepCountCapped: false,
    ...over,
  };
}

/**
 * A client whose `get` answers by path. Anything unrecognised rejects, which is
 * what makes "the section survives a read it did not get" a real assertion
 * rather than a stub returning `[]` on its behalf.
 */
function makeClient(routes: Record<string, unknown>) {
  const get = vi.fn(async (path: string) => {
    const key = Object.keys(routes).find((prefix) => path.startsWith(prefix));
    if (key === undefined) throw new Error(`no route for ${path}`);
    return routes[key];
  });
  return {
    get,
    scopeFor: () => "",
  } as unknown as OpenCompanyClient & { get: typeof get };
}

let container: HTMLDivElement;
let root: Root;

async function mount(
  client: OpenCompanyClient,
  props: { agentId?: string; agentName?: string } = {},
) {
  await act(async () => {
    root.render(
      createElement(AgentRuns, {
        client,
        company: null,
        agentId: props.agentId ?? "engineer",
        agentName: props.agentName ?? "Robin",
      }),
    );
  });
  await act(async () => {});
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("a teammate's run history", () => {
  it("asks the host for this desk's attempts rather than filtering a company page", async () => {
    const client = makeClient({
      "/runs": [run()],
      "/tasks": [],
      "/workflows": [],
    });
    await mount(client);

    const asked = client.get.mock.calls
      .map(([path]) => path)
      .filter((path) => path.startsWith("/runs"));
    expect(asked.length).toBeGreaterThan(0);
    // The selector, not a client-side slice of everything: a `limit` applied to
    // the whole company would make a desk that has been quiet lately read as
    // one that has never run.
    expect(asked[0]).toContain("agent=engineer");
  });

  it("drops a row the host attributed to somebody else", async () => {
    // What a host predating `?agent=` answers with. Belt and braces: the
    // section under-reports instead of showing another teammate's work under
    // this teammate's name.
    const client = makeClient({
      "/runs": [run({ id: "mine" }), run({ id: "theirs", agentId: "designer" })],
      "/tasks": [],
      "/workflows": [],
    });
    await mount(client);

    expect(container.querySelector('[data-testid="agent-run-mine"]')).not.toBeNull();
    expect(container.querySelector('[data-testid="agent-run-theirs"]')).toBeNull();
  });

  it("lists every attempt even when the board and workflow reads fail", async () => {
    // The source lists only decide what a run is *called*. Withholding the
    // record because a card title could not be looked up would be withholding
    // the very thing the section exists to show.
    const client = makeClient({ "/runs": [run({ taskId: "card-7" })] });
    await mount(client);

    const row = container.querySelector('[data-testid="agent-run-run-1"]');
    expect(row).not.toBeNull();
    // Named by its id, because that is all that is known about it.
    expect(row?.textContent).toContain("card-7");
  });

  it("says the teammate has not run rather than showing an empty list", async () => {
    const client = makeClient({ "/runs": [], "/tasks": [], "/workflows": [] });
    await mount(client);
    expect(container.textContent).toContain("Robin hasn't run yet");
  });

  it("sends the selected status filter to the host rather than filtering the page", async () => {
    // The history is fetched at a page limit; a status filter applied to the
    // truncated page would make "no attempt matches" a claim about the newest
    // 50, hiding older matches that sat past the cut. It must go to the host.
    const client = makeClient({
      "/runs": [run(), run({ id: "failed-1", status: "failed" })],
      "/tasks": [],
      "/workflows": [],
    });
    await mount(client);

    const failed = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-runs-filter-failed"]',
    );
    expect(failed).not.toBeNull();
    await act(async () => failed!.click());
    await act(async () => {});

    const asked = client.get.mock.calls
      .map(([path]) => path)
      .filter((path) => path.startsWith("/runs"));
    expect(asked[asked.length - 1]).toContain("agent=engineer");
    expect(asked[asked.length - 1]).toContain("status=failed");
    // And the empty-state claim is made against the filtered answer.
    expect(
      container.querySelector('[data-testid="agent-run-run-1"]'),
    ).toBeNull();
    expect(
      container.querySelector('[data-testid="agent-run-failed-1"]'),
    ).not.toBeNull();
  });

  it("says no attempt matches an empty filtered fetch, not that the teammate never ran", async () => {
    // The desk has history, but none of it is failed. The host answers the
    // filtered read with an empty page; the section must say the filter matched
    // nothing — not that the teammate has never run — and keep the controls up
    // so the filter can be turned off again.
    const get = vi.fn(async (path: string) => {
      if (path.includes("status=")) return [];
      if (path.startsWith("/runs")) return [run()];
      if (path.startsWith("/tasks")) return [];
      if (path.startsWith("/workflows")) return [];
      throw new Error(`no route for ${path}`);
    });
    const client = {
      get,
      scopeFor: () => "",
    } as unknown as OpenCompanyClient & { get: typeof get };
    await mount(client);

    const failed = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-runs-filter-failed"]',
    );
    expect(failed).not.toBeNull();
    await act(async () => failed!.click());
    await act(async () => {});

    expect(container.textContent).toContain(
      "No attempt in this history matches that filter.",
    );
    expect(container.textContent).not.toContain("Robin hasn't run yet");
    expect(
      container.querySelector('[data-testid="agent-runs-filter-all"]'),
    ).not.toBeNull();
  });

  it("opens one attempt full-width, and comes back", async () => {
    const client = makeClient({
      "/runs/run-1": { run: run(), steps: [] },
      "/runs": [run()],
      "/tasks": [],
      "/workflows": [],
    });
    await mount(client);

    const row = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-run-run-1"]',
    );
    expect(row).not.toBeNull();
    await act(async () => row!.click());
    await act(async () => {});

    expect(container.querySelector('[data-testid="agent-run-detail"]')).not.toBeNull();
    // The list is replaced, not covered: the trace is the content here, and a
    // drawer would put it in the narrowest column on the page.
    expect(container.querySelector('[data-testid="agent-runs"]')).toBeNull();

    const back = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-run-back"]',
    );
    await act(async () => back!.click());
    expect(container.querySelector('[data-testid="agent-runs"]')).not.toBeNull();
  });

  it("keeps a filtered older run's detail open when the poll omits it", async () => {
    vi.useFakeTimers();
    // The desk has more than a page of attempts, so a failed run the operator
    // reaches through the Failed filter can sit older than the newest 50. Once
    // it is open, the poll drops the status filter (a live run may settle out
    // of its bucket) and the unfiltered page omits the older run — the detail
    // must stay up anyway, held from the previously-known summary.
    const older = run({
      id: "old-failed-1",
      status: "failed",
      createdAtMillis: 1_500_000_000_000,
    });
    const get = vi.fn(async (path: string) => {
      if (path.startsWith("/runs/")) return { run: older, steps: [] };
      if (path.includes("status=failed")) return [older];
      if (path.startsWith("/runs")) return [run()];
      if (path.startsWith("/tasks")) return [];
      if (path.startsWith("/workflows")) return [];
      throw new Error(`no route for ${path}`);
    });
    const client = {
      get,
      scopeFor: () => "",
    } as unknown as OpenCompanyClient & { get: typeof get };

    await mount(client);

    const failed = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-runs-filter-failed"]',
    );
    await act(async () => failed!.click());
    await act(async () => {});

    const row = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-run-old-failed-1"]',
    );
    expect(row).not.toBeNull();
    await act(async () => row!.click());
    await act(async () => {});
    expect(
      container.querySelector('[data-testid="agent-run-detail"]'),
    ).not.toBeNull();

    // A poll tick. The unfiltered newest page does not contain the open run;
    // the section holds the selected summary rather than closing the panel.
    await act(async () => {
      vi.advanceTimersByTime(POLL_MS);
    });
    expect(
      container.querySelector('[data-testid="agent-run-detail"]'),
    ).not.toBeNull();
  });

  it("refreshes a held run's summary from its detail response", async () => {
    vi.useFakeTimers();
    // The open run sits older than the newest page, so the unfiltered poll
    // omits it and the section holds the previously-known summary. The detail
    // read keeps returning a fresh one — the attempt settled while the panel
    // was open — and the panel must follow the attempt, not the moment it was
    // opened, or a settled run would stay "Running" and poll forever.
    const older = run({
      id: "old-live-1",
      status: "running",
      phase: "active",
      createdAtMillis: 1_500_000_000_000,
    });
    const settled = {
      ...older,
      status: "succeeded" as const,
      phase: "terminal" as const,
      finishedAtMillis: 1_700_000_000_000,
    };
    const get = vi.fn(async (path: string) => {
      if (path.startsWith("/runs/")) return { run: settled, steps: [] };
      if (path.includes("status=")) return [older];
      if (path.startsWith("/runs")) return [run()];
      if (path.startsWith("/tasks")) return [];
      if (path.startsWith("/workflows")) return [];
      throw new Error(`no route for ${path}`);
    });
    const client = {
      get,
      scopeFor: () => "",
    } as unknown as OpenCompanyClient & { get: typeof get };

    await mount(client);

    const live = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-runs-filter-live"]',
    );
    expect(live).not.toBeNull();
    await act(async () => live!.click());
    await act(async () => {});

    const row = container.querySelector<HTMLButtonElement>(
      '[data-testid="agent-run-old-live-1"]',
    );
    expect(row).not.toBeNull();
    await act(async () => row!.click());
    await act(async () => {});

    // The detail read resolved with the settled summary; the held copy must
    // give way so the panel reads the attempt's current state. The old status
    // must be gone too — a panel that still claimed "Running" would be the
    // bug this regression exists to catch.
    const panel = container.querySelector('[data-testid="agent-run-detail"]');
    expect(panel).not.toBeNull();
    expect(panel?.textContent).toContain("Succeeded");
    expect(panel?.textContent).not.toContain("Running");
  });

  it("discards a stale list response after the operator switches teammates", async () => {
    // The engineer's list read is slow; the operator moves to Dana before it
    // resolves. The late answer was filtered against the old agentId and must
    // not render the engineer's rows beneath Dana's name.
    let releaseEngineer!: (rows: RunSummary[]) => void;
    const engineerGate = new Promise<RunSummary[]>((resolve) => {
      releaseEngineer = resolve;
    });

    const get = vi.fn(async (path: string) => {
      if (path.includes("agent=engineer")) return engineerGate;
      if (path.includes("agent=designer"))
        return [run({ id: "designer-run", agentId: "designer" })];
      if (path.startsWith("/tasks")) return [];
      if (path.startsWith("/workflows")) return [];
      throw new Error(`no route for ${path}`);
    });
    const client = {
      get,
      scopeFor: () => "",
    } as unknown as OpenCompanyClient & { get: typeof get };

    await mount(client);
    await mount(client, { agentId: "designer", agentName: "Dana" });

    expect(
      container.querySelector('[data-testid="agent-run-designer-run"]'),
    ).not.toBeNull();

    // The engineer's answer arrives after the switch.
    await act(async () => {
      releaseEngineer([run({ id: "engineer-run" })]);
    });
    await act(async () => {});

    expect(
      container.querySelector('[data-testid="agent-run-engineer-run"]'),
    ).toBeNull();
    expect(
      container.querySelector('[data-testid="agent-run-designer-run"]'),
    ).not.toBeNull();
  });
});
