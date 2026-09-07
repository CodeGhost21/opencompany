import { useEffect, useRef, useState } from "react";
import { Mail } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { getInferenceStatus, type CognitionPath } from "@/api/inference";
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
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { designTeammate, refusalNotice, type DraftRefusal } from "@/api/agent-copilot";
import {
  addTeammateSurface,
  carriedDescribe,
  describeBlocked as blockedReason,
  designedTeammateFields,
  heldFields,
  type DesignedTeammateFields,
} from "@/lib/team-add-surface";
import { DescribeTeammate } from "@/views/team/DescribeTeammate";

export interface NewMemberFields {
  name: string;
  role: string;
  description: string;
  /**
   * The standing instructions this teammate is born with (issue #1989).
   *
   * Set only by the reduced dialog, from the host's design pass. The full form
   * does not collect one, and a teammate created without it keeps the behaviour
   * it always had: no persona override, the blueprint's own wording in force.
   */
  instructions?: string;
  inbox?: boolean;
  /**
   * Land on the new teammate's detail page with its edit form open, rather than
   * staying where the dialog was opened from (issue #1989).
   *
   * Set only by the reduced dialog, and it is that dialog's second half: it
   * collects a name and a sentence, so the description, the persona, the budget
   * and the inbox are all still to be filled in — on the page this flag opens,
   * beside the copilot that drafts two of them. A caller with nowhere to
   * navigate to may ignore it; a caller whose write fell back to a local-only
   * row has no id to navigate to and must.
   */
  landOnProfile?: boolean;
}

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /**
   * Writes the teammate, answering whether the write landed.
   *
   * Awaited, and the dialog is cleared only on `true`. It used to be `void`
   * and called fire-and-forget: `onAdd(...)` then `reset()` on the next line,
   * while `POST {scope}/team` was still in flight. A 5xx or a dropped
   * connection then left the dialog open, blank and enabled, having thrown
   * away the operator's name, their sentence, and a design the company had
   * already been charged a model call for. `false` keeps all three so Create
   * can simply be pressed again.
   */
  onAdd: (fields: NewMemberFields) => boolean | Promise<boolean>;
  /**
   * For the cognition read that decides which dialog renders (issue #1989).
   * This dialog writes nothing itself — `onAdd` is still what creates.
   */
  client: OpenCompanyClient;
  company: string | null;
}

/**
 * Add teammate. Reached from the chat pane's member list and from the org
 * chart's desk cards.
 *
 * Two shapes since issue #1989, told apart by `addTeammateSurface`:
 *
 * - **Reduced** — a name and one box. Create asks the host to design the
 *   teammate from that sentence — role, mandate and persona in one pass — then
 *   writes it and lands the operator on its detail page with all three in
 *   editable boxes. Nothing is written if the design does not come back whole.
 * - **Full** — this dialog's original Name / Role / What they do / inbox form,
 *   byte for byte, for a company whose copilot cannot draft. Hidden, never
 *   deleted: a company on the offline brain would otherwise be locked out of
 *   ever writing a description, since nothing downstream could draft one for it.
 */
export function AddMemberDialog({ open, onOpenChange, onAdd, client, company }: Props) {
  const [name, setName] = useState("");
  const [role, setRole] = useState("");
  const [description, setDescription] = useState("");
  const [inbox, setInbox] = useState(false);
  /** Everything the reduced dialog collects. */
  const [described, setDescribed] = useState({ name: "", description: "" });
  /**
   * Whether a Create asked the host to design this teammate and got nothing
   * back, which retires the reduced dialog for this open rather than writing a
   * teammate the model could not finish.
   */
  const [designRefused, setDesignRefused] = useState<DraftRefusal | "unknown" | null>(null);
  /** A design pass is in flight; the box is held and the button says so. */
  const [designing, setDesigning] = useState(false);
  /** The write is in flight. Create is held so one press cannot become two. */
  const [creating, setCreating] = useState(false);
  /**
   * A design the host already returned for exactly what is in the box now, so
   * a write that failed after a successful design is retried without paying
   * for a second model call. `heldFields` drops it the moment either field is
   * edited — a design belongs to the sentence it was written from.
   */
  const heldDesign = useRef<{
    name: string;
    description: string;
    fields: DesignedTeammateFields;
  } | null>(null);
  /**
   * Which design request the operator is still waiting for.
   *
   * Bumped by every close and every reset, so an answer for a dialog that has
   * been shut — or reopened onto a different teammate — is dropped rather than
   * creating one nobody asked for. The window is real: a design is a model call
   * and takes seconds.
   */
  const attempt = useRef(0);
  /**
   * The design request currently in flight, so shutting the dialog can tear it
   * down rather than only ignoring its answer.
   *
   * `attempt` alone was half the job: it makes a late answer harmless and does
   * nothing about the cost. A design pass runs a model for up to ninety seconds
   * and is metered against the company's plan, and `close()` is reachable from
   * Cancel, Escape, the backdrop and the header's close icon. See
   * `designTeammate` for why this route is the one copilot call that takes a
   * signal.
   */
  const designAbort = useRef<AbortController | null>(null);
  /**
   * The cognition path this company booted onto, read while the dialog is open.
   * `null` until the check settles and on a host without the route, which the
   * surface function reads as "can draft" — see `addTeammateSurface` for why
   * that is the right way to be wrong.
   */
  const [cognition, setCognition] = useState<CognitionPath | null>(null);
  /**
   * Whether the host says a design pass can run for this company. `null` until
   * the check settles, and on a host that does not report the capability —
   * both read as "unknown", which the surface function treats as "offer it".
   */
  const [designsProfiles, setDesignsProfiles] = useState<boolean | null>(null);

  /**
   * Everything the dialog holds belongs to one company. Dropped when the scope
   * changes under it.
   *
   * The console can switch hosts with this dialog mounted, and its state does
   * not follow: `cognition` and `designsProfiles` are the previous company's
   * answers, and they decide which of the two forms is on screen. So a company
   * that could not design would keep showing the full form until the new read
   * landed, and the flip to the reduced one would take whatever had been typed
   * into it with it — the carry runs the other way, and could not sensibly run
   * this way in any case.
   *
   * Cleared rather than carried, deliberately. A half-written teammate is
   * addressed to the company it was written for; carrying it across a host
   * switch would offer to create it somewhere the operator did not describe it,
   * which is a worse answer than an empty box.
   */
  useEffect(() => {
    setCognition(null);
    setDesignsProfiles(null);
    reset();
    // `reset` is stable enough for this: it only closes over setters and refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, company]);

  useEffect(() => {
    if (!open) return;
    let live = true;
    (async () => {
      try {
        const status = await getInferenceStatus(client, company);
        if (live) {
          setCognition(status.cognition);
          setDesignsProfiles(status.designsProfiles ?? null);
        }
      } catch {
        if (live) {
          setCognition(null);
          setDesignsProfiles(null);
        }
      }
    })();
    return () => {
      live = false;
    };
  }, [open, client, company]);

  const describing =
    addTeammateSurface({
      cognition,
      designsProfiles,
      designRefused: designRefused !== null,
    }) === "describe";
  /** Why the reduced dialog's Create is dead, or `null` when it is not. */
  const describeBlocked = blockedReason(described);
  /**
   * Whether shutting the dialog right now would actually stop what it started.
   *
   * The rule the four exits all obey: **offer the way out only when taking it
   * does something.** A write in flight is not cancellable at all — closing
   * during it leaves the create running, and the parent still reports it and
   * navigates, while a reopen-and-submit in the gap creates a second teammate.
   * A design in flight is cancellable only where the transport can actually
   * abort: the desktop app's `ProxyTransport` cannot cancel an in-flight Tauri
   * `invoke` (`transport/types.ts`), so the pass runs to completion in the
   * app's core and is metered either way. Held open, the dialog keeps saying
   * what it is doing — an honest wait beats a cancel that only looks like one.
   */
  const heldOpen = creating || (designing && !client.cancelsInFlightRequests);

  /**
   * Moves the reduced dialog's two values into the full form when the surface
   * flips under the operator.
   *
   * The flip nobody accounted for is a *late* cognition read: `/inference` is
   * slow, `cognition` is `null`, the reduced dialog renders, the operator
   * starts typing, and the answer comes back `echo` and swaps the form. The two
   * shapes hold separate state, so without this the name and the sentence are
   * simply gone. `carriedDescribe` refuses to overwrite anything already in the
   * form, which makes this idempotent and harmless on the hand-over path, where
   * `handOver` has already carried the same two values.
   */
  useEffect(() => {
    if (describing) return;
    const carried = carriedDescribe(described, { name, description });
    if (!carried) return;
    setName(carried.name);
    setDescription(carried.description);
  }, [describing, described, name, description]);

  /**
   * A design in flight when this unmounts is one nobody can be shown, so it is
   * torn down here as well as in `reset` — leaving the chat closes the dialog
   * without going through either.
   */
  useEffect(() => {
    return () => {
      designAbort.current?.abort();
      designAbort.current = null;
    };
  }, []);

  function reset() {
    setName("");
    setRole("");
    setDescription("");
    setInbox(false);
    setDescribed({ name: "", description: "" });
    // The hand-over lasts for one open: the next add starts reduced again,
    // because the sentence the host could not design from is gone with it.
    setDesignRefused(null);
    setDesigning(false);
    setCreating(false);
    heldDesign.current = null;
    // Abandons any design still in flight, so its answer cannot create a
    // teammate into a dialog that has been reset under it — and tears the
    // request down, so the host stops paying for one nobody is waiting for.
    attempt.current += 1;
    designAbort.current?.abort();
    designAbort.current = null;
  }

  /**
   * Shut the dialog and clear it, whichever control did the shutting.
   *
   * One function because the reset MUST NOT hang off Radix's `onOpenChange`
   * alone. Cancel used to call the raw `onOpenChange(false)` prop, which closes
   * the dialog without going through the wrapper that resets — so Escape and
   * the overlay cleared the form and Cancel did not. That is invisible until
   * the dialog has a second shape: one hand-over to the full form, cancelled
   * rather than escaped, left the hand-over state (`designRefused`) set and so
   * retired the reduced dialog for the rest of the page's life, still carrying
   * the name and the sentence from the abandoned attempt. Verified in a
   * browser: Cancel then reopen showed six fields and the old text; Escape
   * then reopen showed two empty ones. The module's own promise is "the hand-over lasts for one open",
   * and only this makes it true.
   */
  function close() {
    // Every exit lands here — Cancel, Escape, the backdrop, the header's close
    // icon — so the one place that can refuse them all is this one.
    if (heldOpen) return;
    onOpenChange(false);
    reset();
  }

  /**
   * Hands the operator the full form, carrying what they typed, because the
   * host could not design this teammate.
   *
   * The reduced dialog never dead-ends. Nothing has been written at this point
   * and nothing will be: a teammate is created only from a design the host
   * returned whole.
   */
  function handOver(reason: DraftRefusal | "unknown") {
    setName(described.name.trim());
    setDescription(described.description.trim());
    setDesignRefused(reason);
    setDesigning(false);
  }

  async function submit() {
    if (creating) return;
    if (describing) {
      if (blockedReason(described)) return;
      const mine = attempt.current;
      // A design already paid for, for exactly this name and sentence. Only a
      // retry after a failed write can find one here.
      let fields = heldFields(heldDesign.current, described);
      if (!fields) {
        const controller = new AbortController();
        designAbort.current = controller;
        setDesigning(true);
        let design;
        try {
          design = await designTeammate(client, company, described, controller.signal);
        } catch {
          // A transport, auth or not-found failure — not one of the four
          // design refusals, which arrive as a 200. Treated the same way by
          // the dialog because the operator's move is the same: take the form.
          if (attempt.current === mine) handOver("unknown");
          return;
        }
        if (attempt.current !== mine) return;
        fields = designedTeammateFields(described, design);
        if (!fields) {
          handOver(design.reason ?? "unknown");
          return;
        }
        heldDesign.current = {
          name: described.name.trim(),
          description: described.description.trim(),
          fields,
        };
      }
      setDesigning(false);
      // `finally`, because `creating` is what holds the dialog shut: a parent
      // that rejected rather than answering `false` would otherwise trap the
      // operator in a dialog with every exit disabled.
      setCreating(true);
      let landed: boolean;
      try {
        landed = await onAdd({ ...fields, landOnProfile: true });
      } catch {
        // A parent that rejected rather than answering. Read as "did not
        // land", which keeps the sentence and the design for a retry — and
        // caught rather than left to escape, because `submit` is invoked as
        // `void submit()` and an escaping rejection is an unhandled one.
        landed = false;
      } finally {
        if (attempt.current === mine) setCreating(false);
      }
      if (attempt.current !== mine) return;
      // Only on a write that landed. A failure keeps the box, the name and the
      // design, so Create is a retry rather than a re-ask.
      if (landed) reset();
      return;
    }
    if (!name.trim() || !role.trim()) return;
    setCreating(true);
    let landed: boolean;
    try {
      landed = await onAdd({ name, role, description, inbox });
    } catch {
      landed = false;
    } finally {
      setCreating(false);
    }
    if (landed) reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) return close();
        onOpenChange(o);
      }}
    >
      {/* The icon goes away rather than going dead while the dialog is held: a
          control that is present and does nothing reads as a broken dialog,
          where its absence beside "Adding…" reads as "wait". */}
      <DialogContent className="sm:max-w-md" showCloseButton={!heldOpen}>
        <DialogHeader>
          <DialogTitle>Add teammate</DialogTitle>
          <DialogDescription>
            {describing
              ? "Name them and say what they should do. You can fill in the rest on their profile."
              : "Add a teammate to your company's roster."}
          </DialogDescription>
        </DialogHeader>
        {describing ? (
          <DescribeTeammate
            idPrefix="member-chat"
            name={described.name}
            description={described.description}
            // Both waits, not just the design one. `submit` captured these
            // values when Create was pressed, so an edit made while the button
            // says "Adding…" is already not in the request — and a write that
            // lands then resets or navigates and takes the edit with it. Held
            // for the same reason the exits are: the dialog should not accept
            // input it is going to discard.
            disabled={designing || creating}
            onNameChange={(next) => setDescribed((d) => ({ ...d, name: next }))}
            onDescriptionChange={(next) =>
              setDescribed((d) => ({ ...d, description: next }))
            }
          />
        ) : (
          <>
            {/* Said only when the full form arrived by hand-over, so the
                operator knows why the dialog changed under them. Never shown on
                the no-model path, where this form is simply what the dialog is. */}
            {designRefused && (
              <p className="text-2xs text-muted-foreground" data-testid="chat-add-handover">
                {/* The host's own reason, not a sentence of ours: "set up a
                    model", "try again", "say more" and "wait for the period to
                    reset" are four different next moves, and one line covering
                    all four could only be too vague to act on. */}
                {refusalNotice(designRefused === "unknown" ? undefined : designRefused)}
              </p>
            )}
            <div className="grid gap-2">
              <Label htmlFor="member-name">Name</Label>
              <Input
                id="member-name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. Nova"
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="member-role">Role</Label>
              <Input
                id="member-role"
                value={role}
                onChange={(e) => setRole(e.target.value)}
                placeholder="e.g. Growth Marketer"
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="member-desc">What they do</Label>
              <Textarea
                id="member-desc"
                rows={3}
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="e.g. Runs paid acquisition and reports on ROAS."
              />
            </div>
            <label className="flex items-center justify-between rounded-lg border p-3">
              <span className="flex items-center gap-2 text-sm">
                <Mail className="size-4 text-muted-foreground" /> Give this teammate an inbox
              </span>
              <Switch
                checked={inbox}
                onCheckedChange={setInbox}
                aria-label="Give this teammate an inbox"
              />
            </label>
          </>
        )}
        <DialogFooter className="items-center">
          {describing && describeBlocked && (
            <p className="mr-auto text-2xs text-muted-foreground" data-testid="chat-add-blocked">
              {describeBlocked}
            </p>
          )}
          {/* Live whenever leaving would actually stop something — which on a
              cancellable transport includes the whole "Designing…" wait, and
              never includes the write. It used to be disabled while `designing`
              while Escape, the backdrop and the close icon stayed live, so the
              one control that said what it would do was the one that would not
              do it. All four take this exit now, and `heldOpen` is the single
              place that decides whether the exit exists. */}
          <Button variant="ghost" onClick={close} disabled={heldOpen}>
            Cancel
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={
              describing
                ? Boolean(describeBlocked) || designing || creating
                : !name.trim() || !role.trim() || creating
            }
          >
            {/* Says what is happening, because both halves take time: the host
                runs a model over the sentence to write the role, the mandate
                and the persona, and only then is the teammate written. Two
                labels rather than one, because they are two waits and only the
                first is a model call the operator may want to walk away from. */}
            {designing ? "Designing…" : creating ? "Adding…" : "Add teammate"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
