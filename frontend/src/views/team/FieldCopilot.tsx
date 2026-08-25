// Issue #1776: the copilot control that sits under a teammate's mandate or
// persona box.
//
// ## It suggests; it never fills, and it never saves
//
// A draft lands in a card BESIDE the field, not in it. The operator reads it
// and presses Use it or Discard; Use it writes into the form draft, and the
// form's own Save is still what stores anything. Two deliberate actions stand
// between a model's sentence and a running persona.
//
// That ordering is the point rather than a nicety. `WorkflowCreateDialog`'s
// create-time copilot hydrates its form directly and has to ask
// "replace what you've started?" after the fact, with a `window.confirm` and a
// dirty check, because a draft that lands in the form is a draft that can
// destroy work (issues #1005, #1052). A draft that lands beside the form cannot
// — so this control needs no confirm, no dirty tracking, and no rule about what
// a late response may overwrite. It renders next to what the operator typed and
// lets them compare.

import { useRef, useState } from "react";
import { Loader2, Sparkles } from "lucide-react";

import type { DraftableField, ProfileDraft } from "@/api/agent-copilot";
import { refusalNotice } from "@/api/agent-copilot";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

/** What the copilot calls each field when it talks about it. */
const FIELD_NOUN: Record<DraftableField, string> = {
  description: "mandate",
  instructions: "instructions",
};

/** The note box's placeholder, per field. */
const HINT_PLACEHOLDER: Record<DraftableField, string> = {
  description:
    "Optional: anything the copilot should know — e.g. “they own paid social, not the newsletter”.",
  instructions:
    "Optional: anything the copilot should know — e.g. “must never launch a campaign without sign-off”.",
};

export function FieldCopilot({
  field,
  onDraft,
  onAccept,
  disabled,
  disabledNotice,
}: {
  field: DraftableField;
  /**
   * Asks the host for a draft. Supplied by the surface, because only it knows
   * whether this teammate exists yet — the detail view addresses it by id, and
   * the Add-teammate form sends the role being typed.
   */
  onDraft: (hint: string) => Promise<ProfileDraft>;
  /** Puts accepted text into the form draft. Saves nothing. */
  onAccept: (text: string) => void;
  /** Whether drafting can be asked for at all right now. */
  disabled?: boolean;
  /** Why not, when it cannot. Rendered in place of the usual hint line. */
  disabledNotice?: string;
}) {
  const [open, setOpen] = useState(false);
  const [hint, setHint] = useState("");
  const [drafting, setDrafting] = useState(false);
  const [suggestion, setSuggestion] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  /**
   * Which request the operator is still waiting for.
   *
   * Bumped by every action that abandons one — closing the panel, discarding,
   * accepting, asking again. A response whose epoch is stale is dropped: the
   * operator has moved on, and a suggestion card reappearing under a panel they
   * closed reads as the copilot ignoring them.
   */
  const epoch = useRef(0);

  function abandon() {
    epoch.current += 1;
    setDrafting(false);
  }

  async function run() {
    if (drafting) return;
    const requested = ++epoch.current;
    setDrafting(true);
    setNotice(null);
    setSuggestion(null);
    try {
      const draft = await onDraft(hint);
      if (epoch.current !== requested) return;
      if (draft.text) {
        setSuggestion(draft.text);
      } else {
        // A refusal names which of the three happened, so the sentence can name
        // the operator's next move rather than saying "couldn't draft that".
        setNotice(refusalNotice(draft.reason));
      }
    } catch (error) {
      if (epoch.current !== requested) return;
      setNotice(
        error instanceof Error
          ? error.message
          : "The copilot couldn't be reached.",
      );
    } finally {
      if (epoch.current === requested) setDrafting(false);
    }
  }

  if (!open) {
    return (
      <div className="flex items-center justify-between gap-2">
        <p className="text-2xs leading-snug text-muted-foreground">
          {disabled && disabledNotice ? disabledNotice : ""}
        </p>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-2xs text-muted-foreground"
          disabled={disabled}
          onClick={() => setOpen(true)}
          data-testid={`agent-copilot-open-${field}`}
        >
          <Sparkles className="mr-1 size-3.5" />
          Draft with copilot
        </Button>
      </div>
    );
  }

  return (
    <div
      className="space-y-2 rounded-lg border bg-muted/30 p-3"
      data-testid={`agent-copilot-${field}`}
    >
      <Textarea
        rows={2}
        value={hint}
        onChange={(e) => setHint(e.target.value)}
        placeholder={HINT_PLACEHOLDER[field]}
        disabled={drafting}
        data-testid={`agent-copilot-hint-${field}`}
      />
      <div className="flex items-center justify-between gap-2">
        <p className="text-2xs leading-snug text-muted-foreground">
          {/* Said plainly and every time, because it is the promise this whole
              control rests on — and the one an operator has no way to verify. */}
          Nothing is saved until you press Use it and then Save.
        </p>
        <div className="flex items-center gap-1">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-2xs"
            onClick={() => {
              abandon();
              setOpen(false);
              setSuggestion(null);
              setNotice(null);
            }}
            data-testid={`agent-copilot-close-${field}`}
          >
            Close
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={drafting || disabled}
            onClick={() => void run()}
            data-testid={`agent-copilot-draft-${field}`}
          >
            {drafting ? (
              <Loader2 className="mr-1 size-3.5 animate-spin" />
            ) : (
              <Sparkles className="mr-1 size-3.5" />
            )}
            {drafting
              ? "Drafting…"
              : suggestion
                ? "Draft again"
                : `Draft the ${FIELD_NOUN[field]}`}
          </Button>
        </div>
      </div>

      {suggestion && (
        <div
          className="space-y-2 rounded-md border bg-background p-3"
          data-testid={`agent-copilot-suggestion-${field}`}
        >
          {/* The draft is rendered as text, never in an editable box. An
              editable suggestion invites someone to polish it in place and then
              lose the work to Discard — the field itself is where editing
              belongs, which is what Use it puts it in. */}
          <p className="whitespace-pre-wrap text-sm">{suggestion}</p>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              size="sm"
              onClick={() => {
                abandon();
                onAccept(suggestion);
                setSuggestion(null);
                setNotice(null);
                setOpen(false);
              }}
              data-testid={`agent-copilot-accept-${field}`}
            >
              Use it
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => {
                abandon();
                setSuggestion(null);
              }}
              data-testid={`agent-copilot-discard-${field}`}
            >
              Discard
            </Button>
            <span className="text-2xs text-muted-foreground">
              Use it fills the box below — you can still edit it before saving.
            </span>
          </div>
        </div>
      )}

      {notice && (
        <Alert data-testid={`agent-copilot-notice-${field}`}>
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}
    </div>
  );
}
