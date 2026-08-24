/**
 * One attempt, collapsed to a header until a reader opens it.
 *
 * Collapsed by default and auto-open on trouble — the rule `StepTimeline`
 * already applies one surface over. A run with five agents and fifteen turns
 * each is unreadable fully expanded, and the turns worth reading first are the
 * ones that failed or are waiting on somebody.
 */

import { useState } from "react";

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
  /** Scroll target: the step named by the address, when this attempt owns it. */
  focusStep?: number | null;
}

export function AttemptCard({ run, nowMs, focusStep }: Props) {
  const [open, setOpen] = useState(() => opensItself(run) || focusStep !== null);
  const state = runState(run);
  const elapsed =
    (run.finishedAtMillis ?? nowMs) - (run.startedAtMillis ?? run.createdAtMillis);
  const tokens =
    run.usage.inputTokens + run.usage.outputTokens + run.usage.cachedInputTokens;

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
          <Badge variant="secondary" className="shrink-0 text-[10px]">
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
                <StepRow key={step.seq} step={step} />
              ))}
            </ul>
          )}
          <p className="text-muted-foreground border-border/60 border-t px-3 py-1.5 text-[10px]">
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
