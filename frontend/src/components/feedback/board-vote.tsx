import { ChevronDown, ChevronUp } from "lucide-react";

import type { BoardItem } from "@/api/types";
import { nextVote } from "@/lib/feedback-board";
import { cn } from "@/lib/utils";

interface Props {
  item: BoardItem;
  /** Called with what the click means — a fresh vote, or `0` to retract. */
  onVote: (value: BoardItem["my_vote"]) => void;
  /** Disabled while a vote is in flight, so a double-click cannot race. */
  busy?: boolean;
}

/**
 * The score column: up, the running total, down.
 *
 * Clicking the arrow you already picked retracts your vote rather than casting
 * it again — the same rule the hub enforces — so the control is honest about
 * being a toggle instead of silently ignoring the second click.
 */
export function BoardVote({ item, onVote, busy = false }: Props) {
  return (
    <div className="flex w-10 shrink-0 flex-col items-center gap-0.5">
      <Arrow
        direction={1}
        active={item.my_vote === 1}
        busy={busy}
        label={item.my_vote === 1 ? "Take your upvote back" : "Upvote"}
        onClick={() => onVote(nextVote(item.my_vote, 1))}
      />
      <span
        className={cn(
          "text-sm font-semibold tabular-nums",
          item.my_vote === 1 && "text-primary",
          item.my_vote === -1 && "text-muted-foreground",
        )}
      >
        {item.score}
      </span>
      <Arrow
        direction={-1}
        active={item.my_vote === -1}
        busy={busy}
        label={item.my_vote === -1 ? "Take your downvote back" : "Downvote"}
        onClick={() => onVote(nextVote(item.my_vote, -1))}
      />
    </div>
  );
}

function Arrow({
  direction,
  active,
  busy,
  label,
  onClick,
}: {
  direction: 1 | -1;
  active: boolean;
  busy: boolean;
  label: string;
  onClick: () => void;
}) {
  const Icon = direction === 1 ? ChevronUp : ChevronDown;
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={active}
      title={label}
      disabled={busy}
      onClick={onClick}
      className={cn(
        "flex size-7 items-center justify-center rounded-lg text-muted-foreground transition-colors",
        "hover:bg-muted hover:text-foreground disabled:opacity-50",
        active && "bg-primary/10 text-primary hover:bg-primary/15 hover:text-primary",
      )}
    >
      <Icon className="size-4" />
    </button>
  );
}
