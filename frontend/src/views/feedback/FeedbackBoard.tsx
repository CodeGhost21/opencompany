import { useCallback, useEffect, useRef, useState } from "react";
import { RefreshCw } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { BoardItem, BoardKind, BoardSort, BoardStatus, BoardVote } from "@/api/types";
import { BoardRow } from "@/components/feedback/board-row";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  BOARD_KIND_FILTERS,
  BOARD_SORTS,
  BOARD_STATUS_FILTERS,
  applyVote,
  isBoardUnavailable,
  matchesBoardFilters,
} from "@/lib/feedback-board";
import { cn } from "@/lib/utils";

/** One page. Matches the host's own default, so page 1 costs no query string. */
const PAGE_SIZE = 20;

/**
 * Value→label maps for the two filters.
 *
 * The `Select` trigger renders its *value* unless it is handed the labels, so
 * without these the closed control would read "feature" where the open menu
 * reads "Ideas" — the same mismatch the feedback form's `items` prop exists to
 * avoid.
 */
const KIND_LABELS = Object.fromEntries(
  BOARD_KIND_FILTERS.map((option) => [option.value, option.label]),
);
const STATUS_LABELS = Object.fromEntries(
  BOARD_STATUS_FILTERS.map((option) => [option.value, option.label]),
);

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /**
   * Bumped by the page when a local report is forwarded to the hub, so a
   * report that just became a board item appears without a manual refresh.
   */
  refreshKey: number;
  /**
   * Called once with `false` when this host turns out to have no board at all,
   * so the page can drop the whole section instead of showing an empty one.
   */
  onAvailability: (available: boolean) => void;
}

/**
 * The shared feedback board: what everyone has asked for, what it is worth to
 * them, and where it stands.
 *
 * The rows are not this host's — they live on the TinyHumans hub, and the host
 * proxies them (`GET .../feedback/board`). This is the same board OpenHuman's
 * console renders, which is the point: an operator here sees, votes on and
 * replies to the same asks as everyone else on the product, instead of filing
 * into a private list that looks like it goes nowhere.
 */
export function FeedbackBoard({ client, company, refreshKey, onAvailability }: Props) {
  const [items, setItems] = useState<BoardItem[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [gone, setGone] = useState(false);

  const [sort, setSort] = useState<BoardSort>("hot");
  const [kind, setKind] = useState<BoardKind | "all">("all");
  const [status, setStatus] = useState<BoardStatus | "all">("all");
  const [voting, setVoting] = useState<string | null>(null);

  // Every fetch stamps this; a response whose stamp is stale is dropped. Two
  // filter changes in quick succession would otherwise land out of order and
  // leave the board showing the *first* query's rows under the second's filters.
  const requestRef = useRef(0);
  const pageRef = useRef(1);

  const load = useCallback(
    async (page: number, append: boolean) => {
      const stamp = ++requestRef.current;
      setLoading(true);
      setError(null);
      try {
        const result = await client.feedbackBoard(
          {
            sort,
            kind: kind === "all" ? undefined : kind,
            status: status === "all" ? undefined : status,
            page,
            limit: PAGE_SIZE,
          },
          company,
        );
        if (stamp !== requestRef.current) return;
        pageRef.current = result.page;
        setTotal(result.total);
        setItems((prev) => (append ? [...prev, ...result.items] : result.items));
      } catch (err) {
        if (stamp !== requestRef.current) return;
        // "This instance has no board" is not a failure to report — it is a
        // surface that does not exist here.
        if (isBoardUnavailable(err)) {
          setGone(true);
          onAvailability(false);
          return;
        }
        setError(err instanceof ApiError ? err.message : "could not load the board");
      } finally {
        if (stamp === requestRef.current) setLoading(false);
      }
    },
    [client, company, sort, kind, status, onAvailability],
  );

  useEffect(() => {
    void load(1, false);
    // Invalidate any in-flight response on the way out, so a late answer cannot
    // write into a board that has since changed its query.
    return () => {
      requestRef.current += 1;
    };
  }, [load, refreshKey]);

  const vote = useCallback(
    async (item: BoardItem, value: BoardVote) => {
      // Fill the arrow now; the round trip only confirms it.
      const optimistic = applyVote(item, value);
      setItems((prev) => prev.map((row) => (row.id === item.id ? optimistic : row)));
      setVoting(item.id);
      try {
        const updated = await client.voteFeedbackBoard(item.id, value, company);
        setItems((prev) => prev.map((row) => (row.id === item.id ? updated : row)));
      } catch (err) {
        // Put the row back exactly as it was — a vote that silently did not
        // land is worse than one that visibly did not.
        setItems((prev) => prev.map((row) => (row.id === item.id ? item : row)));
        setError(err instanceof ApiError ? err.message : "that vote did not land");
      } finally {
        setVoting(null);
      }
    },
    [client, company],
  );

  const bumpComments = useCallback((id: string) => {
    setItems((prev) =>
      prev.map((row) =>
        row.id === id ? { ...row, comment_count: row.comment_count + 1 } : row,
      ),
    );
  }, []);

  if (gone) return null;

  const visible = items.filter((item) => matchesBoardFilters(item, kind, status));
  const hasMore = items.length < total;

  return (
    <section className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        {/* `h2`, one level under FeedbackView's `h1`: this board is the page's
            other top-level section, beside the "Flag something" card, not a
            subsection of anything (issue #1392). */}
        <h2 className="flex items-center gap-2 text-base font-semibold">
          What everyone's asking for
          {total > 0 && (
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
              {total}
            </span>
          )}
        </h2>

        <div className="inline-flex rounded-lg border bg-muted/40 p-0.5">
          {BOARD_SORTS.map((option) => (
            <button
              key={option.value}
              type="button"
              title={option.hint}
              aria-pressed={sort === option.value}
              onClick={() => setSort(option.value)}
              className={cn(
                "rounded-md px-3 py-1 text-xs font-medium transition-colors",
                sort === option.value
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={kind}
          onValueChange={(v) => setKind(v as BoardKind | "all")}
          items={KIND_LABELS}
        >
          <SelectTrigger size="sm" className="w-36" aria-label="Filter by type">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {BOARD_KIND_FILTERS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Select
          value={status}
          onValueChange={(v) => setStatus(v as BoardStatus | "all")}
          items={STATUS_LABELS}
        >
          <SelectTrigger size="sm" className="w-36" aria-label="Filter by status">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {BOARD_STATUS_FILTERS.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <Button
          variant="ghost"
          size="sm"
          onClick={() => void load(1, false)}
          disabled={loading}
          className="ml-auto"
        >
          <RefreshCw className={cn("size-3.5", loading && "animate-spin")} /> Refresh
        </Button>
      </div>

      {error && (
        <p className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">{error}</p>
      )}

      {loading && items.length === 0 ? (
        <ul className="space-y-2">
          {Array.from({ length: 4 }).map((_, i) => (
            <li key={i}>
              <Skeleton className="h-24 w-full rounded-xl" />
            </li>
          ))}
        </ul>
      ) : visible.length > 0 ? (
        <ul className="space-y-2">
          {visible.map((item) => (
            <BoardRow
              key={item.id}
              client={client}
              company={company}
              item={item}
              voting={voting === item.id}
              onVote={(value) => void vote(item, value)}
              onCommented={() => bumpComments(item.id)}
            />
          ))}
        </ul>
      ) : error ? null : (
        <p className="rounded-xl border border-dashed py-10 text-center text-sm text-muted-foreground">
          Nothing here yet. Flag something above and it lands on this board.
        </p>
      )}

      {hasMore && (
        <div className="flex justify-center">
          <Button
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void load(pageRef.current + 1, true)}
          >
            {loading ? "Loading…" : "Show more"}
          </Button>
        </div>
      )}
    </section>
  );
}
