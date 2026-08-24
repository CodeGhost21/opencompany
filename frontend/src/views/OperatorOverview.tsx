import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, ArrowRight, CircleAlert, Clock3, MessageSquare, ShieldCheck } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { listRuns, RUN_STATUS_LABEL, type RunSummary } from "@/api/runs";
import type { LocalScope } from "@/connections/types";
import type { CompanyFeed } from "@/hooks/use-company";
import { readOverviewVisit, writeOverviewVisit } from "@/lib/overview-visit";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  companyName: string;
  feed: Pick<CompanyFeed, "approvals" | "queue">;
  scope: LocalScope;
  /**
   * Bumped by the shell on every `run_status_changed` event — the same counter
   * the task-detail screen re-reads on (issue #1015). The run panels below
   * re-read when a live attempt parks or fails while this page stays open;
   * without it they would keep saying nothing stopped until the view remounted
   * or the page reloaded.
   */
  attemptEventTick?: number;
}

type RunLoad = "loading" | "ready" | "error";

/**
 * The operator's landing page (issue #1321).
 *
 * A graph remains available at `#/company/graph`; this page concentrates on
 * what needs a person, work that stopped, and durable failed runs since the
 * last time *this browser* opened it. The boundary is browser-local because
 * the host has no persisted company-wide event read cursor yet.
 */
export function OperatorOverview({
  client,
  company,
  companyName,
  feed,
  scope,
  attemptEventTick,
}: Props) {
  const [previousVisit] = useState(() => readOverviewVisit(scope));
  const [stoppedRuns, setStoppedRuns] = useState<RunSummary[]>([]);
  const [failedRuns, setFailedRuns] = useState<RunSummary[]>([]);
  const [runLoad, setRunLoad] = useState<RunLoad>("loading");

  /**
   * The two run reads this page makes, kept separate on purpose.
   *
   * "Work that stopped" wants the newest parked-or-failed attempts of either
   * kind; "Since you last opened" is a claim about *failures*. Mixing them in
   * one capped page would let a run of newer paused attempts push an older
   * failed attempt that finished after the previous visit out of the answer —
   * and the since-visit panel's empty state would then print "No failed
   * attempts were recorded" while one existed. Failures get a page of their
   * own, read at the host's default cap so the boundary filter below is
   * exhaustive over a real history rather than a sample of it.
   */
  const fetchRuns = useCallback(async () => {
    const [stopped, failed] = await Promise.all([
      listRuns(client, company, { status: ["failed", "paused"], limit: 12 }),
      listRuns(client, company, { status: ["failed"], limit: 50 }),
    ]);
    return { stopped, failed };
  }, [client, company]);

  useEffect(() => {
    let live = true;
    setRunLoad("loading");
    fetchRuns()
      .then(({ stopped, failed }) => {
        if (!live) return;
        setStoppedRuns(stopped);
        setFailedRuns(failed);
        setRunLoad("ready");
      })
      .catch(() => {
        if (live) setRunLoad("error");
      });
    return () => {
      live = false;
    };
  }, [fetchRuns]);

  // Issue #1015: re-read (silently — no loading flash) when the shell reports a
  // run status change while this page stays open. The initial-load effect above
  // owns the "loading"/"error" states; a re-read that fails keeps the last good
  // lists rather than dropping a settled page to the error state mid-view.
  const seenRunTick = useRef(attemptEventTick);
  useEffect(() => {
    if (attemptEventTick === undefined || attemptEventTick === seenRunTick.current) return;
    seenRunTick.current = attemptEventTick;
    let live = true;
    fetchRuns()
      .then(({ stopped, failed }) => {
        if (!live) return;
        setStoppedRuns(stopped);
        setFailedRuns(failed);
      })
      .catch(() => {
        /* the current lists stay; the next event or reload re-reads */
      });
    return () => {
      live = false;
    };
  }, [attemptEventTick, fetchRuns]);

  useEffect(() => {
    writeOverviewVisit(scope, Date.now());
  }, [scope]);

  const stopped = useMemo(
    () => stoppedRuns.filter((run) => run.status === "paused" || run.status === "failed"),
    [stoppedRuns],
  );
  const failuresSinceVisit = useMemo(
    () =>
      previousVisit === null
        ? []
        : failedRuns.filter(
            (run) =>
              run.status === "failed" &&
              run.finishedAtMillis !== undefined &&
              run.finishedAtMillis >= previousVisit,
          ),
    [previousVisit, failedRuns],
  );

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-6 overflow-auto p-5 sm:p-8" data-testid="operator-overview" data-tour="operator-overview">
      <header className="flex flex-col gap-4 border-b pb-6 sm:flex-row sm:items-end sm:justify-between">
        <div>
          <p className="text-sm text-muted-foreground">{companyName}</p>
          <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
          <p className="mt-1 text-sm text-muted-foreground">Start with the work that needs your judgment.</p>
        </div>
        <a href="#/chat" className="inline-flex items-center justify-center gap-2 rounded-md bg-primary px-3 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90">
          <MessageSquare className="size-4" aria-hidden /> Start a conversation
        </a>
      </header>

      <section aria-labelledby="overview-attention" className="rounded-xl border bg-card p-5 shadow-sm">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 id="overview-attention" className="font-semibold">Needs your attention</h2>
            <p className="mt-1 text-sm text-muted-foreground">Approvals are decisions the company cannot make for itself.</p>
          </div>
          <ShieldCheck className="size-5 text-status-blocked-text" aria-hidden />
        </div>
        <ApprovalSummary feed={feed} />
      </section>

      <div className="grid gap-6 lg:grid-cols-2">
        <section aria-labelledby="overview-stopped" className="rounded-xl border bg-card p-5 shadow-sm">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h2 id="overview-stopped" className="font-semibold">Work that stopped</h2>
              <p className="mt-1 text-sm text-muted-foreground">Paused and failed attempts that may need a closer look.</p>
            </div>
            <AlertTriangle className="size-5 text-status-failed-text" aria-hidden />
          </div>
          <RunRows state={runLoad} runs={stopped} empty="No work is paused or failed right now." />
        </section>

        <section aria-labelledby="overview-since" className="rounded-xl border bg-card p-5 shadow-sm">
          <div className="flex items-start justify-between gap-4">
            <div>
              <h2 id="overview-since" className="font-semibold">Since you last opened this browser</h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {previousVisit === null
                  ? "There is no earlier visit in this browser to compare yet."
                  : "Failed attempts recorded after the previous visit."}
              </p>
            </div>
            <Clock3 className="size-5 text-muted-foreground" aria-hidden />
          </div>
          {previousVisit === null ? (
            <p className="mt-5 text-sm text-muted-foreground">Future visits will compare against this one. Company-wide activity history is not stored by the host yet.</p>
          ) : (
            <RunRows state={runLoad} runs={failuresSinceVisit} empty="No failed attempts were recorded since the previous visit." />
          )}
        </section>
      </div>

      <p className="text-xs text-muted-foreground">
        Looking for the company&apos;s structure? <a className="underline-offset-2 hover:underline" href="#/company/graph">Open the knowledge graph</a>.
      </p>
    </div>
  );
}

function ApprovalSummary({ feed }: { feed: Pick<CompanyFeed, "approvals" | "queue"> }) {
  if (feed.queue === "loading") return <p className="mt-5 text-sm text-muted-foreground" aria-busy="true">Loading approvals…</p>;
  if (feed.queue === "error" && feed.approvals.length === 0) {
    return <p role="alert" className="mt-5 text-sm text-destructive">Couldn&apos;t read what needs your approval. Open Approvals to try again.</p>;
  }
  if (feed.approvals.length === 0) return <p className="mt-5 text-sm text-muted-foreground">Nothing is waiting for your approval.</p>;
  const count = feed.approvals.length;
  return (
    <div className="mt-5 flex items-center justify-between gap-3">
      <p className="text-sm font-medium">{count === 1 ? "1 decision is waiting" : `${count} decisions are waiting`}</p>
      <a href="#/approvals" className="inline-flex shrink-0 items-center gap-1 text-sm font-medium underline-offset-2 hover:underline">Review approvals <ArrowRight className="size-4" aria-hidden /></a>
    </div>
  );
}

function RunRows({ state, runs, empty }: { state: RunLoad; runs: RunSummary[]; empty: string }) {
  if (state === "loading") return <p className="mt-5 text-sm text-muted-foreground" aria-busy="true">Loading recent work…</p>;
  if (state === "error") return <p role="alert" className="mt-5 text-sm text-destructive">Couldn&apos;t read recent work from the company host.</p>;
  if (runs.length === 0) return <p className="mt-5 text-sm text-muted-foreground">{empty}</p>;
  return (
    <ul className="mt-5 divide-y">
      {runs.slice(0, 3).map((run) => (
        <li key={run.id} className="flex items-center justify-between gap-3 py-3 first:pt-0 last:pb-0">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{run.taskId ? `Task ${run.taskId}` : "Conversation work"}</p>
            <p className="text-xs text-muted-foreground">{RUN_STATUS_LABEL[run.status]}{run.error ? ` — ${run.error}` : ""}</p>
          </div>
          {run.taskId ? <a href={`#/tasks/${encodeURIComponent(run.taskId)}?run=${encodeURIComponent(run.id)}`} className="shrink-0 text-sm font-medium underline-offset-2 hover:underline">Open <ArrowRight className="inline size-3.5" aria-hidden /></a> : <CircleAlert className="size-4 shrink-0 text-muted-foreground" aria-label="No task is attached to this attempt" />}
        </li>
      ))}
    </ul>
  );
}
