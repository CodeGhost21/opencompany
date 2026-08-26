// Issue #1776: the copilot that drafts a teammate's mandate or persona — in
// conversation, and never into the field until you say so.
//
// ## Why this is a chat and not a Draft button
//
// It shipped as one shot: a note box, a Draft button, one answer. Using it made
// the problem obvious. A single note cannot say "no, more like this", and every
// refinement meant retyping the whole instruction, because nothing carried
// between presses. The shape was wrong, not the model.
//
// So it is turns now. Each request sends the transcript; each answer is a
// sentence about what changed plus the whole field rewritten. The copilot may
// also ask a question and draft nothing — the thing a one-shot pass
// structurally cannot do, and the reason it could never find out what the
// operator actually meant.
//
// ## It still suggests; it still never fills, and it still never saves
//
// A draft lands in a card BESIDE the field, not in it. `Use it` writes into the
// form draft; the form's own Save is what stores anything. Two deliberate
// actions stand between a model's sentence and a running persona, and that is
// what makes this safe to point at a system prompt at all.
//
// That ordering is also what keeps the panel simple. `WorkflowCreateDialog`'s
// copilot hydrates its form directly and has to ask "replace what you've
// started?" after the fact, with a confirm and a dirty check (issues #1005,
// #1052), because a draft that lands in the form can destroy work. A draft that
// lands beside it cannot — so there is no confirm here, no dirty tracking, and
// no rule about what a late answer may overwrite.

import { useEffect, useRef, useState } from "react";
import { Loader2, Send, Sparkles } from "lucide-react";

import type { CopilotTurn, DraftableField, ProfileDraft } from "@/api/agent-copilot";
import { copilotTurnText, refusalNotice } from "@/api/agent-copilot";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";

/** What the copilot calls each field when it talks about it. */
const FIELD_NOUN: Record<DraftableField, string> = {
  description: "mandate",
  instructions: "instructions",
};

/** The composer's placeholder, per field. */
const REPLY_PLACEHOLDER: Record<DraftableField, string> = {
  description: "Tell it what to change — e.g. “they own paid social, not the newsletter”",
  instructions: "Tell it what to change — e.g. “add that they never launch without sign-off”",
};

export function FieldCopilot({
  field,
  onTurn,
  onAccept,
  disabled,
  disabledNotice,
}: {
  field: DraftableField;
  /**
   * Asks the host for the next turn, given the conversation so far.
   *
   * Supplied by the surface, because only it knows whether this teammate exists
   * yet — the detail view addresses it by id, the Add-teammate form sends the
   * role being typed.
   */
  onTurn: (conversation: CopilotTurn[]) => Promise<ProfileDraft>;
  /** Puts an accepted draft into the form draft. Saves nothing. */
  onAccept: (text: string) => void;
  /** Whether the copilot can be asked for anything right now. */
  disabled?: boolean;
  /** Why not, when it cannot. Rendered in place of the usual hint line. */
  disabledNotice?: string;
}) {
  const [open, setOpen] = useState(false);
  const [turns, setTurns] = useState<CopilotTurn[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  /**
   * Which request the operator is still waiting for.
   *
   * Bumped by every action that abandons one — closing the panel, accepting.
   * An answer whose epoch is stale is dropped: the operator has moved on, and a
   * turn appearing under a panel they closed reads as the copilot ignoring
   * them.
   */
  const epoch = useRef(0);
  const scroller = useRef<HTMLDivElement | null>(null);

  // Keep the newest turn in view. A conversation that silently grows upward is
  // one where the answer you just asked for is off-screen.
  useEffect(() => {
    // `scrollTo` is optional-called: jsdom does not implement it, and a test
    // environment must not be the reason a panel throws on mount.
    scroller.current?.scrollTo?.({ top: scroller.current.scrollHeight });
  }, [turns, busy]);

  /**
   * Runs one turn: appends what the operator said (when they said anything),
   * asks the host with the whole transcript, and appends the answer.
   *
   * The transcript is built here rather than read from state, because `setState`
   * has not applied by the time the request goes out — asking with the state
   * as it stands would send the conversation minus the message being answered.
   */
  async function run(said?: string) {
    if (busy || disabled) return;
    const asked: CopilotTurn[] = said
      ? [...turns, { role: "operator" as const, text: said }]
      : turns;
    const requested = ++epoch.current;
    setTurns(asked);
    setInput("");
    setBusy(true);
    setNotice(null);
    try {
      const answer = await onTurn(asked);
      if (epoch.current !== requested) return;
      if (answer.source === "unavailable" || (!answer.reply && !answer.text)) {
        // A refusal names which of the three happened, so the sentence can name
        // the operator's next move rather than saying "couldn't draft that".
        setNotice(refusalNotice(answer.reason));
        return;
      }
      const reply = answer.reply?.trim() ?? "";
      setTurns([
        ...asked,
        {
          role: "copilot",
          text: copilotTurnText(reply, answer.text),
          draft: answer.text,
        },
      ]);
    } catch (error) {
      if (epoch.current !== requested) return;
      setNotice(
        error instanceof Error ? error.message : "The copilot couldn't be reached.",
      );
    } finally {
      if (epoch.current === requested) setBusy(false);
    }
  }

  function close() {
    epoch.current += 1;
    setBusy(false);
    setOpen(false);
    setTurns([]);
    setInput("");
    setNotice(null);
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
          onClick={() => {
            setOpen(true);
            // Opens with a first version rather than an empty chat. Someone who
            // opened the copilot on a blank persona box wants something to react
            // to — asking them to describe it first is asking for the very thing
            // they could not write.
            void run();
          }}
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
      <div className="flex items-center justify-between gap-2">
        <p className="text-2xs font-medium text-muted-foreground">
          Copilot · {FIELD_NOUN[field]}
        </p>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-6 px-2 text-2xs"
          onClick={close}
          data-testid={`agent-copilot-close-${field}`}
        >
          Close
        </Button>
      </div>

      <div
        ref={scroller}
        className="max-h-80 space-y-2 overflow-y-auto"
        data-testid={`agent-copilot-transcript-${field}`}
      >
        {turns.map((turn, i) =>
          turn.role === "operator" ? (
            <p
              key={i}
              className="ml-6 rounded-md bg-background px-2.5 py-1.5 text-sm"
              data-testid={`agent-copilot-said-${field}`}
            >
              {turn.text}
            </p>
          ) : (
            <div key={i} className="space-y-1.5">
              {/* The copilot's own sentence — what it changed, or what it needs
                  to know. Rendered even when there is no draft: a turn that
                  asked is the point of this being a conversation. */}
              {turn.text.split("\n\nDraft:\n")[0]?.trim() && (
                <p className="text-sm text-muted-foreground">
                  {turn.text.split("\n\nDraft:\n")[0].trim()}
                </p>
              )}
              {turn.draft && (
                <div
                  className="space-y-2 rounded-md border bg-background p-2.5"
                  data-testid={`agent-copilot-suggestion-${field}`}
                >
                  {/* Rendered as text, never in an editable box. An editable
                      suggestion invites polishing it in place and then losing
                      the work — the field itself is where editing belongs,
                      which is what Use it puts it in. */}
                  <p className="whitespace-pre-wrap text-sm">{turn.draft}</p>
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      size="sm"
                      className="h-7"
                      onClick={() => {
                        const text = turn.draft;
                        if (!text) return;
                        epoch.current += 1;
                        onAccept(text);
                        close();
                      }}
                      data-testid={`agent-copilot-accept-${field}`}
                    >
                      Use it
                    </Button>
                    <span className="text-2xs text-muted-foreground">
                      Fills the box below — you can still edit it before saving.
                    </span>
                  </div>
                </div>
              )}
            </div>
          ),
        )}
        {busy && (
          <p
            className="flex items-center gap-1.5 text-2xs text-muted-foreground"
            data-testid={`agent-copilot-busy-${field}`}
          >
            <Loader2 className="size-3 animate-spin" /> Thinking…
          </p>
        )}
      </div>

      {notice && (
        <Alert data-testid={`agent-copilot-notice-${field}`}>
          <AlertDescription>{notice}</AlertDescription>
        </Alert>
      )}

      <div className="flex items-end gap-2">
        <Textarea
          rows={2}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // Enter sends, Shift+Enter breaks the line — the convention every
            // composer in this console follows.
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              if (input.trim()) void run(input.trim());
            }
          }}
          placeholder={REPLY_PLACEHOLDER[field]}
          disabled={busy}
          data-testid={`agent-copilot-input-${field}`}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8"
          disabled={busy || disabled || !input.trim()}
          onClick={() => void run(input.trim())}
          data-testid={`agent-copilot-send-${field}`}
        >
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Send className="size-3.5" />}
        </Button>
      </div>
      <p className="text-2xs leading-snug text-muted-foreground">
        {/* Said plainly and every time, because it is the promise this whole
            control rests on — and the one an operator has no way to verify. */}
        Nothing is saved until you press Use it and then Save.
      </p>
    </div>
  );
}
