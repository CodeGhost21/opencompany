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

import type { PatchTask, Task } from "@/api/tasks";

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
