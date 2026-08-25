// @vitest-environment jsdom
// Temporary diagnostic: reject inside the SAME act as the toggle that fires
// the fetch — does the error render reliably?

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

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

it("diag: 30 iterations rejecting inside the toggle act", async () => {
  let misses = 0;
  for (let i = 0; i < 30; i++) {
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

    const details = container.querySelector<HTMLDetailsElement>(
      '[data-testid="workflow-run-files"]',
    )!;
    await act(async () => {
      details.open = true;
      details.dispatchEvent(new Event("toggle", { bubbles: true }));
      if (deferred.length !== 1) {
        throw new Error(`deferred.length=${deferred.length}`);
      }
      deferred[0].reject(new Error("boom"));
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
