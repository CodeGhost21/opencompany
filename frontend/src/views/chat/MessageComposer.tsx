import { useRef, useState } from "react";
import {
  ArrowUp,
  AtSign,
  Bold,
  CaseSensitive,
  Code,
  Italic,
  Loader2,
  Paperclip,
  Strikethrough,
  X,
} from "lucide-react";

import type { MessageIntent } from "@/api/tasks";
import type { AttachmentDto } from "@/api/types";
import { formatBytes } from "@/api/workspace";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface Props {
  placeholder: string;
  disabled?: boolean;
  onSend: (text: string, intent?: MessageIntent, attachments?: AttachmentDto[]) => void;
  /**
   * Called as the box is typed in, so the company can show a typing
   * indicator.
   *
   * Fired on **every** change rather than on a timer: throttling is the
   * caller's job, because it is per channel and this component does not know
   * which channel it is in. Absent on the composers where a typing indicator
   * would be noise.
   */
  onTyping?: () => void;
  /** Compact form, for the narrower thread panel. */
  compact?: boolean;
  /**
   * Show the what-is-this-message-for control (issues #580, #1152), opt-in per
   * composer.
   *
   * The channel and DM composers ask for it — either can open a board card, so
   * "just chatting" versus "do it once" versus "build me the workflow" belongs
   * at both prompt boxes. The thread and copilot composers never carry it, so
   * their `onSend` stays a plain `(text)` and their wire shape is unchanged.
   *
   * DMs were omitted when #580 landed (issue #845). Nothing downstream was
   * scoped to channels — the chat route reads `deliverable` off the payload
   * whatever thread it came from — so a DM asking for a workflow was sent as a
   * `once` card, dispatched to a desk agent holding no authoring tool, and came
   * back as a refusal. The control was the only part missing.
   */
  deliverableChoice?: boolean;
  /**
   * Upload one attachment's bytes and hand back its stored reference (issue
   * #1682). Given only where attaching makes sense — the channel and DM
   * composers — so the paperclip is present exactly when the surface can carry
   * a file. The composer holds the returned reference as a pending chip and
   * threads it onto the next `onSend`; the actual upload/verify lives in
   * `ChatView`.
   */
  uploadAttachment?: (file: File) => Promise<AttachmentDto>;
}

/** The markdown a toolbar button wraps the selection in. */
const WRAPS = [
  { icon: Bold, label: "Bold", mark: "**" },
  { icon: Italic, label: "Italic", mark: "_" },
  { icon: Strikethrough, label: "Strikethrough", mark: "~~" },
  { icon: Code, label: "Code", mark: "`" },
] as const;

/**
 * The composer dock.
 *
 * A bordered box that owns its own toolbar rather than a bare input: the
 * formatting buttons wrap the current selection in markdown, and the box grows
 * with the draft up to a cap before scrolling. Enter sends; Shift+Enter breaks
 * the line, which is the convention every chat client shares.
 */
export function MessageComposer({
  placeholder,
  disabled,
  onSend,
  compact,
  deliverableChoice,
  onTyping,
  uploadAttachment,
}: Props) {
  const [draft, setDraft] = useState("");
  // The single file staged for the next send (issue #1682). v1 carries one
  // attachment per message, so a fresh pick replaces the last rather than
  // appending — the wire (`Vec<Attachment>`) already allows more when the UI
  // grows to it.
  const [pending, setPending] = useState<AttachmentDto | null>(null);
  // The upload is in flight: the paperclip spins and Send waits, so a message
  // cannot post ahead of the bytes it references.
  const [uploading, setUploading] = useState(false);
  const [attachError, setAttachError] = useState<string>();
  const fileInput = useRef<HTMLInputElement>(null);
  // What the NEXT line is for, and only the next one. It starts and resets
  // unselected: an intent is an operator assertion, so no button may claim one
  // until the operator presses it (issue #984). An unmarked message therefore
  // reaches the host without an override and lets triage decide whether it is
  // work or conversation.
  const [intent, setIntent] = useState<MessageIntent>();
  // The formatting row is opt-in, behind the `Aa` toggle in the icon row. It
  // used to sit open above every composer, which spent the widest strip of the
  // dock on four buttons most lines never use.
  const [formatting, setFormatting] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);

  function send() {
    const text = draft.trim();
    // A message must carry text; an attachment rides an operator's words, it is
    // not a message on its own. Also held back while the upload is mid-flight,
    // so a send never references bytes that have not landed.
    if (!text || disabled || uploading) return;
    setDraft("");
    onSend(text, deliverableChoice ? intent : undefined, pending ? [pending] : undefined);
    setIntent(undefined);
    setPending(null);
    setAttachError(undefined);
  }

  /** Upload the picked file and stage its reference as the pending chip. */
  async function onPickFile(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    // Reset the input so re-picking the same file fires `change` again.
    e.target.value = "";
    if (!file || !uploadAttachment) return;
    setUploading(true);
    setAttachError(undefined);
    try {
      const reference = await uploadAttachment(file);
      setPending(reference);
    } catch (err) {
      // The filename is operator content — the message says an upload failed
      // without echoing what it was called.
      setAttachError(err instanceof Error ? err.message : "Couldn't attach that file.");
    } finally {
      setUploading(false);
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  /** Wrap the selection (or the caret) in `mark`, keeping focus in the box. */
  function wrap(mark: string) {
    const el = input.current;
    if (!el) return;
    const { selectionStart: start, selectionEnd: end } = el;
    const next = `${draft.slice(0, start)}${mark}${draft.slice(start, end)}${mark}${draft.slice(end)}`;
    setDraft(next);
    // Restore the selection around what was wrapped, after React repaints.
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(start + mark.length, end + mark.length);
    });
  }

  return (
    <div
      className={cn("shrink-0 px-4", compact ? "pb-3" : "pb-4")}
      // The guided tour spotlights the channel composer. The thread panel's
      // compact copy stays unlabelled so the tour can't anchor on the wrong one.
      data-tour={compact ? undefined : "chat-composer"}
    >
      <div className="rounded-xl border bg-card shadow-sm focus-within:ring-2 focus-within:ring-ring/40">
        {!compact && formatting && (
          <div className="flex items-center gap-0.5 border-b px-2 py-1">
            {WRAPS.map((w) => (
              <Button
                key={w.label}
                variant="ghost"
                size="icon"
                className="size-7 text-muted-foreground"
                onClick={() => wrap(w.mark)}
                aria-label={w.label}
                title={w.label}
              >
                <w.icon className="size-3.5" />
              </Button>
            ))}
          </div>
        )}

        {/* The staged attachment (issue #1682), shown above the box the moment
            its upload lands and cleared on send or removal. One chip in v1. */}
        {pending && (
          <div className="flex items-center gap-2 border-b px-3 py-1.5">
            <Paperclip className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            <span className="min-w-0 truncate text-xs font-medium" title={pending.name}>
              {pending.name}
            </span>
            <span className="shrink-0 text-2xs text-muted-foreground">
              {formatBytes(pending.size)}
            </span>
            <Button
              variant="ghost"
              size="icon"
              className="ml-auto size-6 shrink-0 text-muted-foreground"
              aria-label={`Remove ${pending.name}`}
              title="Remove attachment"
              onClick={() => setPending(null)}
            >
              <X className="size-3.5" />
            </Button>
          </div>
        )}
        {attachError && (
          <p role="alert" className="border-b px-3 py-1.5 text-2xs text-destructive">
            {attachError}
          </p>
        )}

        {/* A native textarea rather than the design-system one: the composer
            needs a ref to wrap the selection, and `Textarea` is a plain
            function component (React 18 — no ref forwarding). */}
        <textarea
          ref={input}
          value={draft}
          onChange={(e) => {
            setDraft(e.target.value);
            onTyping?.();
          }}
          onKeyDown={onKeyDown}
          aria-label={placeholder}
          placeholder={placeholder}
          rows={1}
          className="field-sizing-content max-h-48 min-h-10 w-full resize-none bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground"
        />

        {/* `flex-wrap` keeps Send reachable in a narrow pane (issue #1383):
            when the intent group and icon buttons can't share a line with it,
            the row wraps and Send drops to its own line — still `ml-auto`, so
            right-aligned and in-flow — rather than overflowing off-screen with
            no way to scroll to it. On a roomy composer it stays a single row. */}
        <div className="flex flex-wrap items-center gap-0.5 px-2 pb-1.5">
          {deliverableChoice && !compact && (
            <div
              className="mr-1 flex items-center gap-0.5 rounded-lg border p-0.5"
              role="group"
              // Issue #1152: the group no longer only asks what the message
              // should *produce* — "Just chatting" produces nothing — so it asks
              // what the message is for.
              aria-label="What this message is for"
            >
              {(
                [
                  // "Just chatting" leads, because it is the position that
                  // withholds: the operator reaches for it to stop something
                  // happening, and a control you press to prevent an action
                  // belongs before the ones that cause it. None is pre-pressed:
                  // an operator has to state which outcome they want.
                  { value: "chat", label: "Just chatting" },
                  { value: "once", label: "Do it once" },
                  { value: "workflow", label: "Build me the workflow" },
                ] as const
              ).map((option) => (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={intent === option.value}
                  onClick={() => setIntent(option.value)}
                  data-testid={`composer-deliverable-${option.value}`}
                  className={cn(
                    "rounded-md px-2 py-1 text-2xs font-medium transition-colors",
                    intent === option.value
                      ? "bg-primary/10 text-brand-700 dark:text-brand-300"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  {option.label}
                </button>
              ))}
            </div>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="size-7 text-muted-foreground"
            aria-label="Mention someone"
            title="Mention someone"
            onClick={() => wrap("@")}
          >
            <AtSign className="size-4" />
          </Button>
          {/* The paperclip (issue #1682), present exactly where attaching makes
              sense — a composer given an `uploadAttachment`. Born disabled and
              wired to nothing in the #361 console rebuild; this is where it
              starts working. */}
          {uploadAttachment && (
            <>
              <input
                ref={fileInput}
                type="file"
                className="hidden"
                aria-hidden
                tabIndex={-1}
                onChange={(e) => void onPickFile(e)}
              />
              <Button
                variant="ghost"
                size="icon"
                className="size-7 text-muted-foreground"
                aria-label="Attach a file"
                title="Attach a file"
                disabled={disabled || uploading}
                onClick={() => fileInput.current?.click()}
              >
                {uploading ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Paperclip className="size-4" />
                )}
              </Button>
            </>
          )}
          {!compact && (
            <Button
              variant="ghost"
              size="icon"
              className={cn(
                "size-7 text-muted-foreground",
                formatting && "bg-accent text-accent-foreground",
              )}
              aria-label="Formatting"
              aria-pressed={formatting}
              title="Formatting"
              onClick={() => setFormatting((f) => !f)}
            >
              <CaseSensitive className="size-4" />
            </Button>
          )}
          <Button
            size="icon"
            className="ml-auto size-9 rounded-full"
            onClick={send}
            disabled={disabled || !draft.trim()}
            aria-label="Send"
          >
            <ArrowUp className="size-4" />
          </Button>
        </div>
      </div>
      {!compact && (
        <p className="mt-1.5 px-1 text-2xs text-muted-foreground">
          <kbd className="font-sans font-medium">Enter</kbd> to send ·{" "}
          <kbd className="font-sans font-medium">Shift+Enter</kbd> for a new line
        </p>
      )}
    </div>
  );
}
