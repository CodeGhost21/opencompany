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
 * back empty. That holds **per field** — one half wired and the other absent is
 * still "cannot say", not a half-price all-clear. See {@link readEffectHistory},
 * which turns this into the three states rather than leaving it to falsiness.
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
 * What the journal is able to claim about this card's irreversible effects.
 *
 * Three states, spelled out, because **two of them are falsy and only one of
 * those two may skip the confirmation**. Leaning on falsiness is what let a
 * half-read history — one field wired, the other never passed — decide "clean"
 * and wave a re-dispatch through on a card that may already have sent a
 * payment.
 *
 * * `"dirty"`  — read, and it recorded something that cannot be taken back.
 * * `"clean"`  — **both** halves read, and both came back empty.
 * * `"unknown"` — a half was not read. Not a claim about the card at all.
 *
 * `"unknown"` confirms. It has to: the whole point of the gate is that a caller
 * which cannot describe what this card already did must not assert it did
 * nothing, and a guard that failed open there would be dead in a way
 * indistinguishable from working.
 */
export type EffectHistoryVerdict = "clean" | "dirty" | "unknown";

/**
 * Reads {@link EffectHistory} into the three states above.
 *
 * A field that *was* read and says something happened settles it as `"dirty"`
 * whichever way the other field went — an unread second field cannot make a
 * recorded payment un-happen. Only when nothing is dirty does the absence of
 * either half matter, and then it is `"unknown"` rather than `"clean"`.
 */
export function readEffectHistory(history: EffectHistory): EffectHistoryVerdict {
  if ((history.irreversible?.length ?? 0) > 0) return "dirty";
  if (history.historyIncomplete === true) return "dirty";
  // Nothing known-dirty. "Clean" is a positive claim, so it needs both reads to
  // have actually happened; an absent half is "cannot say", never an all-clear.
  const effectsRead = history.irreversible !== undefined;
  const completenessRead = history.historyIncomplete !== undefined;
  return effectsRead && completenessRead ? "clean" : "unknown";
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
  return readEffectHistory(history) !== "clean";
}
