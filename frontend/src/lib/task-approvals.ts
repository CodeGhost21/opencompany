import type { TaskApproval } from "@/api/tasks";
import type { ApprovalSummary, Verdict } from "@/api/types";
import type { DecidedApproval } from "@/views/chat/model";

/**
 * What the task card says about approvals (issue #468).
 *
 * `null` when the card is not waiting on anything — the caller renders nothing
 * at all rather than a "no approvals" line, which is the clutter the removed
 * Approvals tab was made of.
 */
export interface PendingApprovalWait {
  /** How many of this task's approvals are still parked. */
  count: number;
  /** Epoch-millis the *oldest* parked one arrived. */
  since: number;
  /** How long the card has been stopped, measured to the caller's clock. */
  waited: number;
}

/**
 * Derives the card's one approval line from this task's approvals.
 *
 * Lives here rather than inline in the view for the reason every other
 * derivation in `lib/` does: getting it wrong produces a card that looks
 * completely normal while saying the wrong thing. A card that reports the
 * *newest* park reads as freshly blocked no matter how long it has actually sat
 * there, and nothing about the rendering would look broken.
 *
 * Three decisions worth stating, because each is a way this could be silently
 * wrong:
 *
 * - **Only `pending` counts.** `approved`, `denied` and `expired` are
 *   resolutions; they are already on the timeline with their own waited span,
 *   and counting them here would rebuild the removed tab one row at a time.
 * - **The oldest park wins**, not the newest. It is how long this card has
 *   really been stopped. With the newest, a second effect parking behind the
 *   first would reset a clock that should be climbing.
 * - **`waited` is clamped at zero.** `atMillis` comes from the host and `now`
 *   from the browser; a clock skew of a few seconds must not render "waiting
 *   for -3s".
 */
export function pendingApprovalWait(
  approvals: readonly TaskApproval[],
  now: number,
): PendingApprovalWait | null {
  let count = 0;
  let since = Number.POSITIVE_INFINITY;
  for (const a of approvals) {
    if (a.status !== "pending") continue;
    count += 1;
    if (a.atMillis < since) since = a.atMillis;
  }
  if (count === 0) return null;
  return { count, since, waited: Math.max(0, now - since) };
}

/**
 * The parked approvals the board's own poll says belong to one card (#883).
 *
 * The board reads `…/tasks`, whose card projection carries no approvals, and
 * opening every card to find out would be N reads per 4s poll — the cost
 * `TaskCard::output` is documented as existing to avoid. So the join happens
 * here instead, against the approvals feed the shell **already** polls for the
 * sidebar badge. No new request, no new wire field, and the board and the
 * Approvals page are reading one list rather than two that can disagree.
 *
 * `GET …/approvals` returns the parked queue and nothing else, so every entry is
 * pending by construction — unlike {@link pendingApprovalWait}, which filters a
 * task-detail projection that includes resolutions.
 *
 * **Only `{link: "task"}` matches, and that is the whole ownership rule here.**
 * `CycleHostImpl::park` stamps the link on every park path, so a card-dispatched
 * approval always carries it. The host's own `approval_owner` has a first rule
 * this cannot reach — the attempt-level `run_id`, which separates two *runs of
 * the same card* — but that distinction does not change the answer to "is this
 * card blocked", which is the only question asked here. `{link: "unlinked"}`
 * (a workflow delivery, an operator-chat turn, a scheduler tick) and an absent
 * link (a park predating #333) both belong to no card and are skipped rather
 * than guessed at: the host keeps a run-window heuristic for the ambiguous case
 * and the board has no window to apply it against.
 */
export function approvalsForTask(
  approvals: readonly ApprovalSummary[],
  taskId: string,
): ApprovalSummary[] {
  return approvals.filter((a) => a.task?.link === "task" && a.task.id === taskId);
}

/** Why a card is stopped — read by its board card and by its Resume button (#883). */
export interface TaskApprovalBlock {
  /** The still-parked approvals for this card, oldest park first. */
  approvals: ApprovalSummary[];
  /** How many there are — the number the card reports. */
  count: number;
  /** Epoch-millis the *oldest* of them parked — what the card measures from. */
  since: number;
}

/**
 * What is blocking one card right now — `null` when nothing is (#883).
 *
 * This is the derivation behind both halves of the fix, and it is one function
 * rather than two so they cannot disagree: the line that says *why* the card is
 * stopped and the `disabled` that stops Resume re-running it read the same
 * result. A card that said "blocked on 4 approvals" beside a Resume button that
 * dispatched anyway would be a worse surface than the silent one it replaces.
 *
 * Ordered oldest-first, and `since` is the oldest park — same reasoning as
 * {@link pendingApprovalWait}: it is how long the card has really been stopped,
 * and taking the newest would reset a clock that should be climbing every time
 * a second effect parks behind the first.
 *
 * No `waited` counterpart, unlike {@link pendingApprovalWait}: this block's only
 * reader renders the span through `timeAgo`, which takes the instant and clamps
 * its own negative — so a second copy of that arithmetic here would be a field
 * with no consumer to keep it honest.
 */
export function taskApprovalBlock(
  approvals: readonly ApprovalSummary[],
  taskId: string,
): TaskApprovalBlock | null {
  const mine = approvalsForTask(approvals, taskId).sort((a, b) => a.at_millis - b.at_millis);
  if (mine.length === 0) return null;
  return { approvals: mine, count: mine.length, since: mine[0].at_millis };
}

/**
 * One approval this card is (or was) held on, and what has become of it (#1891).
 *
 * The `verdict` half is why this exists rather than the card rendering
 * {@link TaskApprovalBlock.approvals} straight: a resolve's answer and the
 * feed's next poll are two moments, and for that gap a decided row must read as
 * decided rather than offering its buttons a second time.
 */
export interface TaskApprovalRow {
  approval: ApprovalSummary;
  /**
   * The verdict already witnessed for it — from this card, from the Approvals
   * page, or from another operator's console over the event stream — or `null`
   * while it is still waiting on somebody.
   */
  verdict: Verdict | null;
}

/**
 * Every approval one card is blocked on, decidable, in a stable order (#1891).
 *
 * The task-board twin of `runApprovals` in `@/views/workflows/run-approvals`,
 * and deliberately the same shape: the run drawer had exactly this problem
 * first (#1002) — it *told* an operator their run had parked and gave them
 * nowhere to act — and the board is that fix arriving at the surface an
 * operator actually works from. Two sources, both needed:
 *
 * - the **live queue**, which is the point. A card decided anywhere leaves it,
 *   so the board stops calling this card blocked without a reload;
 * - the shell's **witnessed decisions**, which keep the last-seen summary of an
 *   approval the queue has already dropped, so the operator's own decision
 *   settles in place instead of blinking out from under their cursor.
 *
 * Ordered oldest park first, then by id — stable across a refresh, because the
 * queue hands back a fresh array on every poll and a list that reordered under
 * a pointer mid-click would be its own bug. Same ordering
 * {@link taskApprovalBlock} uses, so the row the card measures its wait from is
 * the row it draws first.
 *
 * A witnessed decision is only folded back in when it belongs to **this** card,
 * by the same `{link: "task"}` rule {@link approvalsForTask} applies — a
 * decision taken on some other card's row is not this card's business, and
 * `decided` is a console-wide map covering every surface that resolves.
 */
export function taskApprovalRows(
  approvals: readonly ApprovalSummary[],
  decided: Readonly<Record<string, DecidedApproval>>,
  taskId: string,
): TaskApprovalRow[] {
  const byId = new Map<string, TaskApprovalRow>();
  for (const approval of approvalsForTask(approvals, taskId)) {
    byId.set(approval.id, { approval, verdict: decided[approval.id]?.verdict ?? null });
  }
  for (const [id, d] of Object.entries(decided)) {
    if (byId.has(id)) continue;
    if (d.approval.task?.link !== "task" || d.approval.task.id !== taskId) continue;
    byId.set(id, { approval: d.approval, verdict: d.verdict });
  }
  return [...byId.values()].sort(
    (a, b) =>
      a.approval.at_millis - b.approval.at_millis ||
      (a.approval.id < b.approval.id ? -1 : a.approval.id > b.approval.id ? 1 : 0),
  );
}

/** The subset still waiting on somebody — what the card is actually stopped behind. */
export function blockingTaskApprovals(rows: readonly TaskApprovalRow[]): TaskApprovalRow[] {
  return rows.filter((r) => r.verdict === null);
}

/** Stable empties, so an idle card's props do not churn on every board poll. */
const NO_VERDICTS: Record<string, Verdict> = {};
const NO_DECIDING: ReadonlyMap<string, Verdict> = new Map();

/**
 * The verdicts already witnessed for one card's rows (#1891).
 *
 * What `ApprovalRow` means by `decided`: the map it subtracts from its batch to
 * work out what an Approve still covers. Handing it the console-wide map would
 * be nearly the same thing and quietly wrong on the count — a decision on some
 * other card's approval would be subtracted from this card's total.
 */
export function taskApprovalVerdicts(rows: readonly TaskApprovalRow[]): Record<string, Verdict> {
  const verdicts: Record<string, Verdict> = {};
  for (const row of rows) if (row.verdict) verdicts[row.approval.id] = row.verdict;
  return Object.keys(verdicts).length === 0 ? NO_VERDICTS : verdicts;
}

/**
 * The decisions in flight on **this** card, as their own map (#1891).
 *
 * Narrowed for the reason #373 established and `RunResultPanel` repeats: the
 * shell's `deciding` map is console-wide, and `ApprovalRow` reads `size > 0` as
 * "I am busy". Passed whole, one operator click would grey out the buttons on
 * every blocked card on the board at once.
 *
 * Per *card* rather than per row, unlike the run drawer's version: the card
 * renders one batch with one Approve, so every id it covers is genuinely in
 * flight together and each should show it.
 */
export function decidingForTask(
  rows: readonly TaskApprovalRow[],
  deciding: ReadonlyMap<string, Verdict>,
): ReadonlyMap<string, Verdict> {
  if (deciding.size === 0) return NO_DECIDING;
  const mine = new Map<string, Verdict>();
  for (const row of rows) {
    const verdict = deciding.get(row.approval.id);
    if (verdict) mine.set(row.approval.id, verdict);
  }
  return mine.size === 0 ? NO_DECIDING : mine;
}
