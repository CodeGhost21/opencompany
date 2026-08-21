import { useState } from "react";
import { Bug, ExternalLink, Lightbulb, MessageSquare } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { BoardItem, BoardStatus, BoardVote } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { BoardComments } from "@/components/feedback/board-comments";
import { BoardVote as BoardVoteControl } from "@/components/feedback/board-vote";
import { BOARD_KIND_LABELS, BOARD_STATUS_LABELS, boardTimeMillis } from "@/lib/feedback-board";
import { timeAgo } from "@/lib/language";
import { cn } from "@/lib/utils";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  item: BoardItem;
  /** Cast (or retract) a vote. The parent owns the optimistic update. */
  onVote: (value: BoardVote) => void;
  /** A reply landed — bump this row's count. */
  onCommented: () => void;
  /** True while this row's vote is in flight. */
  voting?: boolean;
}

/** How each status reads at a glance. `closed` is muted; `completed` is the win. */
const STATUS_STYLES: Record<BoardStatus, string> = {
  open: "bg-muted text-muted-foreground",
  planned: "bg-primary/10 text-primary",
  completed: "bg-status-done-soft text-status-done-text",
  closed: "bg-muted text-muted-foreground line-through decoration-1",
};

/**
 * One row on the shared board: score, what was asked for, where it stands, and
 * the replies — collapsed until asked for.
 */
export function BoardRow({ client, company, item, onVote, onCommented, voting }: Props) {
  const [open, setOpen] = useState(false);
  const KindIcon = item.kind === "bug" ? Bug : Lightbulb;
  const at = boardTimeMillis(item.created_at);

  return (
    <li className="flex gap-3 rounded-xl border bg-card p-3 transition-colors hover:border-foreground/15">
      <BoardVoteControl item={item} onVote={onVote} busy={voting} />

      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <p className="min-w-0 flex-1 text-sm font-medium break-words">{item.title}</p>
          <Badge variant="outline" className="gap-1">
            <KindIcon className="size-3" /> {BOARD_KIND_LABELS[item.kind]}
          </Badge>
          <Badge className={cn("border-transparent", STATUS_STYLES[item.status])}>
            {BOARD_STATUS_LABELS[item.status]}
          </Badge>
        </div>

        {item.body && (
          <p
            className={cn(
              "text-sm whitespace-pre-wrap text-muted-foreground",
              // Collapsed rows stay scannable; opening the replies opens the
              // full text too, because a truncated ask is a bad thing to reply to.
              !open && "line-clamp-2",
            )}
          >
            {item.body}
          </p>
        )}

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
          <span>{item.author ?? "Someone"}</span>
          {at !== null && <span>{timeAgo(at, Date.now())}</span>}
          <button
            type="button"
            onClick={() => setOpen((wasOpen) => !wasOpen)}
            aria-expanded={open}
            className="inline-flex items-center gap-1 font-medium hover:text-foreground"
          >
            <MessageSquare className="size-3" />
            {item.comment_count === 0
              ? "Reply"
              : `${item.comment_count} ${item.comment_count === 1 ? "reply" : "replies"}`}
          </button>
          {item.issue_url && (
            <a
              href={item.issue_url}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 font-medium underline underline-offset-4 hover:text-foreground"
            >
              Tracking issue <ExternalLink className="size-3" />
            </a>
          )}
        </div>

        {open && (
          <BoardComments
            client={client}
            company={company}
            itemId={item.id}
            onPosted={onCommented}
          />
        )}
      </div>
    </li>
  );
}
