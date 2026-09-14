// One row in the results list.
//
// Separated from the dialog so the row's anatomy — what a channel says versus
// what a message says — can be read, changed and tested without mounting a
// modal, a client or a keyboard.

import { FileText, Hash, MessageSquare, User } from "lucide-react";

import { cn } from "@/lib/utils";
import { HighlightedText } from "./HighlightedText";
import type { ResultKind, SearchResult } from "./types";

/** The mark each kind wears, so a mixed list is scannable by shape. */
const ICON: Record<ResultKind, typeof Hash> = {
  channel: Hash,
  agent: User,
  message: MessageSquare,
  file: FileText,
};

export function SearchResultRow({
  result,
  active,
  onActivate,
  onHover,
}: {
  result: SearchResult;
  /** Whether the keyboard cursor is on this row. */
  active: boolean;
  onActivate: () => void;
  onHover: () => void;
}) {
  const Icon = ICON[result.kind];
  return (
    <button
      type="button"
      role="option"
      aria-selected={active}
      data-testid={`search-result-${result.id}`}
      data-active={active || undefined}
      onClick={onActivate}
      // Pointer moves the cursor rather than fighting it: with both a mouse and
      // arrow keys live, whichever moved last is the one the operator meant.
      onMouseMove={onHover}
      className={cn(
        "flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors",
        active ? "bg-accent" : "hover:bg-accent/50",
      )}
    >
      <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="truncate text-sm font-medium">
            <HighlightedText text={result.title} ranges={result.titleMatches} />
          </span>
          {result.subtitle && (
            <span className="truncate text-xs text-muted-foreground">{result.subtitle}</span>
          )}
        </span>
        {result.excerpt && (
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
            <HighlightedText text={result.excerpt} ranges={result.excerptMatches} />
          </span>
        )}
      </span>
      {/* What Enter does, spelled out on the row the cursor is on — the one
          place a fused palette is genuinely ambiguous. */}
      {active && (
        <span className="shrink-0 self-center text-2xs text-muted-foreground">{result.action}</span>
      )}
    </button>
  );
}
