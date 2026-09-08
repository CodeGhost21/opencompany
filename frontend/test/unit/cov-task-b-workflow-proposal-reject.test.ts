// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { Task, TaskWorkflowProposal } from "@/api/tasks";
import { TaskWorkflowProposalPanel } from "@/views/TaskWorkflowProposalPanel";

/**
 * Rejecting a built workflow proposal.
 *
 * `POST …/tasks/{id}/workflow-proposal/reject` is `ScopedCompany`
 * (`src/server/ops/tasks.rs`) — the same "any member" guard `apply` carries,
 * not `AdminScopedCompany`. The Reject control (unlike Apply) is also offered
 * on a proposal the diff adapter refused to render, so it is the one way off a
 * stuck card and is worth pinning that it stays reachable even then.
 *
 * `task-workflow-proposal.test.ts` never renders this panel, so a real reject
 * click reaching the host — and a refused reject leaving the proposal intact
 * — is untested until this file.
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

/** A proposal the diff adapter refuses to render — Apply is withheld on this. */
const UNRENDERABLE_PROPOSAL: TaskWorkflowProposal = {
  ...PROPOSAL,
  ops: { id: "x", name: "Y", nodes: [], edges: [] },
};

function task(proposal: TaskWorkflowProposal): Task {
  return {
    id: "task-1",
    title: "Build the weekly summary",
    column: "working",
    stage: "in_review",
    priority: "medium",
    assignee: "analyst",
    updatedAt: 0,
    workflowProposal: proposal,
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

async function render(
  client: OpenCompanyClient,
  proposal: TaskWorkflowProposal = PROPOSAL,
  onReload = vi.fn(),
) {
  await act(async () => {
    root.render(
      createElement(TaskWorkflowProposalPanel, {
        client,
        company: "acme",
        task: task(proposal),
        onReload,
      }),
    );
  });
  return onReload;
}

function rejectButton(): HTMLButtonElement {
  const found = document.querySelector('[data-testid="task-workflow-proposal-reject"]');
  if (!found) throw new Error(`no Reject control in:\n${container.innerHTML}`);
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

describe("Reject a workflow proposal — offered to every member, not just an admin", () => {
  it("reaches the host's reject on click, with no admin-only gate", async () => {
    const post = vi.fn(async (_path: string, _body?: unknown) => ({ ...task(PROPOSAL), workflowProposal: undefined, column: "todo" }));
    const onReload = await render(clientAs(post));

    await act(async () => {
      rejectButton().click();
    });

    expect(post).toHaveBeenCalledTimes(1);
    expect(post.mock.calls[0][0]).toBe(
      "/api/v1/companies/acme/tasks/task-1/workflow-proposal/reject",
    );
    expect(onReload).toHaveBeenCalledTimes(1);
    expect(toasts.success).toHaveBeenCalled();
  });

  it("stays reachable even on a proposal Apply withholds", async () => {
    const post = vi.fn(async () => ({ ...task(UNRENDERABLE_PROPOSAL), workflowProposal: undefined }));
    await render(clientAs(post), UNRENDERABLE_PROPOSAL);

    // Apply is withheld — the adapter could not render this graph — but
    // Reject is the one way off a card stuck like this, so it must not be
    // withheld along with it.
    expect(
      document.querySelector('[data-testid="task-workflow-proposal-apply"]'),
    ).toBeNull();
    expect(rejectButton().disabled).toBe(false);
  });
});

describe("Reject a workflow proposal — the host refuses it", () => {
  it("keeps the proposal intact, shows the host's reason, and never claims success", async () => {
    const post = vi.fn(async () => {
      throw new ApiError(409, "conflict", "this card no longer has a proposal to reject", true);
    });
    const onReload = await render(clientAs(post));

    await act(async () => {
      rejectButton().click();
    });

    expect(toasts.success).not.toHaveBeenCalled();
    expect(onReload).not.toHaveBeenCalled();
    const error = document.querySelector('[data-testid="task-workflow-proposal-error"]');
    expect(error?.textContent).toContain(
      "this card no longer has a proposal to reject",
    );
    expect(rejectButton().disabled).toBe(false);
  });
});
