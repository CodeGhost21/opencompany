// @vitest-environment jsdom
// Temporary diagnostic: does awaiting the settled fetch promise inside the
// reject act reliably render the error? Also try non-async client.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

type DeferredEntry = {
  promise: Promise<RunArtifactRow[]>;
  resolve: (rows: RunArtifactRow[]) => void;
  reject: (err: unknown) => void;
};

function deferredFilesClient(deferred: DeferredEntry[]): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: <T>(path: string): Promise<T> => {
      const promise = new Promise<T>((resolve, reject) => {
        deferred.push({
          promise: null as unknown as Promise<RunArtifactRow[]>,
          resolve,
          reject,
        });
        void path;
      });
      const entry = deferred[deferred.length - 1];
      entry.promise = promise as Promise<RunArtifactRow[]>;
      return promise;
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

it("diag: 30 iterations awaiting the settled promise inside the reject act", async () => {
  let misses = 0;
  for (let i = 0; i < 30; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const deferred: DeferredEntry[] = [];
    const client = deferredFilesClient(deferred);
    await renderPanel(completedRun("run-1"), client);
    await expandFiles();
    if (deferred.length !== 1) {
      console.log(`iter ${i}: deferred.length=${deferred.length}`);
      continue;
    }

    await act(async () => {
      deferred[0].reject(new Error("boom"));
      await deferred[0].promise.catch(() => {});
    });

    if (
      !container.querySelector('[data-testid="workflow-run-files-error"]')
    ) {
      misses++;
      console.log(
        `iter ${i}: MISS. DOM=${JSON.stringify(
          container.querySelector('[data-testid="workflow-run-files"]')
            ?.innerHTML,
        )}`,
      );
    }
  }
  console.log(`misses=${misses}/30`);
  expect(misses).toBe(0);
});
