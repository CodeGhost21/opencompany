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

import { EyeOff, FileText, Folder, Loader2, Lock, Search } from "lucide-react";

import { formatBytes, highlightRuns, isBinary, originLabel, type SearchHit } from "@/api/workspace";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  DERIVED_LABEL,
  DERIVED_REASON,
  isDerivedPath,
  isSecretPath,
  SECRETS_LABEL,
  SECRETS_REASON,
} from "@/lib/workspace";

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
          /**
           * Whether this hit is a ledger's file (issue #1377).
           *
           * Read off `hit.path` rather than the tree, because a search replaces
           * the tree in this pane — and the hits may name notes in folders the
           * explorer has never expanded, so there is no ancestry here to walk.
           */
          const derived = isDerivedPath(hit.path);
          /**
           * Whether this hit names a note the company's agents cannot read
           * (issue #1465).
           *
           * Operator search is deliberately unfiltered — `search_workspace`
           * keeps the whole tree while `search_workspace_for_agent` drops
           * `secrets/` — which is right, and is also why a private note used to
           * come back in this list looking exactly like a shared one. A console
           * on a shared screen leaked it.
           *
           * The sibling of `derived` above and never true at the same time: a
           * path has one first segment, and the two rules read that same
           * segment against different names.
           */
          const secret = isSecretPath(hit.path);
          // `Seeded` on a derived hit is not merely uninformative, it is
          // wrong — see the note above `Authorship` in `WorkspaceView`. The
          // badge slot says the true thing instead of the false one.
          //
          // A `secrets/` hit is the other way round and keeps its origin badge:
          // a `Seeded` note under `secrets/` really was seeded (the host lays
          // down one README there on first boot), so both badges are true and
          // say different things (issue #1465).
          const origin = derived ? null : originLabel(hit.updatedBy);
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
                    them apart is not on screen while a search is showing.

                    It is also where the folder badge goes, rather than up
                    beside the name where `Seeded` used to sit. "Written by a
                    ledger" and "Hidden from agents" are both wide, the pane is
                    256px, and on the name line either truncated the very thing
                    the operator is scanning for — `DECISIONS.md` became
                    `DECISIONS…`. Down here it costs no name width and lands
                    next to its own evidence, the `derived/` or `secrets/`
                    segment it explains. The scan-for-glyphs affordance lives in
                    the tree, which is the surface people browse. */}
                <span className="mt-0.5 flex items-center gap-1.5 text-2xs text-muted-foreground">
                  {/* `truncate` is doing two jobs, and the second one is easy
                      to mistake for missing. It carries `overflow: hidden`,
                      which makes this flex item a scroll container — and a
                      scroll container's automatic minimum size is 0, not its
                      min-content width. So the path already shrinks and
                      ellipsises rather than shouldering the badge out of a
                      256px pane; a `min-w-0` beside it would be a no-op.
                      Measured: with `truncate` the computed floor is `0px` and
                      the badge overflows by 0; strip `truncate` and the floor
                      is `auto` and it overflows. Keep them together. */}
                  <span className="truncate">
                    {hit.path}
                    {isBinary(hit) && ` · ${hit.mime} · ${formatBytes(hit.size)}`}
                  </span>
                  {derived && (
                    <Badge
                      variant="outline"
                      className="shrink-0 gap-1 pl-1 text-3xs font-normal"
                      title={DERIVED_REASON}
                      data-testid="workspace-search-derived"
                    >
                      <Lock className="size-2.5" aria-hidden />
                      {DERIVED_LABEL}
                    </Badge>
                  )}
                  {secret && (
                    <Badge
                      variant="outline"
                      className="shrink-0 gap-1 pl-1 text-3xs font-normal"
                      title={SECRETS_REASON}
                      data-testid="workspace-search-secret"
                    >
                      {/* An eye-off rather than a lock: a lock in this console
                          means `derived/`, "you may not write this", and this
                          rule is the other one. */}
                      <EyeOff className="size-2.5" aria-hidden />
                      {SECRETS_LABEL}
                    </Badge>
                  )}
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
