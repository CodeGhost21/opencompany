import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError, type BoardComment } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { boardTimeMillis } from "@/lib/feedback-board";
import { timeAgo } from "@/lib/language";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  itemId: string;
  /** Called once a comment is stored, so the row's count can follow. */
  onPosted: () => void;
}

/**
 * The comment thread under one board row, fetched when the row is opened.
 *
 * Deliberately lazy: the list endpoint carries only a count, and fetching every
 * thread for every row on the board would be twenty round trips to render a
 * number the row already has.
 */
export function BoardComments({ client, company, itemId, onPosted }: Props) {
  const [comments, setComments] = useState<BoardComment[] | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setError(null);
    client
      .feedbackBoardItem(itemId, company)
      .then((detail) => live && setComments(detail.comments))
      .catch((err) => {
        if (!live) return;
        setComments([]);
        setError(err instanceof ApiError ? err.message : "could not load the replies");
      });
    return () => {
      live = false;
    };
  }, [client, company, itemId]);

  async function post() {
    const body = draft.trim();
    if (!body || busy) return;
    setBusy(true);
    setError(null);
    try {
      const stored = await client.commentFeedbackBoard(itemId, body, company);
      setComments((prev) => [...(prev ?? []), stored]);
      setDraft("");
      onPosted();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "could not post that");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-3 border-t pt-3">
      {comments === null ? (
        <p className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="size-3 animate-spin" /> Loading replies…
        </p>
      ) : comments.length === 0 ? (
        <p className="text-xs text-muted-foreground">No replies yet. Be the first.</p>
      ) : (
        <ul className="space-y-3">
          {comments.map((comment) => (
            <li key={comment.id} className="space-y-1">
              <p className="text-xs text-muted-foreground">
                {comment.author ?? "Someone"}
                {(() => {
                  const at = boardTimeMillis(comment.created_at);
                  return at === null ? null : ` · ${timeAgo(at, Date.now())}`;
                })()}
              </p>
              <p className="text-sm whitespace-pre-wrap">{comment.body}</p>
            </li>
          ))}
        </ul>
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}

      <div className="flex items-end gap-2">
        <Textarea
          aria-label="Add a reply"
          rows={2}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          placeholder="Add what you'd want out of this…"
          className="text-sm"
        />
        <Button size="sm" disabled={busy || !draft.trim()} onClick={() => void post()}>
          Reply
        </Button>
      </div>
    </div>
  );
}
