// Which of the company's parked approvals belong to the run on screen (#1002).
//
// The run drawer already *tells* an operator their run stopped for a person —
// the parked badge and the "Not finished — …" sentence (#880 / #881) — and gave
// them nowhere to act: `resolveApproval` was reachable from the Approvals page
// and the shell alone, so deciding meant leaving the run, finding the row in a
// flat queue, and coming back. This is the join that lets a second surface ask
// the same question the page asks, narrowed to one run.
//
// ## Purely additive
//
// Nothing here filters the queue. These helpers take the queue the Approvals
// page and the sidebar badge already read (`feed.approvals`) and *select* from
// it for one run; the page keeps showing every row, and the badge keeps
// counting every row. An approval no workflow run parked — a chat turn, a
// scheduler tick, a task attempt — matches nothing here and belongs to the page
// alone, exactly as it did before this shipped.
//
// ## Live, never a snapshot
//
// The run body carries its own receipt of what it parked
// (`WorkflowRunApprovalRow`), and that receipt is deliberately frozen: #880
// wrote it as "this run opened this card", never as "this card is still
// waiting". Deriving the drawer's blocking list from it would go stale the
// instant anyone decided anything, anywhere. So the *rows* come from the live
// queue and the receipt is used only for the one thing it is authoritative
// about — which node a card was parked by.

import type { ApprovalSummary, Verdict } from "@/api/types";
import type { WorkflowRunApprovalRow } from "@/api/workflows";
import type { DecidedApproval } from "@/views/chat/model";

/** One approval the run on screen is held on, and what has become of it. */
export interface RunApproval {
  approval: ApprovalSummary;
  /**
   * The verdict already witnessed for it — from this drawer, from the Approvals
   * page, or from another operator's console over the event stream — or `null`
   * while it is still waiting on somebody.
   */
  verdict: Verdict | null;
  /**
   * The agent node whose turn parked it, when the run's receipt named one.
   * `null` for a card the receipt does not cover (an older host, or a park the
   * run recorded without node identity).
   */
  nodeId: string | null;
}

/**
 * The still-queued approvals this run parked.
 *
 * `runId` may legitimately be absent — a host predating #371 hands back no run
 * correlation id — and an absent id joins to **nothing** rather than to
 * everything. Matching every unlinked approval there would put another run's
 * cards, and a scheduler's, under this run's steps.
 */
export function approvalsForRun(
  approvals: readonly ApprovalSummary[],
  runId: string | null | undefined,
): ApprovalSummary[] {
  if (!runId) return [];
  return approvals.filter((a) => a.workflow_run_id === runId);
}

/** Approval id → the node that parked it, from the run's own receipt (#880). */
export function nodeByApprovalId(
  rows: readonly WorkflowRunApprovalRow[] | undefined,
): Map<string, string> {
  const byId = new Map<string, string>();
  for (const row of rows ?? []) {
    if (row.approvalId && row.nodeId) byId.set(row.approvalId, row.nodeId);
  }
  return byId;
}

/**
 * Every card this run is (or was) held on, in a stable order.
 *
 * Two sources, and both are needed:
 *
 *  - the **live queue**, which is the whole point — a card decided anywhere
 *    leaves it, so the drawer stops calling that step blocked without a reload;
 *  - the shell's **witnessed decisions**, which keep the last-seen summary of a
 *    card the queue has already dropped, so the operator's own decision settles
 *    in place instead of blinking out of the panel. Exactly what the inline chat
 *    card does with the same map, and for the same reason.
 *
 * Ordered by park time then id, which is stable across a refresh: the queue's
 * array identity changes on every poll, and a list that reordered under the
 * operator's cursor would be its own bug.
 */
export function runApprovals(
  approvals: readonly ApprovalSummary[],
  decided: Readonly<Record<string, DecidedApproval>>,
  runId: string | null | undefined,
  receipt?: readonly WorkflowRunApprovalRow[],
): RunApproval[] {
  if (!runId) return [];
  const nodes = nodeByApprovalId(receipt);
  const byId = new Map<string, RunApproval>();
  for (const approval of approvalsForRun(approvals, runId)) {
    byId.set(approval.id, {
      approval,
      // A decision witnessed while the queue has not yet dropped the row: the
      // resolve's answer and the feed's next poll are two moments, and for that
      // gap the row must read as decided rather than offering its buttons again.
      verdict: decided[approval.id]?.verdict ?? null,
      nodeId: nodes.get(approval.id) ?? null,
    });
  }
  for (const [id, d] of Object.entries(decided)) {
    if (byId.has(id) || d.approval.workflow_run_id !== runId) continue;
    byId.set(id, { approval: d.approval, verdict: d.verdict, nodeId: nodes.get(id) ?? null });
  }
  return [...byId.values()].sort(
    (a, b) =>
      a.approval.at_millis - b.approval.at_millis ||
      (a.approval.id < b.approval.id ? -1 : a.approval.id > b.approval.id ? 1 : 0),
  );
}

/** The subset still waiting on somebody — what the run is actually held on. */
export function blockingApprovals(rows: readonly RunApproval[]): RunApproval[] {
  return rows.filter((r) => r.verdict === null);
}
