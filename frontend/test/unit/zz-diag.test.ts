// @vitest-environment jsdom
// Temporary diagnostic: which expand gesture fires load() exactly once and
// renders the error reliably? Compare:
//   A) details.open = true only (native async toggle)
//   B) summary click

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

function completedRun(runId: string): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_000,
    workflowId: "launch",
    scheduled: false,
    runId,
    deliveries: [],
    pendingApprovals: [],
  };
}

async function renderPanel(run: WorkflowRunOutcome, client: OpenCompanyClient) {
  await act(async () => {
    root.render(
      createElement(RunHistoryPanel, {
        client,
        company: "acme",
        runs: [run],
        graph: null,
        workflowName: "Launch",
        onClose: () => {},
        selectedRunSeq: null,
        onSelectRun: () => {},
      }),
    );
  });
}

function rejectClient(sink: { calls: string[] }): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      sink.calls.push(path);
      return Promise.reject(new Error("boom")) as never;
    },
  } as unknown as OpenCompanyClient;
}

async function expandByOpen() {
  const details = container.querySelector<HTMLDetailsElement>(
    '[data-testid="workflow-run-files"]',
  );
  await act(async () => {
    details!.open = true;
  });
  await act(async () => {});
  await act(async () => {});
}

async function expandBySummaryClick() {
  const summary = container.querySelector<HTMLElement>(
    '[data-testid="workflow-run-files-toggle"]',
  );
  await act(async () => {
    summary!.click();
  });
  await act(async () => {});
  await act(async () => {});
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

it("diag: expand-by-open native toggle reliability", async () => {
  let misses = 0;
  const callCounts = new Map<number, number>();
  for (let i = 0; i < 30; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const sink = { calls: [] as string[] };
    await renderPanel(completedRun("run-1"), rejectClient(sink));
    await expandByOpen();

    const err = container.querySelector(
      '[data-testid="workflow-run-files-error"]',
    );
    callCounts.set(sink.calls.length, (callCounts.get(sink.calls.length) ?? 0) + 1);
    if (!err || sink.calls.length !== 1) {
      misses++;
      console.log(`iter ${i}: MISS. calls=${sink.calls.length} err=${!!err}`);
    }
  }
  console.log(`open-only: misses=${misses}/30 callsHisto=${JSON.stringify([...callCounts.entries()])}`);
  expect(misses).toBe(0);
});

it("diag: expand-by-summary-click reliability", async () => {
  let misses = 0;
  const callCounts = new Map<number, number>();
  for (let i = 0; i < 30; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const sink = { calls: [] as string[] };
    await renderPanel(completedRun("run-1"), rejectClient(sink));
    await expandBySummaryClick();

    const err = container.querySelector(
      '[data-testid="workflow-run-files-error"]',
    );
    callCounts.set(sink.calls.length, (callCounts.get(sink.calls.length) ?? 0) + 1);
    if (!err || sink.calls.length !== 1) {
      misses++;
      console.log(`iter ${i}: MISS. calls=${sink.calls.length} err=${!!err}`);
    }
  }
  console.log(`summary-click: misses=${misses}/30 callsHisto=${JSON.stringify([...callCounts.entries()])}`);
  expect(misses).toBe(0);
});
