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
  onAdd: (fields: NewMemberFields) => void;
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

  useEffect(() => {
    if (!open) return;
    let live = true;
    (async () => {
      try {
        const status = await getInferenceStatus(client, company);
        if (live) setCognition(status.cognition);
      } catch {
        if (live) setCognition(null);
      }
    })();
    return () => {
      live = false;
    };
  }, [open, client, company]);

  const describing =
    addTeammateSurface({ cognition, designRefused: designRefused !== null }) === "describe";
  /** Why the reduced dialog's Create is dead, or `null` when it is not. */
  const describeBlocked = blockedReason(described);

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
    if (describing) {
      if (blockedReason(described)) return;
      const mine = attempt.current;
      const controller = new AbortController();
      designAbort.current = controller;
      setDesigning(true);
      let design;
      try {
        design = await designTeammate(client, company, described, controller.signal);
      } catch {
        // A transport, auth or not-found failure — not one of the four design
        // refusals, which arrive as a 200. Treated the same way by the dialog
        // because the operator's move is the same: take the form.
        if (attempt.current === mine) handOver("unknown");
        return;
      }
      if (attempt.current !== mine) return;
      const fields = designedTeammateFields(described, design);
      if (!fields) {
        handOver(design.reason ?? "unknown");
        return;
      }
      onAdd({ ...fields, landOnProfile: true });
      reset();
      return;
    }
    if (!name.trim() || !role.trim()) return;
    onAdd({ name, role, description, inbox });
    reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) return close();
        onOpenChange(o);
      }}
    >
      <DialogContent className="sm:max-w-md">
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
            disabled={designing}
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
          {/* Live during a design, not dead. It was disabled while `designing`
              and the three other ways out of a dialog — Escape, the backdrop,
              the header's close icon — were not, so the one control that said
              what it would do was the one that would not do it. All four now
              take the same exit, and that exit aborts the request. */}
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={
              describing
                ? Boolean(describeBlocked) || designing
                : !name.trim() || !role.trim()
            }
          >
            {/* Says what is happening, because it takes seconds: the host is
                running a model over the sentence to write the role, the mandate
                and the persona before anything is created. */}
            {designing ? "Designing…" : "Add teammate"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
