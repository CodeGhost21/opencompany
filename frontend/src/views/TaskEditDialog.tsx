// The task edit form, extracted from the board (#184) so both the Kanban board
// and the Task Detail screen open the same dialog without a circular import.
// This is an *edit* form — title / note / column / priority / assignee plus a
// delete — unchanged from its original home on the board screen (retired in
// issue #1140; the board is the `tasks` ledger's columns now).

import { useEffect, useMemo, useState } from "react";
import { AlertCircle, Loader2, Trash2 } from "lucide-react";

import {
  deleteTask,
  patchTask,
  type InflightRun,
  type IrreversibleEffect,
  type PatchTask,
  type Task,
  type TaskDeliverable,
} from "@/api/tasks";
import type { OpenCompanyClient } from "@/api/client";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { computeTaskPatch, dispatchNeedsConfirm } from "@/lib/task-edit";
import { effectDone } from "@/lib/language";
import { timeOf } from "@/lib/timeline-format";
import { labelFor } from "@/lib/board-columns";
import { useBoardColumns } from "@/hooks/use-board-columns";
import { toast } from "sonner";
import { AssigneeSelect } from "./AssigneeSelect";

const PRIORITIES = ["low", "medium", "high"] as const;

/** The once-vs-workflow options, in review order (issue #580). */
const DELIVERABLES: { value: TaskDeliverable; label: string }[] = [
  { value: "once", label: "Do it once" },
  { value: "workflow", label: "Build me the workflow" },
];

/**
 * The columns where the deliverable can still be flipped (issue #580).
 *
 * Once a card leaves Pending (or the Planning stage) the choice is settled: the
 * builder pass fires on the drag into Working, so changing once-vs-workflow afterwards
 * cannot rebuild what already ran. The control is disabled there rather than
 * hidden — an honest "locked" reads better than a field that silently vanishes —
 * but this is a **UI-honesty** guard, not enforcement: the host is the authority
 * on whether a late patch is accepted, and the untouched-field diff below means
 * a save that does not touch the deliverable never sends it anyway.
 */
const DELIVERABLE_EDITABLE = new Set(["todo", "planning"]);

/**
 * Whether the once-vs-workflow choice is still open on this card.
 *
 * Matches on the **stage** and falls back to the phase, because since issue
 * #1512 `column` is `pending`/`working`/`done`: a pending card is editable, a
 * working one only while it is still `planning`, and matching on the column
 * alone would either lock every working card or unlock all four stages.
 */
function deliverableEditable(task: Task): boolean {
  const stage = task.stage ?? task.column;
  return stage === "pending" || DELIVERABLE_EDITABLE.has(stage);
}

/**
 * Edit a card (or delete it). Open when `task` is non-null; `onClose` fires on
 * dismiss, `onSaved`/`onDeleted` hand the reconciled row back to the caller so
 * the board or detail screen can update its own state.
 */
export function TaskEditDialog({
  task,
  onClose,
  onSaved,
  onDeleted,
  client,
  company,
  inflight,
  irreversible,
  historyIncomplete,
}: {
  task: Task | null;
  onClose: () => void;
  onSaved: (t: Task) => void;
  onDeleted: (id: string) => void;
  client: OpenCompanyClient;
  company: string | null;
  /**
   * The run currently holding this card, if any.
   *
   * Delete is refused with a `409` while one exists (`server::ops::tasks`,
   * issue #984), so the dialog must not offer it. Absent means the caller has
   * no in-flight read; the button stays enabled and the host refuses, which is
   * the behaviour that existed before this prop.
   */
  inflight?: InflightRun | null;
  /**
   * What this card already did that cannot be undone (#351), and whether the
   * journal admits it cannot describe its own history.
   *
   * Both feed the same gate `RetryButton` uses on the Task Detail screen,
   * because saving `column: "working"` here is the identical write. **Absent
   * is not "clean"** — a caller that has not wired these gets the confirmation
   * unconditionally on a dispatch, because a dialog that cannot say what
   * already happened must not pass that gap off as an all-clear. See
   * `dispatchNeedsConfirm`.
   */
  irreversible?: IrreversibleEffect[];
  historyIncomplete?: boolean;
}) {
  // From the `tasks` ledger, so this select can never offer a column the host's
  // write boundary would refuse — which is what a second, local list allowed.
  const columns = useBoardColumns(client, company);
  const [draft, setDraft] = useState<PatchTask>({});
  /**
   * The card exactly as it read when this draft was seeded.
   *
   * The patch below is diffed against **this**, not against the live `task`
   * prop, so "the fields the operator touched" keeps meaning that even while
   * the host changes the card underneath an open dialog. See the effect.
   */
  const [seed, setSeed] = useState<Task | null>(null);
  const [busy, setBusy] = useState(false);

  // Seed the edit draft when the dialog *opens*, and when it opens onto a
  // different card — deliberately **not** whenever the `task` object changes
  // identity.
  //
  // The Task Detail screen refetches this card every 4s (`POLL_MS` there) and
  // hands down a brand-new `detail.task` object on every tick, carrying the
  // same values. Keyed on `task`, this effect therefore re-ran four times a
  // minute and overwrote whatever had been typed or picked: measured in a
  // browser, a Column select set to Working reverted to Pending ~2.3s later and
  // never came back. Title, Note, Priority and Assignee went the same way.
  //
  // That also silently disarmed this dialog's dispatch confirmation. After the
  // reset `computeTaskPatch` returns `{}`, so `confirmDispatch` is false *and*
  // Save writes nothing — no confirmation, no request, no toast. The #351 gate
  // was only reachable by picking and saving inside the same ~150ms.
  //
  // Which side wins a conflict is the actual decision here, and it is **the
  // operator's open draft**. A half-written note exists nowhere else — nothing
  // on the screen holds a copy, which is why Cancel asks before discarding it —
  // so a second operator's edit must not retype it out from under the first.
  // The cost, pinned by a test: a server-side change to this same card is not
  // reflected in the open dialog. It is visible on the screen behind, and the
  // dialog picks it up the next time it opens.
  //
  // `seed` is what keeps that cost bounded to the fields in play. Diffing a
  // frozen draft against a *live* `task` would quietly widen the write: a poll
  // that moved the column to `done` would make an unrelated note edit also send
  // `column: "pending"`, and a roster change would resubmit the stale assignee
  // that issue #263's diff exists to keep out of the request. Diffing against
  // the seed sends only what the operator actually changed, last-write-wins on
  // those fields alone.
  useEffect(() => {
    if (!task) return;
    setSeed(task);
    setDraft({
      title: task.title,
      note: task.note ?? "",
      column: task.column,
      priority: task.priority,
      assignee: task.assignee,
      // Absent means `"once"` on the wire, so the control is seeded with the
      // normalized value and a card with no stored deliverable edits as the
      // one-off it is (issue #580).
      deliverable: task.deliverable ?? "once",
    });
    // Keyed on the card's identity rather than the object, for the reason
    // above. `task` is read only to seed from, on the render where the identity
    // changed; closing the dialog nulls it, so reopening the same card re-seeds.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task?.id]);

  // The diff the Save button would send, computed here rather than at click
  // time so the footer can decide what kind of button it is: a `column` move
  // into Working dispatches, and an untouched form has nothing to discard.
  //
  // Against `seed` — the card as it read when the dialog opened — rather than
  // the live `task`, so this keeps meaning "what the operator changed" and
  // never reports the poll's own updates as unsaved edits. `seed` is null for
  // the one render before the effect above runs, and an empty patch is the
  // honest answer there: nothing has been typed yet.
  const patch = useMemo(() => (seed ? computeTaskPatch(draft, seed) : {}), [draft, seed]);

  if (!task) return null;

  const dirty = Object.keys(patch).length > 0;
  // Issue #351, reached from the *other* direction. The Column select can emit
  // exactly `{ column: "working" }` — byte for byte the write the Task Detail
  // Retry button makes — so the confirmation that button carries has to be here
  // too, or the protection is one dropdown away from being walked around on the
  // screen it was built for. The condition is `RetryButton`'s, not "always", so
  // a card with nothing at stake still saves in one click.
  const confirmDispatch = dispatchNeedsConfirm(patch, { irreversible, historyIncomplete });
  const named = irreversible?.length ?? 0;
  // Wording matched to the host's own refusal (`delete_task`) rather than
  // invented here: it names cancelling first, and it says why deleting now
  // would not actually remove the card. **Cancel, not Stop** — a paused run is
  // still in the steer registry until its turn actually stops, so the `409`
  // stands, and pointing at Stop would send the operator to a button that does
  // not clear this.
  const deleteBlocked = inflight
    ? "This task is running — cancel the run first, watch it stop, then delete it. Deleting it now wouldn’t remove it: the turn writes the card back when it settles."
    : undefined;

  async function save() {
    if (!task) return;
    // Only the fields the operator actually touched (issue #263's roster-safety
    // diff, extended with #580's deliverable) — see `computeTaskPatch`. Read
    // off the memo above so the patch that was *gated* is the patch that is
    // sent; recomputing here would let the two drift.
    if (Object.keys(patch).length === 0) {
      // Nothing to write. Saying so beats a round-trip that reports "Saved."
      // for an edit that never happened.
      toast.success("No changes to save.");
      onSaved(task);
      return;
    }
    setBusy(true);
    try {
      const saved = await patchTask(client, company, task.id, patch);
      onSaved(saved);
      toast.success("Saved.");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "could not save");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!task) return;
    setBusy(true);
    try {
      await deleteTask(client, company, task.id);
      onDeleted(task.id);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "could not delete");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={!!task} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Edit task</DialogTitle>
          <DialogDescription>
            Edit the card, or drop it into “In progress” on the board to dispatch it.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="task-title">Title</Label>
            <Input
              id="task-title"
              value={draft.title ?? ""}
              onChange={(e) => setDraft((d) => ({ ...d, title: e.target.value }))}
            />
          </div>

          <div className="grid gap-1.5">
            <Label htmlFor="task-note">Note / result</Label>
            <Textarea
              id="task-note"
              rows={8}
              className="font-mono text-xs"
              value={draft.note ?? ""}
              onChange={(e) => setDraft((d) => ({ ...d, note: e.target.value }))}
            />
          </div>

          <div className="grid grid-cols-3 gap-3">
            <div className="grid gap-1.5">
              <Label htmlFor="task-column">Column</Label>
              {/* Fall back to the card's own value rather than `undefined`.
                  `draft` starts empty and is seeded a tick later by the effect
                  above, so a bare `draft.column` hands Base UI `undefined` on
                  the first render — which latches the select as *uncontrolled*
                  and makes it ignore the seeded value, leaving the trigger
                  blank for the whole life of the dialog. */}
              <Select
                value={draft.column ?? task.column}
                onValueChange={(v) => setDraft((d) => ({ ...d, column: v ?? undefined }))}
              >
                <SelectTrigger id="task-column">
                  {/* The trigger renders the raw value unless told otherwise,
                      and a column's id is not its label (`in_progress` vs "In
                      progress"). */}
                  <SelectValue>
                    {(selected) =>
                      selected ? labelFor(columns, String(selected)) : ""
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {columns.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="task-priority">Priority</Label>
              {/* Same seeding hazard as Column above. A priority is its own
                  label, so only the casing the items carry needs restating. */}
              <Select
                value={draft.priority ?? task.priority}
                onValueChange={(v) => setDraft((d) => ({ ...d, priority: v ?? undefined }))}
              >
                <SelectTrigger id="task-priority">
                  <SelectValue className="capitalize" />
                </SelectTrigger>
                <SelectContent>
                  {PRIORITIES.map((p) => (
                    <SelectItem key={p} value={p} className="capitalize">
                      {p}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            {/* `min-w-0`: a grid item's automatic minimum size is its content's
                min-content width, so the assignee's long ids would otherwise
                widen this track and squeeze Column and Priority away. */}
            <div className="grid min-w-0 gap-1.5">
              <Label htmlFor="task-assignee">Assignee</Label>
              {/* Issue #263: picked from the roster, not typed. An assignee the
                  roster no longer carries still renders — flagged — so a save
                  that does not touch it can never quietly rewrite it.

                  Disabled while a save is in flight, like its twin in the
                  detail screen's reassign row. A picker that still moves during
                  the write invites a second choice that the in-flight PATCH
                  will not carry — so the row reads as the new assignee while
                  the host stored the old one. */}
              <AssigneeSelect
                id="task-assignee"
                client={client}
                company={company}
                value={draft.assignee ?? ""}
                onChange={(next) => setDraft((d) => ({ ...d, assignee: next }))}
                disabled={busy}
              />
            </div>
          </div>

          <div className="grid gap-1.5">
            <Label htmlFor="task-deliverable">Deliverable</Label>
            <Select
              value={draft.deliverable ?? task.deliverable ?? "once"}
              onValueChange={(v) =>
                setDraft((d) => ({ ...d, deliverable: (v as TaskDeliverable) ?? undefined }))
              }
              disabled={!deliverableEditable(task)}
            >
              <SelectTrigger id="task-deliverable" data-testid="edit-deliverable">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {DELIVERABLES.map((d) => (
                  <SelectItem key={d.value} value={d.value}>
                    {d.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {!deliverableEditable(task) && (
              <p className="text-2xs text-muted-foreground">
                Locked once work starts — the workflow is built when a card enters In progress, so
                this can only be changed while it&apos;s still in To-do or Planning.
              </p>
            )}
          </div>
        </div>

        <DialogFooter className="justify-between sm:justify-between">
          <AlertDialog>
            {/* Issue #984: `delete_task` answers `409` for a card a run is
                holding, deliberately — the turn writes the card back when it
                settles, so deleting underneath it removes nothing and leaves a
                card no surface names. Offering the button anyway makes the
                operator discover that as a failed click; the control bar three
                lines away already branches on the same `inflight`. */}
            <AlertDialogTrigger
              disabled={!!inflight}
              render={
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy || !!inflight}
                  title={deleteBlocked}
                >
                  <Trash2 className="mr-1.5 size-4" />
                  Delete
                </Button>
              }
            />
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Delete “{task.title}”?</AlertDialogTitle>
                <AlertDialogDescription>
                  This permanently removes the task and can’t be undone.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Keep task</AlertDialogCancel>
                <AlertDialogAction
                  onClick={() => void remove()}
                  className="bg-destructive text-white hover:bg-destructive/90"
                >
                  Delete task
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
          <div className="flex gap-2">
            {/* Three-way, matching the Artifacts editor's Cancel: a dirty form
                asks before it throws the edit away, a clean one dismisses
                straight off, and both are disabled mid-save. A note typed into
                the textarea above is not recoverable once this closes, and
                nothing else on the screen holds a copy of it. */}
            {dirty ? (
              <AlertDialog>
                <AlertDialogTrigger
                  render={
                    <Button variant="outline" disabled={busy}>
                      Cancel
                    </Button>
                  }
                />
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Discard this edit?</AlertDialogTitle>
                    <AlertDialogDescription>
                      Nothing has been saved yet, so your changes will be lost.
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>Keep editing</AlertDialogCancel>
                    <AlertDialogAction
                      className="bg-destructive text-white hover:bg-destructive/90"
                      onClick={onClose}
                    >
                      Discard
                    </AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            ) : (
              <Button variant="outline" onClick={onClose} disabled={busy}>
                Cancel
              </Button>
            )}
            {confirmDispatch ? (
              <AlertDialog>
                <AlertDialogTrigger
                  render={
                    <Button disabled={busy} data-testid="edit-save">
                      {busy && <Loader2 className="mr-1.5 size-4 animate-spin" />}
                      Save
                    </Button>
                  }
                />
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Run “{task.title}” again?</AlertDialogTitle>
                    <AlertDialogDescription>
                      {/* Leads with the mechanism, because this one is not
                          obvious: the operator moved a dropdown, and nothing
                          about a Column select says "dispatch". */}
                      {`Moving this card into Working runs it again — the same as Retry. ${
                        named === 0
                          ? "Running it again may repeat whatever the last attempt did."
                          : named === 1
                            ? "This task already did something that cannot be undone, and running it again may do it a second time."
                            : `This task already did ${named} things that cannot be undone, and running it again may do them a second time.`
                      }`}
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <div className="space-y-2 text-left">
                    {named > 0 && (
                      <ul className="space-y-1.5 rounded-lg border bg-muted/40 p-3 text-xs">
                        {irreversible?.map((e, i) => (
                          // Two effects of the same kind can land in the same
                          // millisecond, so the index carries the uniqueness
                          // the pair cannot.
                          <li key={`${e.kind}-${e.atMillis}-${i}`} className="flex items-start gap-2">
                            <AlertCircle
                              className="mt-px size-3.5 shrink-0 text-status-blocked-text"
                              aria-hidden
                            />
                            <span className="min-w-0 flex-1">{effectDone(e.kind, e.amountUsd)}</span>
                            <span className="shrink-0 tabular-nums text-muted-foreground">
                              {timeOf(e.atMillis)}
                            </span>
                          </li>
                        ))}
                      </ul>
                    )}
                    {historyIncomplete && (
                      // The list is short, not wrong. Say which, rather than
                      // let a truncated list read as the whole story.
                      <p className="text-xs text-muted-foreground">
                        Some of this company’s earlier activity was recorded before it kept a
                        description, so
                        {named > 0 ? " this list may be incomplete." : " nothing here can be listed."}
                      </p>
                    )}
                    {named > 0 && (
                      <p className="text-2xs text-muted-foreground">
                        Each is recorded the moment it was committed, so one that was interrupted
                        still appears — nothing is ever retried on its own.
                      </p>
                    )}
                  </div>
                  <AlertDialogFooter>
                    <AlertDialogCancel>Leave it alone</AlertDialogCancel>
                    <AlertDialogAction onClick={() => void save()}>Save anyway</AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            ) : (
              <Button onClick={() => void save()} disabled={busy} data-testid="edit-save">
                {busy && <Loader2 className="mr-1.5 size-4 animate-spin" />}
                Save
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
