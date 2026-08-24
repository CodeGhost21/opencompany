import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowUp,
  AtSign,
  Bold,
  CaseSensitive,
  Code,
  Italic,
  Strikethrough,
} from "lucide-react";

import type { MessageIntent } from "@/api/tasks";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { MentionPicker } from "@/views/chat/MentionPicker";
import {
  activeMentionQuery,
  aliasSet,
  insertMention,
  mentionsOutsideChannel,
  mentionsOutsideRange,
  rankMentionables,
  reconcileMentions,
  reconcileWrap,
  resolvableMentions,
  stripCodeRegions,
  type Mention,
  type Mentionable,
} from "@/views/chat/mentions";

interface Props {
  placeholder: string;
  disabled?: boolean;
  onSend: (text: string, intent?: MessageIntent, mentions?: Mention[]) => void;
  /** A new revision replaces the draft and focuses the composer. */
  prefill?: { text: string; revision: number };
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
  /**
   * The ids of the teammates on this channel, for the outside-channel warning.
   * Absent when membership is unknown.
   */
  channelMemberIds?: string[];
  /**
   * Everything an `@` can name here, from `GET {scope}/chat/mentionables`.
   *
   * Absent — a host that predates the route, or a surface with no roster
   * loaded — simply means no picker opens; typing an `@` is then plain text
   * and the host still extracts what it can from it. So the composer degrades
   * to exactly its previous behaviour rather than to a broken one.
   */
  mentionables?: Mentionable[];
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
}

/** The markdown a toolbar button wraps the selection in. */
const WRAPS = [
  { icon: Bold, label: "Bold", mark: "**" },
  { icon: Italic, label: "Italic", mark: "_" },
  { icon: Strikethrough, label: "Strikethrough", mark: "~~" },
  { icon: Code, label: "Code", mark: "`" },
] as const;

/**
 * The end of the `@name` the caret sits inside, or `from` when nothing follows.
 *
 * Scans forward while the text still reads as a name the directory knows: a
 * character is included only if the run up to it is a prefix of some alias.
 * That keeps `@Jane Doe` whole (two words, one name) while stopping at the
 * space in `@engineer and …` the moment "engineer and" ceases to be a name —
 * so picking over an existing mention replaces the mention, not the sentence.
 */
function activeMentionEnd(
  text: string,
  from: number,
  nameStart: number,
  aliases: ReadonlySet<string>,
): number {
  let i = from;
  // No aliases means no name can match, so the token cannot extend past its
  // start: stop scanning rather than walk the loop with `prefix` never set.
  if (aliases.size === 0) return nameStart;
  while (i < text.length) {
    const ch = text[i];
    const next =
      /[A-Za-z0-9_.-]/.test(ch)
        ? i + 1
        : ch === " " && /[A-Za-z0-9_]/.test(text[i + 1] ?? "")
          ? i + 1
          : null;
    if (next === null) return i;
    const name = text.slice(nameStart, next).toLowerCase();
    let prefix = false;
    for (const alias of aliases) {
      if (alias.startsWith(name)) {
        prefix = true;
        break;
      }
    }
    if (!prefix) return i;
    i = next;
  }
  return i;
}

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
  prefill,
  compact,
  channelMemberIds,
  deliverableChoice,
  mentionables,
  onTyping,
}: Props) {
  const [draft, setDraft] = useState("");
  // What the draft currently resolves to. Reconciled on every edit, so editing
  // or backspacing through a chip un-mentions it rather than leaving a ping
  // for somebody whose name is no longer in the message.
  const [mentions, setMentions] = useState<Mention[]>([]);
  // Where the caret is in an `@query`, or null when it is not in one. Held in
  // state (not derived at render) because it has to survive the mouse leaving
  // the textarea to click a row.
  const [query, setQuery] = useState<{ start: number; query: string } | null>(null);
  // Teammate ids the draft addresses who cannot see this channel. Checked on
  // send; a non-empty list warns rather than sending, and a second send passes
  // through after the user has confirmed.
  const [outsideWarning, setOutsideWarning] = useState<string[] | null>(null);
  const [activeRow, setActiveRow] = useState(0);
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

  // A first-run card lives above the timeline, outside this component. The
  // revision lets it request the same prompt more than once after an operator
  // edits or clears it; comparing text alone would make the second click inert.
  useEffect(() => {
    if (!prefill) return;
    setDraft(prefill.text);
    setMentions([]);
    setOutsideWarning(null);
    closePicker();
    setIntent("once");
    input.current?.focus();
  }, [prefill]);

  function closePicker() {
    setQuery(null);
    setActiveRow(0);
  }

  const rows = useMemo(
    () => (query && mentionables ? rankMentionables(mentionables, query.query) : []),
    [query, mentionables],
  );
  // Built once per directory, not per keystroke.
  const aliases = useMemo(
    () => (mentionables ? aliasSet(mentionables) : undefined),
    [mentionables],
  );
  const pickerOpen = query !== null && rows.length > 0;

  /** Re-read the caret's mention query after any edit or caret move. */
  function syncQuery(text: string, caret: number | null) {
    if (!mentionables || caret === null) {
      closePicker();
      return;
    }
    // Code regions are masked so an `@` inside backticks never opens the picker
    // (a supplied mention would survive revalidation, since the host's code
    // mask only applies to its own extraction). Masking preserves offsets, so
    // the range this returns is valid against the raw `text`.
    const next = activeMentionQuery(stripCodeRegions(text), caret, aliases);
    setQuery(next);
    setActiveRow(0);
  }

  function onChange(e: React.ChangeEvent<HTMLTextAreaElement>) {
    const text = e.target.value;
    setDraft(text);
    // Trailing the text, so a mention whose span was edited away goes with it.
    // The previous draft disambiguates which of two same-text mentions the
    // edit deleted — without it, deleting the second `@Sam @Sam` re-anchors
    // the deleted Sam onto the survivor and pings the wrong person.
    setMentions((current) => reconcileMentions(text, current, draft, e.target.selectionStart));
    setOutsideWarning(null);
    syncQuery(text, e.target.selectionStart);
    onTyping?.();
  }

  function pick(entry: Mentionable) {
    const el = input.current;
    if (!query || !el) return;
    const caret = el.selectionStart ?? query.start;
    // Replace the whole `@name`, not just the part before the caret. Moving the
    // caret into an existing `@engineer` (the onSelect path) opens the picker
    // with `query.query` = "eng"; replacing only that would leave `@ceo ineer`.
    // A selection the reader made is honoured, so replacing spans what they
    // selected when the selection reaches past the token.
    const selectionEnd = Math.max(el.selectionEnd ?? caret, caret);
    const tokenEnd = activeMentionEnd(
      draft,
      caret,
      query.start + 1,
      aliases ?? new Set<string>(),
    );
    const range = { start: query.start, end: Math.max(selectionEnd, tokenEnd) };
    const result = insertMention(draft, range, entry);
    // `insertMention` is typed to always return a result, but guard anyway:
    // it is a module-boundary call and a future refactor must not let a
    // malformed range turn a pick into a dereference of `undefined`.
    if (!result) return;
    setDraft(result.text);
    // A pick replaces the token under the caret, so any mention the range
    // touched is gone from the draft. Drop those before reconciling, or the
    // replaced identity re-anchors onto a same-text duplicate elsewhere —
    // replacing the picked first `@Sam` in `@Sam then @Sam` with `@engineer`
    // would otherwise move Sam onto the second, hand-typed span.
    setMentions((current) =>
      reconcileMentions(result.text, [
        ...mentionsOutsideRange(current, range),
        result.mention,
      ]),
    );
    setOutsideWarning(null);
    closePicker();
    // After React repaints, same as `wrap` below.
    requestAnimationFrame(() => {
      el.focus();
      el.setSelectionRange(result.caret, result.caret);
    });
  }

  function send() {
    const text = draft.trim();
    if (!text || disabled) return;
    // The trim can shift every span, so the list is re-anchored against exactly
    // what is being sent — never against the untrimmed draft.
    let sending = reconcileMentions(text, mentions);
    // A previously selected mention can be wrapped in Markdown code after the
    // picker closes. The host's fallback extractor masks code, so supplied
    // mentions must obey the same rule rather than bypassing it.
    const masked = stripCodeRegions(text);
    sending = sending.filter(
      (m) =>
        masked.slice(m.offset, m.offset + m.text.length) ===
        text.slice(m.offset, m.offset + m.text.length),
    );
    // A mention completed by hand (`@ceo ` — the query closed on the finished
    // name) never entered `mentions`. When anything was picked, the host uses
    // the supplied list exclusively, so sending just the picks would silently
    // skip the typed one and the person would never be notified. Resolve every
    // span the directory can name and send the union, keeping the picker's
    // explicit targets for names the host's extraction would refuse as
    // ambiguous.
    if (mentionables) {
      for (const m of resolvableMentions(text, mentionables)) {
        if (!sending.some((s) => s.text === m.text && s.offset === m.offset)) {
          sending = [...sending, m];
        }
      }
      sending = reconcileMentions(text, sending);
    }
    // On first send with outside-channel mentions, warn instead of sending.
    // The directory rows carry each desk's membership, so a desk mention is
    // judged by its blast radius, not skipped because its target is a desk.
    const outside = mentionsOutsideChannel(sending, channelMemberIds, mentionables);
    if (outside.length > 0 && !outsideWarning) {
      setOutsideWarning(outside);
      return;
    }
    setOutsideWarning(null);
    setDraft("");
    setMentions([]);
    closePicker();
    onSend(
      text,
      deliverableChoice ? intent : undefined,
      // Preserve absent-versus-empty: a loaded directory that resolves no
      // spans intentionally sends [] to suppress host fallback extraction.
      mentionables ? sending : undefined,
    );
    // Back to unselected, not to a default (issue #984).
    setIntent(undefined);
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // While the picker is open it owns these keys. Enter in particular PICKS
    // and does not send — a person mid-`@name` is choosing somebody, not
    // finishing a message, and sending there is unrecoverable.
    if (pickerOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveRow((i) => (i + 1) % rows.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveRow((i) => (i - 1 + rows.length) % rows.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        const entry = rows[activeRow];
        // `rows` is non-empty here by `pickerOpen`, but a render between this
        // keydown and the picker closing can shrink the list, leaving
        // `activeRow` past its end. Picking `undefined` would throw inside
        // `insertMention`, so resolve the row and guard before calling.
        if (entry) pick(entry);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        closePicker();
        return;
      }
    }
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
    // The wrap edits the draft out from under the mention spans. A mention the
    // wrap merely encloses keeps its literal (``**@Sam**`` still reads `@Sam`)
    // and shifts; one whose span an insertion point falls inside is broken and
    // must go — otherwise send-time reconciliation re-anchors it onto an
    // unrelated same-text duplicate and pings the wrong person.
    setMentions((current) => reconcileWrap(current, start, end, mark));
    setOutsideWarning(null);
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
      <div className="relative rounded-xl border bg-card shadow-sm focus-within:ring-2 focus-within:ring-ring/40">
        {pickerOpen && (
          <MentionPicker
            entries={rows}
            active={activeRow}
            onPick={pick}
            onHover={setActiveRow}
          />
        )}
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

        {outsideWarning && (
          <p
            role="alert"
            className="flex items-center gap-1.5 border-b bg-warning/10 px-3 py-1.5 text-xs text-muted-foreground"
          >
            <svg className="size-3.5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
              <path d="M12 9v4m0 4h.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" />
            </svg>
            <span className="min-w-0">
              <span className="font-medium">{outsideWarning.join(", ")}</span>
              {" "}can't see this channel — send again to notify anyway
            </span>
          </p>
        )}
        {/* A native textarea rather than the design-system one: the composer
            needs a ref to wrap the selection, and `Textarea` is a plain
            function component (React 18 — no ref forwarding). */}
        <textarea
          ref={input}
          value={draft}
          onChange={onChange}
          onKeyDown={onKeyDown}
          // A click or an arrow can move the caret into (or out of) an existing
          // `@name` without changing the text, so the query is re-read on
          // selection changes too, not only on edits.
          onSelect={(e) => {
            const el = e.currentTarget;
            syncQuery(el.value, el.selectionStart);
          }}
          onBlur={closePicker}
          role="combobox"
          aria-autocomplete="list"
          aria-controls={pickerOpen ? "mention-picker" : undefined}
          aria-activedescendant={pickerOpen ? `mention-option-${activeRow}` : undefined}
          aria-expanded={pickerOpen}
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
                  {
                    value: "chat",
                    label: "Just chatting",
                    title: "Chat without automatically creating a task.",
                  },
                  {
                    value: "once",
                    label: "Do it once",
                    title: "Ask the team to do this once.",
                  },
                  {
                    value: "workflow",
                    label: "Build me the workflow",
                    title: "Turn this into a repeating workflow.",
                  },
                ] as const
              ).map((option) => (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={intent === option.value}
                  onClick={() => setIntent(option.value)}
                  data-testid={`composer-deliverable-${option.value}`}
                  title={option.title}
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
            // Types the `@` and lets the ordinary path take over, rather than
            // opening the picker directly: one code path decides when a picker
            // is open, so the button and the keyboard can never disagree.
            onClick={() => {
              const el = input.current;
              if (!el) return;
              const at = el.selectionStart ?? draft.length;
              // A separator first when the caret is mid-word, or the `@` would
              // land inside another token and open nothing.
              const lead = at > 0 && !/[\s([{]/.test(draft[at - 1] ?? " ") ? " " : "";
              const next = `${draft.slice(0, at)}${lead}@${draft.slice(at)}`;
              const caret = at + lead.length + 1;
              // The insertion shifts every recorded mention at/after it and
              // breaks the literal of one it lands inside. Reconcile now, as
              // `onChange` does for a keystroke, so the stale span cannot
              // re-anchor onto a same-text duplicate at send time.
              setMentions((current) => reconcileMentions(next, current, draft, caret));
              setDraft(next);
              requestAnimationFrame(() => {
                el.focus();
                el.setSelectionRange(caret, caret);
                syncQuery(next, caret);
              });
            }}
          >
            <AtSign className="size-4" />
          </Button>
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
