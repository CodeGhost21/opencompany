import { typingLabel } from "@/lib/awareness";
import { cn } from "@/lib/utils";

/**
 * "Jane is typing…", under the composer.
 *
 * Deliberately a separate component from `WorkingIndicator`, which says an
 * *agent* is running a turn. The two look similar and mean different things —
 * one is a person at a keyboard, the other is a machine mid-task — and
 * collapsing them would make both less informative.
 *
 * Renders nothing at all when nobody is typing, rather than an empty reserved
 * row: the composer is at the bottom of a scrolling pane, and a row that
 * appears and disappears there pushes the transcript. Reserving the space
 * permanently costs a line of every conversation to say nothing.
 */
export function TypingLine({
  names,
  className,
}: {
  names: string[];
  className?: string;
}) {
  const label = typingLabel(names);
  if (!label) return null;
  return (
    <p
      data-testid="typing-line"
      aria-live="polite"
      className={cn(
        "px-4 pb-1 text-xs text-muted-foreground",
        className,
      )}
    >
      {label}
    </p>
  );
}
