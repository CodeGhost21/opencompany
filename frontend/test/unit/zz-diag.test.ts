// @vitest-environment jsdom
// Temporary diagnostic: full timeline in the miss case — does the error ever
// appear, does the latch reset, and when?

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

function deferredFilesClient(
  deferred: {
    resolve: (rows: RunArtifactRow[]) => void;
    reject: (err: unknown) => void;
  }[],
): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      return new Promise<T>((resolve, reject) => {
        deferred.push({ resolve, reject });
        void path;
      });
    },
  } as unknown as OpenCompanyClient;
}

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

async function expandFiles() {
  const details = container.querySelector<HTMLDetailsElement>(
    '[data-testid="workflow-run-files"]',
  );
  await act(async () => {
    details!.open = true;
    details!.dispatchEvent(new Event("toggle", { bubbles: true }));
  });
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

it("diag: timeline after miss", async () => {
  let misses = 0;
  for (let i = 0; i < 25; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const deferred: {
      resolve: (rows: RunArtifactRow[]) => void;
      reject: (err: unknown) => void;
    }[] = [];
    const client = deferredFilesClient(deferred);
    await renderPanel(completedRun("run-1"), client);
    await expandFiles();

    await act(async () => {
      deferred[0].reject(new Error("boom"));
    });

    const err = () =>
      container.querySelector('[data-testid="workflow-run-files-error"]');
    if (err()) continue;
    misses++;

    const loading = () =>
      container.querySelector('[data-testid="workflow-run-files"]')
        ?.textContent?.includes("Loading…") ?? false;
    console.log(
      `iter ${i}: miss. loading=${loading()}. latch reset? -> `,
    );
    // Reopen: did the latch reset (catch ran)?
    const details = container.querySelector<HTMLDetailsElement>(
      '[data-testid="workflow-run-files"]',
    )!;
    await act(async () => {
      details.open = false;
      details.dispatchEvent(new Event("toggle", { bubbles: true }));
    });
    await expandFiles();
    const latchReset = deferred.length === 2;
    console.log(`  latchReset=${latchReset}`);
    // Does the error element exist NOW, after the reopen cycle?
    console.log(
      `  error after reopen=${!!err()} loading=${loading()} deferred=${deferred.length}`,
    );
  }
  console.log(`misses=${misses}`);
  expect(true).toBe(true);
});
