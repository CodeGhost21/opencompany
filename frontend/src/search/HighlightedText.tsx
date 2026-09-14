// A matched string, with the matches marked.
//
// Its own component for one reason: the alternative everybody reaches for is
// building an HTML string with the term wrapped in `<mark>` and setting
// `dangerouslySetInnerHTML`, which puts whatever was typed — and whatever a
// teammate wrote in a message — into the DOM as markup. This takes offsets and
// slices the original string, so nothing is ever parsed as HTML.

import { Fragment } from "react";

import type { MatchRange } from "./types";

export function HighlightedText({
  text,
  ranges,
}: {
  text: string;
  /** Half-open `[start, end)` slices, in order and non-overlapping. */
  ranges?: readonly MatchRange[];
}) {
  if (!ranges || ranges.length === 0) return <>{text}</>;

  const parts: React.ReactNode[] = [];
  let at = 0;
  ranges.forEach(([from, to], index) => {
    // Defensive against an out-of-order or overlapping range: rendering the
    // string twice is a worse failure than dropping one highlight.
    if (from < at || to > text.length) return;
    if (from > at) parts.push(<Fragment key={`t${index}`}>{text.slice(at, from)}</Fragment>);
    parts.push(
      <mark key={`m${index}`} className="rounded-sm bg-transparent font-medium text-foreground">
        {text.slice(from, to)}
      </mark>,
    );
    at = to;
  });
  if (at < text.length) parts.push(<Fragment key="tail">{text.slice(at)}</Fragment>);

  return <>{parts}</>;
}
