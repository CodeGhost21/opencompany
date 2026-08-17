// Sample Kanban data for the Tasks board. Client-side illustrative data — the
// console has no live task API yet, so the board is a local working surface.

export type TaskPriority = "low" | "medium" | "high";

export interface TaskCard {
  id: string;
  title: string;
  note?: string;
  column: string;
  priority: TaskPriority;
  /** Which desk owns it — matches the conversation thread tones. */
  assignee: { name: string; tone: string };
}

/**
 * The board's columns no longer live here.
 *
 * They are the host's, declared once in `src/ledger/board.rs`, published as the
 * `tasks` ledger's statuses — each with the label a person reads — and fetched
 * by `lib/board-columns.ts`. The copy that used to sit here was a
 * hand-maintained mirror whose own comment admitted the drift it could not
 * prevent: a column added on one side and not the other kept every test green,
 * and its cards then either vanished from the board or were refused by the
 * host's write boundary. Nothing in this file should grow a replacement.
 */

/**
 * The one column that offers the "+" add-task button (issue #206).
 *
 * New work enters the board in exactly one place. Offering `+` on every column
 * — as the board used to — let an operator create a card straight into
 * `in_progress`, `in_review`, or `done`, which either skips the dispatch edge
 * or fabricates a terminal state for work that never ran.
 */
export const ADD_TASK_COLUMN = "todo";

const STRATEGY = { name: "Strategy desk", tone: "sky" };
const CREATIVE = { name: "Creative studio", tone: "violet" };
const FRONT = { name: "Front desk", tone: "amber" };

let n = 0;
const id = () => `task-${n++}`;

export function sampleTasks(): TaskCard[] {
  return [
    { id: id(), title: "Q2 campaign brief", note: "Turn the client brief into an angle", column: "todo", priority: "high", assignee: STRATEGY },
    { id: id(), title: "Competitor scan", note: "Pull three rival launches", column: "todo", priority: "low", assignee: STRATEGY },
    { id: id(), title: "Newsletter refresh", note: "New template + segments", column: "planning", priority: "medium", assignee: FRONT },
    { id: id(), title: "Spring launch taglines", note: "Draft three options", column: "in_progress", priority: "high", assignee: CREATIVE },
    { id: id(), title: "Landing hero image", note: "Generate + retouch", column: "in_progress", priority: "medium", assignee: CREATIVE },
    { id: id(), title: "Invoice March retainer", column: "in_review", priority: "medium", assignee: FRONT },
    { id: id(), title: "Brand voice guide", note: "One-pager for the team", column: "done", priority: "low", assignee: STRATEGY },
    { id: id(), title: "Welcome email flow", column: "done", priority: "medium", assignee: FRONT },
  ];
}

/**
 * Priority badges.
 *
 * These deliberately *do* use the status hues, unlike the category and kind
 * palettes elsewhere. Priority and status share one axis — how much this
 * wants your attention — so red-for-high and amber-for-medium reinforce the
 * vocabulary rather than competing with it, and `low` stays neutral for the
 * same reason `idle` does: nothing is being asked of anyone.
 */
export const PRIORITY_STYLES: Record<TaskPriority, string> = {
  high: "border-status-failed/30 bg-status-failed-soft text-status-failed-text",
  medium: "border-status-blocked/30 bg-status-blocked-soft text-status-blocked-text",
  low: "border-border bg-muted text-muted-foreground",
};
