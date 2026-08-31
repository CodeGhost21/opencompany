// The edit-dialog's field diff, pulled out of the component so it is a pure,
// unit-testable function (issue #580 adds the deliverable arm to it).
//
// A `PatchTask` is applied field-by-field on the host, and every field it
// *receives* is re-validated — `assignee` included, which re-runs
// `assignee::resolve` against the current roster. Sending the whole seeded draft
// therefore made a card uneditable the moment its stored assignee left the
// roster: renaming the title resubmitted the stale assignee, which came back
// `Unknown`, and the save failed with a `400` about a field the operator never
// touched. Diffing means an untouched field is never re-validated.

import type { IrreversibleEffect, PatchTask, Task } from "@/api/tasks";
import { BOARD_WORKING } from "@/lib/board-columns";

/**
 * The fields the operator actually changed, and only those.
 *
 * `deliverable` is normalized on both sides — absent means `"once"` (issue
 * #580) — so an untouched control emits no patch and a card with no stored value
 * does not diff `"once"` against `undefined` and patch a field nobody touched;
 * flipping the choice sends exactly `{ deliverable }`.
 */
export function computeTaskPatch(draft: PatchTask, current: Task): PatchTask {
  const patch: PatchTask = {};
  if ((draft.title ?? "") !== current.title) patch.title = draft.title ?? "";
  if ((draft.note ?? "") !== (current.note ?? "")) patch.note = draft.note ?? "";
  if (draft.column !== undefined && draft.column !== current.column) patch.column = draft.column;
  if (draft.priority !== undefined && draft.priority !== current.priority) {
    patch.priority = draft.priority;
  }
  if ((draft.assignee ?? "") !== current.assignee) patch.assignee = draft.assignee ?? "";
  if ((draft.deliverable ?? "once") !== (current.deliverable ?? "once")) {
    patch.deliverable = draft.deliverable ?? "once";
  }
  return patch;
}

/**
 * What the host's journal recorded against this card, as a caller hands it in.
 *
 * Both fields are optional, and "absent" is a **third** state that must not
 * collapse into "clean": a caller that has not wired the read knows nothing
 * about this card's effects, which is not the same claim as a read that came
 * back empty. `dispatchNeedsConfirm` treats absent the way the host treats
 * `historyIncomplete` — as "cannot say", which confirms.
 */
export interface EffectHistory {
  /** The effects the journal recorded as executed, or absent if not read. */
  irreversible?: IrreversibleEffect[];
  /** Whether the journal holds executed history it cannot describe (#351). */
  historyIncomplete?: boolean;
}

/**
 * Whether saving this patch re-enters the run.
 *
 * `{ column: "working" }` is the *identical* write the Task Detail screen's
 * Retry button makes (`patchColumn` there): the host resolves the `working`
 * phase to `in_progress`, which dispatches. The edit dialog's Column select can
 * emit it too, which is how a `<Select>` plus Save came to be an unguarded
 * second route to the thing issue #351 wrapped in a confirmation.
 *
 * Only `working` counts. `pending` and `done` are parks, and a stage word the
 * dialog never offers is not something to guess about.
 */
export function patchDispatchesRun(patch: PatchTask): boolean {
  return patch.column === BOARD_WORKING;
}

/**
 * Whether saving this patch must stop and say what already happened (#351).
 *
 * The condition is deliberately the same one `RetryButton` uses — confirm when
 * the journal recorded an irreversible effect, or when it admits it cannot
 * describe its own history — rather than confirming on every dispatch. A dialog
 * that asked on a card where nothing is at stake trains the operator to click
 * through it, which is how the confirmation stops working on the card where it
 * matters.
 */
export function dispatchNeedsConfirm(patch: PatchTask, history: EffectHistory): boolean {
  if (!patchDispatchesRun(patch)) return false;
  // Neither half read: the dialog cannot say this card is clean, so it says so
  // by confirming. This is what keeps the guard live for a caller that has not
  // wired the journal read yet, instead of silently passing a gap off as an
  // all-clear.
  if (history.irreversible === undefined && history.historyIncomplete === undefined) return true;
  return (history.irreversible?.length ?? 0) > 0 || history.historyIncomplete === true;
}
