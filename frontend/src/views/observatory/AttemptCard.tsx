/**
 * One attempt, collapsed to a header until a reader opens it.
 *
 * Collapsed by default and auto-open on trouble — the rule `StepTimeline`
 * already applies one surface over. A run with five agents and fifteen turns
 * each is unreadable fully expanded, and the turns worth reading first are the
 * ones that failed or are waiting on somebody.
 */

import { useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { formatDuration, relativeTime } from "@/views/workflows/run-health";
import { stepTotal, type ObservatoryRun } from "@/api/observatory";
import { runState } from "./model";
import { StepRow } from "./StepRow";

const TONE = {
  done: "border-l-[var(--status-done)]",
  failed: "border-l-[var(--status-failed)]",
  blocked: "border-l-[var(--status-blocked)]",
  running: "border-l-[var(--status-running)]",
} as const;

/** Whether this attempt should start open. */
function opensItself(run: ObservatoryRun): boolean {
  const state = runState(run);
  return state === "failed" || state === "blocked";
}

interface Props {
  run: ObservatoryRun;
  nowMs: number;
  /** The attempt a deep link names, when this card owns it. */
  turn?: string | null;
  /**
   * The step a deep link names, scrolled to within the named attempt. Carried
   * only by the card `turn` matches — a `step` must never open every card.
   */
  focusStep?: number | null;
  /**
   * Fired when the card opens — including the cards that start open (a deep
   * link's turn, an auto-opened failure) — so the parent can fetch the
   * unredacted half only for attempts a reader actually opened. Fires once per
   * open, again on each reopen, and again whenever an already-open attempt's
   * step trace grows, so a live attempt's new steps are not left with empty
   * deep panes until the reader closes and reopens the card.
   */
  onOpen?: (runId: string) => void;
}

export function AttemptCard({ run, nowMs, turn, focusStep, onOpen }: Props) {
  const focused = turn === run.id;
  const state = runState(run);
  const [open, setOpen] = useState(() => opensItself(run) || focused);
  const previousState = useRef(state);
  useEffect(() => {
    if (
      (previousState.current === "running" || previousState.current === "done") &&
      (state === "failed" || state === "blocked")
    ) {
      setOpen(true);
    }
    previousState.current = state;
  }, [state]);
  // `onOpen` rides an effect rather than the click handler so the cards that
  // start open and the cards that auto-open on trouble announce themselves too.
  // The step length is a dependency on purpose: while a live attempt's trace
  // grows between polls, an open card must re-read the unredacted half or the
  // new steps would keep empty deep panes until it is closed and reopened. The
  // length is the cheap signal — a step status transition alone does not re-read.
  useEffect(() => {
    if (open) onOpen?.(run.id);
  }, [open, run.id, run.steps.length, onOpen]);
  const elapsed =
    (run.finishedAtMillis ?? nowMs) - (run.startedAtMillis ?? run.createdAtMillis);
  // `cachedInput` is a subset of `input` (prompt_tokens_details.cached_tokens),
  // so it is not added here — the canonical metering total is input + output.
  const tokens = run.usage.inputTokens + run.usage.outputTokens;

  return (
    <div className={cn("bg-card rounded border border-l-2", TONE[state])}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="hover:bg-muted/40 flex w-full items-baseline gap-2 px-3 py-2 text-left"
      >
        <span className="text-muted-foreground w-3 shrink-0 text-xs">
          {open ? "▾" : "▸"}
        </span>
        <span className="truncate text-sm font-medium">{run.agentId}</span>
        {run.nodeId && (
          <Badge variant="secondary" className="shrink-0 text-3xs">
            {run.nodeId}
          </Badge>
        )}
        <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs">
          {run.status}
          {run.attempt > 1 && ` · attempt ${run.attempt}`}
        </span>
        <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
          {/* `stepTotal` rather than `stepCount`: the settled total is null
              while an attempt is live, and a live attempt is the one somebody
              is most likely watching. */}
          {stepTotal(run)} steps
        </span>
        <span className="text-muted-foreground shrink-0 text-xs tabular-nums">
          {formatDuration(elapsed)}
        </span>
        {tokens > 0 && (
          <span className="text-muted-foreground hidden shrink-0 text-xs tabular-nums sm:inline">
            {tokens.toLocaleString()} tok
          </span>
        )}
        {run.usage.costUsd > 0 && (
          <span className="text-muted-foreground hidden shrink-0 text-xs tabular-nums sm:inline">
            ${run.usage.costUsd.toFixed(3)}
          </span>
        )}
      </button>

      {open && (
        <div className="border-border/60 border-t">
          {run.error && (
            <p className="text-[var(--status-failed-text)] px-3 py-2 text-xs">
              {run.error}
            </p>
          )}
          {run.steps.length === 0 ? (
            <p className="text-muted-foreground px-3 py-3 text-xs">
              {/* Zero steps is meaningful, not missing: a memory-served or
                  tool-less turn genuinely did nothing worth recording. */}
              No steps recorded — a tool-less turn, or one served from memory.
            </p>
          ) : (
            <ul className="flex flex-col">
              {run.steps.map((step) => (
                <StepRow
                  key={step.seq}
                  step={step}
                  focus={focused && focusStep === step.seq}
                />
              ))}
            </ul>
          )}
          <p className="text-muted-foreground border-border/60 border-t px-3 py-1.5 text-3xs">
            started {relativeTime(run.startedAtMillis ?? run.createdAtMillis)}
            {" · "}
            in {run.usage.inputTokens.toLocaleString()} · out{" "}
            {run.usage.outputTokens.toLocaleString()} · cached{" "}
            {run.usage.cachedInputTokens.toLocaleString()}
          </p>
        </div>
      )}
    </div>
  );
}
