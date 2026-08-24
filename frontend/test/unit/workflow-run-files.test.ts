// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { RunArtifactRow, WorkflowRunOutcome } from "@/api/workflows";
import { artifactHref } from "@/lib/task-output";
import { RunHistoryPanel } from "@/views/workflows/RunHistoryPanel";

/**
 * The "Files associated" disclosure on a run-history row (issue #1684).
 *
 * A completed run's row now links to the files that run produced, deep-linking
 * each into the card's Artifacts tab at the version the run wrote. The section
 * is the exception the unit runner earns the same way `workflow-run-board` does:
 * the thing under test IS what reaches the operator's eye — a lazy fetch that
 * must NOT fire until the row is expanded, and a link whose href is the exact
 * `artifactHref` the rest of the console navigates by — and neither can be
 * asserted anywhere but a render.
 */

let container: HTMLDivElement;
let root: Root;

/** A fake client that answers a canned files array and records every path asked
 * for — so a test can prove the fetch is lazy by asserting the sink is empty
 * until the row is expanded. */
function filesClient(
  rows: RunArtifactRow[],
  sink: { calls: string[] },
): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    get: async <T>(path: string): Promise<T> => {
      sink.calls.push(path);
      return rows as T;
    },
  } as unknown as OpenCompanyClient;
}

/** A completed, quiet run — the compact row the files section hangs off. */
function completedRun(runId: string | undefined): WorkflowRunOutcome {
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

const FILE: RunArtifactRow = {
  taskId: "t-a",
  artifactId: "art-a1",
  title: "Launch spec",
  kind: "markdown",
  source: "specs/launch.md",
  latestVersion: 2,
  updatedAtMillis: 30,
  taskTitle: "Draft the launch",
};

async function renderPanel(
  run: WorkflowRunOutcome,
  client: OpenCompanyClient,
): Promise<void> {
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

/** Opens the native `<details>` the way a click would, and flushes the fetch
 * that fires on first open. */
async function expandFiles(): Promise<void> {
  const details = container.querySelector<HTMLDetailsElement>(
    '[data-testid="workflow-run-files"]',
  );
  if (!details) throw new Error("no files disclosure on the row");
  await act(async () => {
    details.open = true;
    details.dispatchEvent(new Event("toggle", { bubbles: true }));
  });
  // A second empty act flushes the resolved-promise microtask's setState.
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

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  window.location.hash = "";
});

describe("run row — files associated (issue #1684)", () => {
  it("does not fetch on render, then fetches once on expand", async () => {
    const sink = { calls: [] as string[] };
    await renderPanel(completedRun("run-1"), filesClient([FILE], sink));

    // Lazy: a collapsed row makes zero network calls.
    expect(sink.calls).toEqual([]);
    expect(
      container.querySelector('[data-testid="workflow-run-file"]'),
    ).toBeNull();

    await expandFiles();

    expect(sink.calls).toEqual([
      "/api/v1/acme/workflows/runs/run-1/artifacts",
    ]);
    const entries = container.querySelectorAll(
      '[data-testid="workflow-run-file"]',
    );
    expect(entries.length).toBe(1);
  });

  it("deep-links each file into the Artifacts tab at the run's version", async () => {
    const sink = { calls: [] as string[] };
    await renderPanel(completedRun("run-1"), filesClient([FILE], sink));
    await expandFiles();

    const link = container.querySelector<HTMLAnchorElement>(
      '[data-testid="workflow-run-file"] a',
    );
    expect(link).not.toBeNull();
    expect(link?.getAttribute("href")).toBe(
      artifactHref("t-a", "art-a1", 2),
    );
    expect(link?.getAttribute("href")).toBe("#/tasks/t-a?artifact=art-a1&v=2");
    expect(link?.textContent).toContain("Launch spec");
  });

  it("offers the workspace link only when the file was mirrored", async () => {
    const sink = { calls: [] as string[] };
    await renderPanel(
      completedRun("run-1"),
      filesClient([{ ...FILE, workspaceNodeId: "node-9" }], sink),
    );
    await expandFiles();

    const wsLink = container.querySelector<HTMLAnchorElement>(
      '[data-testid="workflow-run-file-workspace"]',
    );
    expect(wsLink?.getAttribute("href")).toBe("#/workspace/node-9");
  });

  it("shows an empty state for a run that produced no files", async () => {
    const sink = { calls: [] as string[] };
    await renderPanel(completedRun("run-1"), filesClient([], sink));
    await expandFiles();

    expect(sink.calls.length).toBe(1);
    const empty = container.querySelector(
      '[data-testid="workflow-run-files-empty"]',
    );
    expect(empty?.textContent).toContain("No files from this run.");
    expect(
      container.querySelector('[data-testid="workflow-run-file"]'),
    ).toBeNull();
  });

  it("renders no files control for a run with no runId", async () => {
    const sink = { calls: [] as string[] };
    await renderPanel(completedRun(undefined), filesClient([FILE], sink));

    expect(
      container.querySelector('[data-testid="workflow-run-files"]'),
    ).toBeNull();
    expect(sink.calls).toEqual([]);
  });
});
