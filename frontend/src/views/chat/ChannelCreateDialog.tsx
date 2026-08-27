// The channel creator: chat's own door onto `client.createDesk` (issue #1835).
//
// Deliberately not `DeskCreateDialog` reused: that form is (correctly, since
// #1827) about hierarchy — "the top teammate leads the desk", a Lead badge, a
// Make-lead control — and a channel has none of it. This one asks only what a
// conversation needs: a name, what it's for, who's in it. It posts with
// `responder: "auto"`, so the host stores a leadless channel whose answerer is
// picked per message; `members` order carries no rank and there is no lead UI
// because there is no lead fact to surface.
//
// Same write path as the org chart's dialog — one `createDesk`, one storage
// shape — so a rail-created channel is byte-identical to an org-chart desk
// apart from the responder mode. No second write path to drift.

import { useEffect, useId, useRef, useState } from "react";
import { Check } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { DeskDto } from "@/api/types";
import { TeammateAvatar } from "@/components/teammate-avatar";
import { Alert, AlertDescription } from "@/components/ui/alert";
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
import { Textarea } from "@/components/ui/textarea";
import type { TeamMember } from "@/lib/team";
import { cn } from "@/lib/utils";

export function ChannelCreateDialog({
  client,
  company,
  members,
  open,
  onOpenChange,
  onCreated,
}: {
  client: OpenCompanyClient;
  company: string | null;
  /** The roster to pick from — chat already holds it, so no refetch. */
  members: TeamMember[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (desk: DeskDto) => void;
}) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  // The chosen member ids, in selection order. Order carries NO rank here —
  // an auto channel has no lead — it is kept only so the list reads stably.
  const [chosen, setChosen] = useState<string[]>([]);
  const [memberFilter, setMemberFilter] = useState("");
  const [submitting, setSubmitting] = useState(false);
  // Field-level vs whole-form messages kept apart, as `DeskCreateDialog` does
  // (issue #1100): the name complaint renders at the name field, the host's
  // refusal of the whole form banners above the footer.
  const [nameError, setNameError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const formId = useId();
  const nameErrorId = `${formId}-name-error`;

  // Reset the draft each time the dialog opens, so a prior attempt never
  // leaks into the next one.
  useEffect(() => {
    if (!open) return;
    setName("");
    setDescription("");
    setChosen([]);
    setMemberFilter("");
    setNameError(null);
    setError(null);
  }, [open]);

  function toggle(id: string) {
    setChosen((prev) =>
      prev.includes(id) ? prev.filter((m) => m !== id) : [...prev, id],
    );
  }

  const needle = memberFilter.trim().toLowerCase();
  const visible = members.filter(
    (m) => !needle || m.name.toLowerCase().includes(needle) || m.role.toLowerCase().includes(needle),
  );

  async function submit() {
    if (!name.trim()) {
      setNameError("Give the channel a name.");
      setError(null);
      requestAnimationFrame(() => {
        nameRef.current?.focus({ preventScroll: true });
        nameRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
      return;
    }
    setNameError(null);
    setSubmitting(true);
    setError(null);
    try {
      const created = await client.createDesk(
        {
          name: name.trim(),
          description: description.trim() || undefined,
          members: chosen.length > 0 ? chosen : undefined,
          responder: "auto",
        },
        company,
      );
      onCreated(created);
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not create the channel");
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !submitting && onOpenChange(o)}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>New channel</DialogTitle>
          <DialogDescription>
            A channel is a conversation, not a hierarchy: nobody leads it. Pick who&apos;s in it —
            whoever fits each message picks it up, and an @-mention always wins.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${formId}-name`}>Name</Label>
            <Input
              id={`${formId}-name`}
              ref={nameRef}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Launch week"
              aria-invalid={nameError ? true : undefined}
              aria-describedby={nameError ? nameErrorId : undefined}
              disabled={submitting}
            />
            {nameError && (
              <p id={nameErrorId} className="text-xs text-destructive">
                {nameError}
              </p>
            )}
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${formId}-description`}>What it&apos;s for</Label>
            <Textarea
              id={`${formId}-description`}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Optional — one line about what belongs here"
              rows={2}
              disabled={submitting}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor={`${formId}-member-filter`}>Members</Label>
            <Input
              id={`${formId}-member-filter`}
              value={memberFilter}
              onChange={(e) => setMemberFilter(e.target.value)}
              placeholder="Filter teammates"
              disabled={submitting}
            />
            <ul className="mt-1 flex max-h-56 flex-col gap-px overflow-y-auto rounded-md border p-1">
              {visible.map((member) => {
                const selected = chosen.includes(member.id);
                return (
                  <li key={member.id}>
                    <button
                      type="button"
                      onClick={() => toggle(member.id)}
                      aria-pressed={selected}
                      disabled={submitting}
                      className={cn(
                        "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent",
                        selected && "bg-accent",
                      )}
                    >
                      <TeammateAvatar
                        name={member.name}
                        avatar={member.avatar}
                        tone={member.tone}
                        className="size-5 shrink-0"
                      />
                      <span className="truncate">{member.name}</span>
                      <span className="truncate text-xs text-muted-foreground">{member.role}</span>
                      {selected && <Check className="ml-auto size-4 shrink-0" aria-hidden />}
                    </button>
                  </li>
                );
              })}
              {visible.length === 0 && (
                <li className="px-2 py-1.5 text-xs text-muted-foreground">No teammates match.</li>
              )}
            </ul>
            {chosen.length > 0 && (
              <p className="text-xs text-muted-foreground">
                {chosen.length} in this channel — no lead; whoever fits each message answers.
              </p>
            )}
          </div>
        </div>

        {error && (
          <Alert variant="destructive">
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={submitting}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={submitting}>
            {submitting ? "Creating…" : "Create channel"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
