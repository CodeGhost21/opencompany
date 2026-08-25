// @vitest-environment jsdom
// Temporary diagnostic: in the stuck case, did the .catch run at all?
// Reopen after a miss: if requested.current was reset, a 2nd fetch fires.

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

it("diag: reopen-after-miss -> did the latch reset?", async () => {
  let latchReset = 0;
  let latchStuck = 0;
  for (let i = 0; i < 20; i++) {
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

    const errorEl = container.querySelector(
      '[data-testid="workflow-run-files-error"]',
    );
    if (errorEl) continue; // rendered, not the flake case

    // Miss. Reopen: does a 2nd fetch fire (latch reset) or not (catch never ran)?
    const details = container.querySelector<HTMLDetailsElement>(
      '[data-testid="workflow-run-files"]',
    )!;
    await act(async () => {
      details.open = false;
      details.dispatchEvent(new Event("toggle", { bubbles: true }));
    });
    await expandFiles();
    if (deferred.length === 2) {
      latchReset++;
      console.log(`iter ${i}: latch RESET (catch ran) but render stuck`);
    } else {
      latchStuck++;
      console.log(
        `iter ${i}: latch NOT reset (deferred.length=${deferred.length})`,
      );
    }
  }
  console.log(`latchReset=${latchReset} latchStuck=${latchStuck}`);
  expect(true).toBe(true);
});
