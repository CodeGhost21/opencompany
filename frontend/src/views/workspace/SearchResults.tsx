// The workspace search hit list (issue #607).
//
// Presentational only: `WorkspaceView` owns the query, the debounce and the
// fetch, and this renders whatever came back. Split out of `WorkspaceView.tsx`
// because that file is already past 1,600 lines and a hit row has nothing to do
// with the tree, the editor or the migration banner it would otherwise sit
// among.
//
// It replaces the tree in the explorer pane while a search is active rather than
// opening a panel beside it: the two answer the same question ("which note do I
// want?") and showing both at once would leave the operator reading a tree that
// is not what they just asked for.

import { FileText, Folder, Loader2, Search } from "lucide-react";

import { formatBytes, highlightRuns, isBinary, originLabel, type SearchHit } from "@/api/workspace";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

interface Props {
  /** The query the results below answer — not the input's live value. */
  query: string;
  hits: SearchHit[];
  /** Matches before the host's limit, so a partial page can say so. */
  total: number;
  loading: boolean;
  error: string | null;
  onOpen: (hit: SearchHit) => void;
}

/**
 * One excerpt with the matched runs marked.
 *
 * `<mark>` rather than a styled `<span>`: the browser and every screen reader
 * already know what a mark means, and the whole point of the excerpt is to show
 * *why* this note came back.
 */
function Excerpt({ text, query }: { text: string; query: string }) {
  return (
    <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground" data-testid="workspace-search-excerpt">
      {highlightRuns(text, query).map((run, i) =>
        run.hit ? (
          <mark key={i} className="rounded-sm bg-highlight px-0.5 text-foreground">
            {run.text}
          </mark>
        ) : (
          <span key={i}>{run.text}</span>
        ),
      )}
    </p>
  );
}

export function SearchResults({ query, hits, total, loading, error, onOpen }: Props) {
  if (error) {
    return (
      <div className="px-2 py-2">
        <Alert variant="destructive">
          <AlertDescription data-testid="workspace-search-error">{error}</AlertDescription>
        </Alert>
      </div>
    );
  }

  if (loading && hits.length === 0) {
    return (
      <p className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" /> Searching…
      </p>
    );
  }

  if (hits.length === 0) {
    return (
      <p className="px-3 py-2 text-xs text-muted-foreground" data-testid="workspace-search-empty">
        No notes mention “{query}”. Search matches whole words and parts of words, but not
        misspellings — try a shorter or different term.
      </p>
    );
  }

  return (
    <div data-testid="workspace-search-results">
      <p className="px-3 py-1.5 text-2xs tracking-wide text-muted-foreground uppercase">
        {/* `total` rather than `hits.length`: the host caps the page, and a
            count that only described what fits would quietly claim a partial
            answer is the whole one. */}
        {hits.length === total
          ? `${total} ${total === 1 ? "match" : "matches"}`
          : `${hits.length} of ${total} matches`}
      </p>
      <ul>
        {hits.map((hit) => {
          const origin = originLabel(hit.updatedBy);
          return (
            <li key={hit.id}>
              <button
                type="button"
                onClick={() => onOpen(hit)}
                data-testid="workspace-search-hit"
                data-node-id={hit.id}
                className={cn(
                  "w-full px-3 py-2 text-left hover:bg-accent/60",
                  "border-b border-border/40 last:border-b-0",
                )}
              >
                <span className="flex items-center gap-1.5">
                  {hit.kind === "folder" ? (
                    <Folder className="size-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <FileText className="size-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span className="truncate text-sm">
                    {highlightRuns(hit.name, hit.matched === "name" ? query : "").map((run, i) =>
                      run.hit ? (
                        <mark
                          key={i}
                          className="rounded-sm bg-highlight px-0.5 text-foreground"
                        >
                          {run.text}
                        </mark>
                      ) : (
                        <span key={i}>{run.text}</span>
                      ),
                    )}
                  </span>
                  {origin && (
                    <Badge variant="outline" className="shrink-0 text-3xs">
                      {origin}
                    </Badge>
                  )}
                </span>
                {/* The path is the reason a flat hit list is usable at all — two
                    notes can share a name, and the tree that would have told
                    them apart is not on screen while a search is showing. */}
                <span className="mt-0.5 block truncate text-2xs text-muted-foreground">
                  {hit.path}
                  {isBinary(hit) && ` · ${hit.mime} · ${formatBytes(hit.size)}`}
                </span>
                {hit.excerpt && <Excerpt text={hit.excerpt} query={query} />}
              </button>
            </li>
          );
        })}
      </ul>
      {loading && (
        <p className="flex items-center gap-2 px-3 py-2 text-xs text-muted-foreground">
          <Search className="size-3.5" /> Updating…
        </p>
      )}
    </div>
  );
}
