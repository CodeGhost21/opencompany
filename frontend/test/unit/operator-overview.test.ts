// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunSummary } from "@/api/runs";
import type { CompanyFeed } from "@/hooks/use-company";
import { OperatorOverview } from "@/views/OperatorOverview";

let container: HTMLDivElement;
let root: Root;

const scope = { connection: "test-host", company: "acme" };
const readyFeed = { approvals: [], queue: "ready" as const };

function run(over: Partial<RunSummary> = {}): RunSummary {
  return {
    id: "run-1",
    taskId: "task-1",
    agentId: "maya",
    attempt: 1,
    status: "failed",
    phase: "terminal",
    createdAtMillis: 1_700_000_000_000,
    finishedAtMillis: 1_700_000_000_100,
    usage: { input: 0, output: 0, cachedInput: 0, costUsd: 0 },
    stepCount: 0,
    stepCountCapped: false,
    ...over,
  };
}

function client(runs: Promise<RunSummary[]>): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: () => runs,
  } as unknown as OpenCompanyClient;
}

/** A client that answers the two run reads this page makes differently. */
function clientByUrl(
  answer: (url: string) => Promise<RunSummary[]>,
): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: (url) => answer(String(url)),
  } as unknown as OpenCompanyClient;
}

async function render(
  host: OpenCompanyClient,
  feed: Pick<CompanyFeed, "approvals" | "queue">,
  attemptEventTick?: number,
) {
  await act(async () => {
    root.render(
      createElement(OperatorOverview, {
        client: host,
        company: "acme",
        companyName: "Acme",
        feed,
        scope,
        ...(attemptEventTick === undefined ? {} : { attemptEventTick }),
      }),
    );
  });
}

async function settle() {
  await act(async () => {
    await Promise.resolve();
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.localStorage.clear();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the operator overview landing page (#1321)", () => {
  it("has one primary action and routes attention to the real queues", async () => {
    await render(client(Promise.resolve([])), {
      approvals: [{ id: "approval-1" }] as CompanyFeed["approvals"],
      queue: "ready",
    });
    await settle();

    expect(container.querySelector('[href="#/chat"]')?.textContent).toContain("Start a conversation");
    expect(container.querySelector('[href="#/approvals"]')?.textContent).toContain("Review approvals");
    expect(container.querySelector('[href="#/company/graph"]')?.textContent).toContain("knowledge graph");
    expect(container.textContent).toContain("No work is paused or failed right now.");
  });

  it("keeps loading and unreadable queue states distinct from an empty queue", async () => {
    let resolveRuns: (runs: RunSummary[]) => void;
    const pending = new Promise<RunSummary[]>((resolve) => {
      resolveRuns = resolve;
    });
    await render(client(pending), { approvals: [], queue: "loading" });

    expect(container.textContent).toContain("Loading approvals…");
    expect(container.textContent).toContain("Loading recent work…");

    await act(async () => resolveRuns!([]));
    await render(client(Promise.reject(new Error("offline"))), { approvals: [], queue: "error" });
    await settle();

    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Couldn't read what needs your approval");
    expect(container.textContent).not.toContain("Nothing is waiting for your approval.");
  });

  it("uses the persisted browser boundary to show failed work since the prior visit", async () => {
    window.localStorage.setItem("oc.overview.last-visit:test-host::acme", "1700000000000");
    await render(client(Promise.resolve([run()])), readyFeed);
    await settle();

    expect(container.textContent).toContain("Failed attempts recorded after the previous visit.");
    expect(container.querySelector('[href="#/tasks/task-1?run=run-1"]')?.textContent).toContain("Open");
  });

  it("reads failures on their own page, so paused attempts cannot crowd one out of the since-visit answer", async () => {
    window.localStorage.setItem("oc.overview.last-visit:test-host::acme", "1700000000000");
    const paused = run({
      id: "paused-1",
      status: "paused",
      phase: "parked",
      finishedAtMillis: 1_700_000_000_200,
    });
    const failed = run({
      id: "failed-1",
      finishedAtMillis: 1_700_000_000_100,
    });
    await render(
      clientByUrl((url) =>
        // The stopped panel's capped mixed page is all-paused — the failure
        // finished after the visit but is older than the paused pack, so it
        // would fall off that page. The since-visit panel reads its own
        // failed-only page, so it still sees the attempt.
        url.includes("status=failed,paused")
          ? Promise.resolve([paused])
          : Promise.resolve([failed]),
      ),
      readyFeed,
    );
    await settle();

    expect(container.textContent).toContain("Failed attempts recorded after the previous visit.");
    expect(container.querySelector('[href="#/tasks/task-1?run=failed-1"]')).not.toBeNull();
  });

  it("re-reads the run panels when the shell reports a run status change", async () => {
    let calls = 0;
    const host: OpenCompanyClient = {
      scopeFor: () => "/api/v1/company/acme",
      get: () => {
        calls += 1;
        return Promise.resolve([]);
      },
    } as unknown as OpenCompanyClient;

    await render(host, readyFeed, 0);
    await settle();
    const afterBoot = calls;
    expect(afterBoot).toBeGreaterThan(0);

    await render(host, readyFeed, 1);
    await settle();
    expect(calls).toBeGreaterThan(afterBoot);
  });
});
