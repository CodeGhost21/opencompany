// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { Task, TaskWorkflowProposal } from "@/api/tasks";
import { TaskWorkflowProposalPanel } from "@/views/TaskWorkflowProposalPanel";

/**
 * Approving a built workflow proposal.
 *
 * `POST …/tasks/{id}/workflow-proposal/apply` is `ScopedCompany`
 * (`src/server/ops/tasks.rs`), the same guard every other task write carries —
 * not `AdminScopedCompany`. `TaskWorkflowProposalPanel` matches: no role check
 * gates the Apply button, so the AUTH claim is that the control genuinely
 * reaches the host for any viewer.
 *
 * `task-workflow-proposal.test.ts` pins the pure adapter (`taskProposalDiff`)
 * and the refusal-breakdown decision; it never renders this panel or clicks
 * Apply, so a refusal keeping the card in review — rather than moving it or
 * claiming success — has never actually been driven through the DOM.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

/** A valid, renderable proposal — `taskProposalDiff` accepts this shape. */
const PROPOSAL: TaskWorkflowProposal = {
  summary: "Posts a weekly summary of dev activity.",
  generatedAtMillis: 1_700_000_000_000,
  runId: "run-1",
  ops: {
    id: "weekly_summary",
    name: "Weekly summary",
    nodes: [
      { id: "cron", kind: "trigger", name: "Weekly", schedule: "0 9 * * 1" },
      { id: "gather", kind: "agent", name: "Gather", agent: "analyst" },
      { id: "post", kind: "output", name: "Post", destination: { kind: "owner" } },
    ],
    edges: [
      { from: "cron", to: "gather" },
      { from: "gather", to: "post" },
    ],
  },
};

function task(): Task {
  return {
    id: "task-1",
    title: "Build the weekly summary",
    column: "working",
    stage: "in_review",
    priority: "medium",
    assignee: "analyst",
    updatedAt: 0,
    workflowProposal: PROPOSAL,
  } as Task;
}

function clientAs(post: (path: string, body?: unknown) => Promise<unknown>): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company ?? "acme"}`,
    post: async (path: string, body?: unknown) => post(path, body),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function render(client: OpenCompanyClient, onReload = vi.fn()) {
  await act(async () => {
    root.render(
      createElement(TaskWorkflowProposalPanel, {
        client,
        company: "acme",
        task: task(),
        onReload,
      }),
    );
  });
  return onReload;
}

function applyButton(): HTMLButtonElement {
  const found = document.querySelector('[data-testid="task-workflow-proposal-apply"]');
  if (!found) throw new Error(`no Apply control in:\n${container.innerHTML}`);
  return found as HTMLButtonElement;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("Apply a workflow proposal — offered to every member, not just an admin", () => {
  it("reaches the host's apply on click, with no admin-only gate", async () => {
    const post = vi.fn(async (_path: string, _body?: unknown) => ({ ...task(), workflowProposal: undefined, column: "done" }));
    const onReload = await render(clientAs(post));

    expect(applyButton().disabled).toBe(false);
    await act(async () => {
      applyButton().click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(post.mock.calls[0][0]).toBe(
      "/api/v1/companies/acme/tasks/task-1/workflow-proposal/apply",
    );
    expect(onReload).toHaveBeenCalledTimes(1);
    expect(toasts.success).toHaveBeenCalled();
  });
});

describe("Apply a workflow proposal — the host refuses it", () => {
  it("keeps the card in review, shows the host's reason, and never claims success", async () => {
    const post = vi.fn(async () => {
      throw new ApiError(400, "conflict", "a workflow named “weekly_summary” already exists", true);
    });
    const onReload = await render(clientAs(post));

    await act(async () => {
      applyButton().click();
    });

    expect(toasts.success).not.toHaveBeenCalled();
    // Never reloads the card into whatever "settled" state the caller would
    // otherwise assume — a refused Apply is not a state transition.
    expect(onReload).not.toHaveBeenCalled();
    const error = document.querySelector('[data-testid="task-workflow-proposal-error"]');
    expect(error?.textContent).toContain(
      "a workflow named “weekly_summary” already exists",
    );
    // The proposal panel — and its Apply control — is still on screen: the
    // card stayed in review rather than the panel simply vanishing.
    expect(applyButton()).toBeDefined();
    expect(applyButton().disabled).toBe(false);
  });
});
