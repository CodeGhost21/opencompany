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

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(AgentRuns, {
        client,
        company: null,
        agentId: "engineer",
        agentName: "Robin",
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
    const client = makeClient({ "/runs": [], "/tasks": [], "/workflows": [] });
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
    // The filter controls stay up so the filter can be turned off again.
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
});
