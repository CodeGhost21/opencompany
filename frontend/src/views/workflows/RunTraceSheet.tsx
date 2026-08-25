// The run traces list's transcript sheet (issue #1697): the full story of one
// run, read in place rather than by navigating into the graph editor. Composes
// two things that already exist rather than re-deriving either:
//
//  - `RunHistoryRow`, verbatim — the same status line, node trail, delivery
//    rows, and blocked/stranded/cancelled/error prose the run history rail
//    renders, so a run reads identically whether it's opened from a
//    workflow's own history or from the company-wide list.
//  - the run's per-node OUTPUT (issue #596's durable snapshot), which the row
//    above does not carry — `nodes[].status` is structural (ok/error/blocked,
//    elapsed), never the text a node actually produced.
//
// "Open in graph editor" is deliberately not a separate control: the row
// already offers "Show on canvas" when it has a node trail, and wiring that
// button's `onSelect` to navigate is the same affordance under the name this
// console already uses for it.

import { useEffect, useMemo, useState } from "react";

import { Markdown } from "@/components/markdown";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import type { OpenCompanyClient } from "@/api/client";
import {
  workflowRunOutput,
  type WorkflowRunOutcome,
  type WorkflowRunOutputRecord,
} from "@/api/workflows";
import { workflowHref } from "@/lib/task-output";

import { RunHistoryRow, useRunningClock } from "./RunHistoryPanel";
import { isRunning } from "./run-health";
import { parseRunNodes } from "./run-output";

/** One past run's output fetch, keyed by run id so a second sheet open for a
 * DIFFERENT run cannot paint a stale record over it while its own request is
 * still in flight. */
interface OutputFetch {
  runId: string;
  loading: boolean;
  record: WorkflowRunOutputRecord | null;
}

/** The transcript side sheet for one run out of the company-wide traces list.
 * `run: null` closes it — same convention as `TaskDetailView`'s `RunDrawer`. */
export function RunTraceSheet({
  client,
  company,
  run,
  workflowName,
  onClose,
}: {
  client: OpenCompanyClient;
  company: string | null;
  run: WorkflowRunOutcome | null;
  workflowName: string;
  onClose: () => void;
}) {
  const runId = run?.runId ?? null;
  const [output, setOutput] = useState<OutputFetch | null>(null);
  const now = useRunningClock(run !== null && isRunning(run));
  // Whether the run this sheet is showing has settled. `run` itself is a live
  // reference (`WorkflowsView` looks it up in `indexRuns` on every render), so
  // this flips from `false` to `true` in place as the SAME run finishes —
  // rather than only ever being read once at mount.
  const runSettled = run !== null && !isRunning(run);

  // Issue #596's durable snapshot. A run journaled before output capture, a
  // dry run (persists nothing), or a hard-aborted run 404s — settled to
  // `record: null`, which renders as an honest empty state rather than an
  // error.
  //
  // Deferred until the run SETTLES, and re-run when it does (issue #1697
  // review): the snapshot is written when the run finishes, so fetching it
  // while still running only ever finds a 404 — and without `runSettled` in
  // the dependency list, that miss would be cached until the sheet is closed
  // and reopened, even though the run went on to finish moments later.
  useEffect(() => {
    if (!runId || !runSettled) {
      setOutput(null);
      return;
    }
    let cancelled = false;
    setOutput({ runId, loading: true, record: null });
    workflowRunOutput(client, company, runId)
      .then((record) => {
        if (!cancelled) setOutput({ runId, loading: false, record });
      })
      .catch(() => {
        if (!cancelled) setOutput({ runId, loading: false, record: null });
      });
    return () => {
      cancelled = true;
    };
  }, [runId, runSettled, client, company]);

  // Memoized so a tick of `now` (the running clock above) cannot re-derive
  // this from scratch — it only has to change when the fetch actually lands a
  // new record.
  const nodeResults = useMemo(
    () =>
      output && output.runId === runId && output.record
        ? parseRunNodes({ nodes: output.record.nodes }, null)
        : null,
    [output, runId],
  );

  return (
    <Sheet open={run !== null} onOpenChange={(next) => !next && onClose()}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-lg"
        data-testid="run-trace-sheet"
      >
        {run && (
          <>
            <SheetHeader className="border-b">
              <SheetTitle className="truncate" title={workflowName}>
                {workflowName}
              </SheetTitle>
              <SheetDescription className="flex flex-wrap items-center gap-1.5 text-xs">
                <Badge variant="outline" className="font-normal">
                  {run.scheduled ? "Scheduled" : "Manual"}
                </Badge>
                <span>{new Date(run.atMillis).toLocaleString()}</span>
              </SheetDescription>
            </SheetHeader>
            <ScrollArea className="min-h-0 flex-1">
              <div className="space-y-4 px-4 pb-4">
                {/* The same row the history rail renders, its "Show on canvas"
                    button repurposed as a navigation rather than an in-place
                    toggle — this sheet has no canvas beside it to toggle.
                    `client`/`company` are forwarded so the row's lazy "Files
                    associated" disclosure works here too (issue #1684). */}
                <RunHistoryRow
                  client={client}
                  company={company}
                  run={run}
                  graph={null}
                  now={now}
                  selected={false}
                  onSelect={() => {
                    // The destination IS a canvas, so the sheet must not stay
                    // open over it — unlike the row click, which opens it.
                    onClose();
                    window.location.hash = workflowHref(run.workflowId, run.runId);
                  }}
                />

                <div className="space-y-2">
                  <p className="text-3xs font-medium uppercase tracking-wide text-muted-foreground">
                    Node output
                  </p>
                  <NodeOutputList
                    runId={run.runId}
                    runSettled={runSettled}
                    fetch={output}
                    nodeResults={nodeResults}
                  />
                </div>
              </div>
            </ScrollArea>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}

/** What each node in the run produced, read from the durable output snapshot —
 * the part of a transcript {@link RunHistoryRow} does not carry. */
function NodeOutputList({
  runId,
  runSettled,
  fetch,
  nodeResults,
}: {
  /** Absent on a run journaled before issue #371 minted a correlation id — no
   * output was ever captured for it, so there is nothing to fetch. */
  runId: string | undefined;
  /** Whether the run has finished. The durable snapshot is written when a run
   * settles, so fetching before then only ever finds a 404 — this renders an
   * honest "still going" state instead of a premature empty one. */
  runSettled: boolean;
  fetch: OutputFetch | null;
  nodeResults: ReturnType<typeof parseRunNodes>;
}) {
  if (!runId) {
    return (
      <p className="text-xs text-muted-foreground">
        This run predates output capture, so no node text was recorded.
      </p>
    );
  }
  if (!runSettled) {
    return (
      <p className="text-xs text-muted-foreground">
        Still running — output appears here once it finishes.
      </p>
    );
  }
  if (!fetch || fetch.runId !== runId || fetch.loading) {
    return <p className="text-xs text-muted-foreground">Loading output…</p>;
  }
  if (!fetch.record) {
    return (
      <p className="text-xs text-muted-foreground" data-testid="run-trace-output-empty">
        No output captured for this run — it may have been a test run, or a
        run this host aborted before it could persist one.
      </p>
    );
  }
  return (
    <div className="space-y-3" data-testid="run-trace-output">
      {fetch.record.partial && (
        <Badge
          variant="outline"
          className="border-status-blocked/40 bg-status-blocked-soft font-normal"
        >
          partial capture — run failed or blocked
        </Badge>
      )}
      {fetch.record.truncated && (
        <Badge
          variant="outline"
          className="border-status-blocked/40 bg-status-blocked-soft font-normal"
        >
          truncated — clipped to fit
        </Badge>
      )}
      {!nodeResults || nodeResults.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          This run's snapshot has no per-node output to show.
        </p>
      ) : (
        nodeResults.map((node) => (
          <div key={node.id} className="rounded-lg border bg-background/40 p-2">
            <p className="mb-1 text-2xs font-medium">{node.name}</p>
            {node.messages.length === 0 ? (
              <p className="text-2xs text-muted-foreground">No readable output.</p>
            ) : (
              node.messages.map((m, i) => (
                <div key={i} className={i > 0 ? "mt-2 border-t pt-2" : undefined}>
                  {m.agentRef && (
                    <p className="mb-1 text-3xs uppercase tracking-wide text-muted-foreground">
                      {m.agentRef}
                    </p>
                  )}
                  {m.text ? (
                    <Markdown className="text-xs">{m.text}</Markdown>
                  ) : (
                    <p className="text-xs text-muted-foreground">—</p>
                  )}
                </div>
              ))
            )}
          </div>
        ))
      )}
    </div>
  );
}
