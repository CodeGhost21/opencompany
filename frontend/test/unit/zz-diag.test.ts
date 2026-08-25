// @vitest-environment jsdom
// Temporary diagnostic: reproduce the mode-B failure (error NEVER renders).

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

it("diag: 30 fresh runs of the reject-render path", async () => {
  let misses = 0;
  let timeouts = 0;
  for (let i = 0; i < 30; i++) {
    // fresh container/root per iteration, like beforeEach/afterEach
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
    if (deferred.length !== 1) {
      console.log(`iter ${i}: deferred.length=${deferred.length}`);
      continue;
    }

    await act(async () => {
      deferred[0].reject(new Error("boom"));
    });

    const errorEl = () =>
      container.querySelector('[data-testid="workflow-run-files-error"]');
    if (errorEl()) {
      // immediate
      continue;
    }
    misses++;
    // poll with empty acts (the file's own idiom) up to 20 tries
    let appeared = false;
    for (let t = 0; t < 20; t++) {
      await act(async () => {});
      if (errorEl()) {
        appeared = true;
        break;
      }
    }
    if (!appeared) {
      timeouts++;
      console.log(
        `iter ${i}: error NEVER rendered after reject. DOM=${JSON.stringify(
          container.querySelector('[data-testid="workflow-run-files"]')
            ?.innerHTML,
        )}`,
      );
    }
  }
  console.log(`misses=${misses} neverRendered=${timeouts}`);
  expect(true).toBe(true);
});
