// Backtick code spans in a one-line string, rendered rather than printed.
//
// A ledger's `purpose` and `writtenBy` are authored as Markdown — the derived
// file renders them, and the strings lean on it: *"this ledger is written
// through the board — not with `record_entry`"*. The console printed them as
// plain text, so every surface that showed one showed the backticks: the Work
// page subtitle, its Details disclosure, and every row of Manage lists.
//
// Deliberately not the full `Markdown` component. These are single-line labels
// sitting inside an `h1`'s subtitle or a table row, and that component brings a
// block layout — paragraphs, headings, lists, its own vertical rhythm — to a
// place with room for none of it. Code spans are the only Markdown these
// strings actually use, so this handles exactly that and leaves everything else
// as the literal characters the author typed.

import { Fragment, type ReactNode } from "react";

/** Splits on paired backticks. An unpaired one is a literal backtick. */
const SPAN = /`([^`]+)`/g;

/**
 * `text` with its `code spans` rendered.
 *
 * Returns the string unchanged when it holds none, so the overwhelmingly common
 * case costs one regex test and no extra elements.
 */
export function inlineCode(text: string): ReactNode {
  if (!text.includes("`")) return text;
  const out: ReactNode[] = [];
  let last = 0;
  for (const match of text.matchAll(SPAN)) {
    const at = match.index ?? 0;
    if (at > last) out.push(text.slice(last, at));
    out.push(
      <code
        key={`${at}-${match[1]}`}
        className="rounded bg-muted px-1 py-0.5 font-mono text-xs"
      >
        {match[1]}
      </code>,
    );
    last = at + match[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out.map((piece, n) => <Fragment key={n}>{piece}</Fragment>);
}
