import { beforeEach, describe, expect, it, vi } from "vitest";

import type { WorkflowGraph } from "@/api/workflows";
import { foldLiveRun } from "@/views/workflows/graph";

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

const { handleEvent } = await import("@/hooks/use-events");
type Ev = import("@/hooks/use-events").CompanyStreamEvent;
type Subs = import("@/hooks/use-events").Subscribers;

/**
 * Issue #382: "currently executing" is now REPORTED by the host, not derived.
 *
 * The engine gained an `on_step_start` hook, so a run brackets each non-trigger
 * node with a `workflow_node_started` frame ahead of its `workflow_node_finished`.
 * Two contracts carry that on the console:
 *
 * - `foldLiveRun` marks a node `running` when its start frame arrives and settles
 *   it on the finish frame — no more topology-derived frontier that over-marked
 *   both arms of a branch; and
 * - the frame reaches the run subscriber rather than falling through `default:`
 *   and vanishing (the trap this file has been bitten by three times).
 */

/** `start → ceo → done` — one trigger, one agent, one output. */
const GRAPH: WorkflowGraph = {
  id: "greet",
  name: "Greet",
  nodes: [
    { id: "start", kind: "trigger", name: "Start" },
    { id: "ceo", kind: "agent", name: "CEO", agent: "ceo" },
    { id: "done", kind: "output", name: "Done" },
  ],
  edges: [
    { from: "start", to: "ceo" },
    { from: "ceo", to: "done" },
  ],
};

const start = (runId: string): Ev => ({
  type: "workflow_run_started",
  seq: 1,
  atMillis: 1,
  workflowId: "greet",
  runId,
  scheduled: false,
});
const nodeStarted = (runId: string, nodeId: string): Ev => ({
  type: "workflow_node_started",
  seq: 2,
  atMillis: 2,
  workflowId: "greet",
  runId,
  nodeId,
});
const nodeFinished = (runId: string, nodeId: string, status: string): Ev => ({
  type: "workflow_node_finished",
  seq: 3,
  atMillis: 3,
  workflowId: "greet",
  runId,
  nodeId,
  status,
  elapsedMs: 5,
});
const finished = (runId: string): Ev => ({
  type: "workflow_run_finished",
  seq: 4,
  atMillis: 4,
  workflowId: "greet",
  scheduled: false,
  deliveries: [],
  pendingApprovals: [],
  runId,
});

describe("foldLiveRun reports running from node-started frames", () => {
  it("only the trigger is marked before any node-started frame arrives", () => {
    // The old fold lit up the trigger's successors as a guessed frontier. Now
    // nothing but the trigger is marked until the host says a node started.
    const live = foldLiveRun([start("r1")], "greet", GRAPH);
    expect(live?.states).toEqual({ start: "ok" });
  });

  it("marks a node running on its started frame, then settles it on finish", () => {
    const live = foldLiveRun(
      [start("r1"), nodeStarted("r1", "ceo")],
      "greet",
      GRAPH,
    );
    expect(live?.states.ceo).toBe("running");
    // `done` is NOT lit — nothing derives it any more; it waits for its own
    // started frame.
    expect(live?.states.done).toBeUndefined();

    const settled = foldLiveRun(
      [start("r1"), nodeStarted("r1", "ceo"), nodeFinished("r1", "ceo", "ok")],
      "greet",
      GRAPH,
    );
    expect(settled?.states.ceo).toBe("ok");
    expect(settled?.active).toBe(true);
  });

  it("a finished frame cannot be downgraded by a stray later start", () => {
    // Ordering guarantees start precedes finish, but the guard must hold even if
    // a frame arrives out of order: a settled node stays settled.
    const live = foldLiveRun(
      [start("r1"), nodeFinished("r1", "ceo", "ok"), nodeStarted("r1", "ceo")],
      "greet",
      GRAPH,
    );
    expect(live?.states.ceo).toBe("ok");
  });

  it("clears an orphaned running mark once the run settles (cancel/crash)", () => {
    // ceo started but never finished — the run ended on it. The settled sweep
    // drops the orphan so the canvas does not pulse "running" forever.
    const live = foldLiveRun(
      [start("r1"), nodeStarted("r1", "ceo"), finished("r1")],
      "greet",
      GRAPH,
    );
    expect(live?.active).toBe(false);
    expect(live?.states.ceo).toBeUndefined();
    // The trigger's reported "ok" is not a frontier guess, so it stays.
    expect(live?.states.start).toBe("ok");
  });

  it("ignores node-started frames from a different run on the shared stream", () => {
    // One SSE connection carries every run in the company; a concurrent run's
    // start frame must not light a node on the run being watched.
    const live = foldLiveRun(
      [start("r1"), nodeStarted("r2", "ceo")],
      "greet",
      GRAPH,
    );
    expect(live?.states.ceo).toBeUndefined();
  });
});

describe("workflow_node_started routing", () => {
  function subscribers() {
    return {
      onAgentReply: vi.fn(),
      onTaskEvent: vi.fn(),
      onWorkspaceEvent: vi.fn(),
      onTurnEvent: vi.fn(),
      onWorkflowRunEvent: vi.fn(),
      onWorkflowChanged: vi.fn(),
      onApprovalEvent: vi.fn(),
    } satisfies Subs;
  }

  beforeEach(() => {
    for (const fn of Object.values(toasts)) fn.mockClear();
  });

  it("reaches the run subscriber and raises no toast", () => {
    const subs = subscribers();
    handleEvent(nodeStarted("r1", "ceo"), subs);

    expect(subs.onWorkflowRunEvent).toHaveBeenCalledTimes(1);
    expect(subs.onWorkflowRunEvent).toHaveBeenCalledWith(nodeStarted("r1", "ceo"));
    // Progress is not an attention signal — a node lighting up on the canvas is
    // the feedback, not a toast.
    for (const fn of Object.values(toasts)) expect(fn).not.toHaveBeenCalled();
    // And no cross-wire to an unrelated surface.
    expect(subs.onTaskEvent).not.toHaveBeenCalled();
    expect(subs.onWorkflowChanged).not.toHaveBeenCalled();
    expect(subs.onWorkspaceEvent).not.toHaveBeenCalled();
  });
});
