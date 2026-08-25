// @vitest-environment jsdom
// Temporary diagnostic: is the async `get` wrapper the culprit? Compare
// async vs non-async deferred clients under a plain reject-inside-act.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

let container: HTMLDivElement;
let root: Root;

type DeferredEntry = {
  resolve: (rows: RunArtifactRow[]) => void;
  reject: (err: unknown) => void;
};

function deferredClient(deferred: DeferredEntry[], asyncGet: boolean): OpenCompanyClient {
  const getImpl = (path: string): Promise<unknown> => {
    return new Promise((resolve, reject) => {
      deferred.push({ resolve, reject } as DeferredEntry);
      void path;
    });
  };
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: asyncGet
      ? async <T>(path: string): Promise<T> => (getImpl(path) as Promise<T>)
      : (getImpl as unknown as (path: string) => Promise<unknown>),
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

const ITERATIONS = 30;

async function runVariant(asyncGet: boolean): Promise<number> {
  let misses = 0;
  for (let i = 0; i < ITERATIONS; i++) {
    await act(async () => root.unmount());
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);

    const deferred: DeferredEntry[] = [];
    const client = deferredClient(deferred, asyncGet);
    await renderPanel(completedRun("run-1"), client);
    await expandFiles();
    if (deferred.length !== 1) continue;

    await act(async () => {
      deferred[0].reject(new Error("boom"));
    });

    if (
      !container.querySelector('[data-testid="workflow-run-files-error"]')
    ) {
      misses++;
    }
  }
  return misses;
}

it("diag: async vs non-async get miss rates", async () => {
  for (const asyncGet of [false, true]) {
    const misses = await runVariant(asyncGet);
    console.log(`asyncGet=${asyncGet} misses=${misses}/${ITERATIONS}`);
  }
  expect(true).toBe(true);
});
