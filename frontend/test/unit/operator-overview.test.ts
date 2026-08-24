// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunSummary } from "@/api/runs";
import type { DeskDto } from "@/api/types";
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

function client(
  runs: Promise<RunSummary[]>,
  desks?: Promise<DeskDto[]>,
): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: () => runs,
    // The desks read is best-effort and may be absent entirely — a mock that
    // does not implement `listDesks` exercises the degraded DM default.
    ...(desks ? { listDesks: () => desks } : {}),
  } as unknown as OpenCompanyClient;
}

/** A client that answers the two run reads this page makes differently. */
function clientByUrl(
  answer: (url: string) => Promise<RunSummary[]>,
): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    // The `as unknown` cast above drops the contextual type for the object
    // literal's methods, so the parameter would otherwise be implicitly `any`.
    get: (url: string) => answer(url),
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
        // `URLSearchParams` percent-encodes the comma, so the stopped page's
        // `status=failed%2Cpaused` is what actually hits the wire.
        url.includes("status=failed%2Cpaused")
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

  it("does not let a slower initial snapshot overwrite a fresher tick re-read", async () => {
    // The tick refresh added in #1015 races the initial load when a run parks
    // or fails while the first snapshot is still outstanding. The generation
    // ticket must make the *latest* read win even when the initial answer
    // lands last — otherwise the fresher lists get overwritten by stale ones.
    window.localStorage.setItem("oc.overview.last-visit:test-host::acme", "1700000000000");
    const tickFailed = run({ id: "tick-failed", taskId: "tick-failed", finishedAtMillis: 1_700_000_000_100 });
    const initialFailed = run({ id: "initial-failed", taskId: "initial-failed", finishedAtMillis: 1_700_000_000_100 });
    const stoppedResolvers: Array<(runs: RunSummary[]) => void> = [];
    const failedResolvers: Array<(runs: RunSummary[]) => void> = [];
    const host: OpenCompanyClient = {
      scopeFor: () => "/api/v1/company/acme",
      get: (url: string) =>
        new Promise<RunSummary[]>((resolve) => {
          const u = String(url);
          (u.includes("status=failed%2Cpaused") ? stoppedResolvers : failedResolvers).push(resolve);
        }),
    } as unknown as OpenCompanyClient;

    await render(host, readyFeed, 0);
    await settle();
    expect(stoppedResolvers).toHaveLength(1);
    expect(failedResolvers).toHaveLength(1);

    // A run status change lands while the initial snapshot is outstanding.
    await render(host, readyFeed, 1);
    await settle();
    expect(stoppedResolvers).toHaveLength(2);
    expect(failedResolvers).toHaveLength(2);

    // The tick re-read answers first with a fresh failure…
    await act(async () => {
      failedResolvers[1]!([tickFailed]);
      stoppedResolvers[1]!([]);
    });
    expect(container.textContent).toContain("Task tick-failed");

    // …then the stale initial snapshot lands; it must not overwrite it.
    await act(async () => {
      failedResolvers[0]!([initialFailed]);
      stoppedResolvers[0]!([]);
    });
    expect(container.textContent).toContain("Task tick-failed");
    expect(container.textContent).not.toContain("Task initial-failed");
  });

  it("does not claim no failures since the visit when the failed read came back capped", async () => {
    // The host clamps the run list read, so a full page of failures cannot
    // prove the absence of older ones that finished after the visit. The
    // empty state must say it is looking at the newest cap, not claim the
    // whole history was read.
    window.localStorage.setItem("oc.overview.last-visit:test-host::acme", "1700000000000");
    const capped = Array.from({ length: 200 }, (_, i) =>
      run({ id: `old-${i}`, finishedAtMillis: 1_600_000_000_000 }),
    );
    await render(client(Promise.resolve(capped)), readyFeed);
    await settle();

    expect(container.textContent).toContain("the host caps the read here");
    expect(container.textContent).not.toContain("No failed attempts were recorded since the previous visit.");
  });

  it("links an operator-chat run to its desk channel when the chat id names a desk", async () => {
    const chatRun = run({
      id: "chat-desk-1",
      taskId: undefined,
      chatId: "engineering",
      agentId: "engineering",
    });
    await render(
      client(
        Promise.resolve([chatRun]),
        Promise.resolve([{ id: "engineering", name: "Engineering desk", members: [] }]),
      ),
      readyFeed,
    );
    await settle();

    // A desk's channel id is its thread id, so the run links by bare id.
    expect(container.querySelector('[href="#/chat/engineering"]')?.textContent).toContain("Open");
    expect(container.textContent).toContain("Conversation work");
  });

  it("links an operator-chat run to its DM when the chat id is not a known desk", async () => {
    const chatRun = run({
      id: "chat-dm-1",
      taskId: undefined,
      chatId: "ada-1f3k",
      agentId: "maya",
    });
    await render(client(Promise.resolve([chatRun]), Promise.resolve([])), readyFeed);
    await settle();

    expect(container.querySelector('[href="#/chat/dm:ada-1f3k"]')?.textContent).toContain("Open");
  });

  it("keeps the alert icon only for attempts with neither a task nor a conversation", async () => {
    const stray = run({
      id: "stray-1",
      taskId: undefined,
      chatId: undefined,
    });
    await render(client(Promise.resolve([stray])), readyFeed);
    await settle();

    expect(container.textContent).toContain("Unattributed attempt");
    expect(
      container.querySelector('[aria-label="No task or conversation is attached to this attempt"]'),
    ).not.toBeNull();
    // The header's own `#/chat` CTA is excluded by requiring a segment after
    // the slash — an unattributed run must mint no task or thread link of its
    // own.
    expect(container.querySelector('a[href^="#/tasks/"], a[href^="#/chat/"]')).toBeNull();
  });
});
