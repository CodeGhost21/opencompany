import { useEffect, useState } from "react";

import type { TurnStep } from "@/api/types";
import { cn } from "@/lib/utils";

/**
 * What a teammate wears while a turn is running (issue #787).
 *
 * ## It says what is happening, not something amusing
 *
 * This follows OpenHuman's handling rather than the rotating-word style: the
 * line is **derived from the run's real state**. When a step is in flight it
 * shows that step's own label — the one the host authored and the tool timeline
 * beside it already renders — with a pulsing status mark. When there is nothing
 * specific to say it falls back to a plain "Working…".
 *
 * The label is never re-derived here. `TurnStep.label` is the host's answer,
 * and a second phrasing computed console-side is exactly the drift #264
 * forbids: two surfaces describing one step differently.
 *
 * ## Ambiguity is answered with less, not with a guess
 *
 * OpenHuman's `workerStatusFromEntry` returns `undefined` for the ambiguous
 * case *"so the card stays label-only rather than render a misleading state"*.
 * Same rule here: with no running step — a turn that has not reported one yet,
 * or one whose steps have all settled while the reply is still being written —
 * this shows the generic "Working…" rather than naming the last thing it saw.
 * A stale step name would read as current, which is worse than saying little.
 *
 * ## The assistive text does not follow the label
 *
 * The visible line changes as steps come and go. The screen-reader string is
 * stable, because a live region re-announcing every step transition is noise
 * where one "a reply is coming" is the whole message.
 *
 * ## Reduced motion
 *
 * `prefers-reduced-motion: reduce` stops the pulse. The mark and the label
 * still say a turn is running; they stop moving to say it.
 */
export function WorkingIndicator({
  srLabel,
  steps,
  className,
}: {
  /** The stable assistive string, e.g. "Replying…". Never the step label. */
  srLabel: string;
  /**
   * The turn's steps so far, when this surface has them. The newest `running`
   * one names the line; absent or all-settled falls back to "Working…".
   */
  steps?: readonly TurnStep[];
  className?: string;
}) {
  const reduced = usePrefersReducedMotion();
  const running = runningStepLabel(steps);

  return (
    <span
      className={cn(
        "flex items-center gap-2 rounded-full bg-muted px-3 py-2 text-sm text-muted-foreground",
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "size-1.5 shrink-0 rounded-full bg-status-running",
          !reduced && "animate-pulse",
        )}
      />
      {/* `aria-hidden`, because the stable label below is what should be read. */}
      <span aria-hidden className="truncate">
        {running ?? GENERIC_LABEL}
      </span>
      <span className="sr-only">{srLabel}</span>
    </span>
  );
}

/** What the line says when no step is in flight. */
export const GENERIC_LABEL = "Working…";

/**
 * The label of the newest step still running, or `undefined`.
 *
 * Last rather than first: a turn's steps arrive in order, so the most recent
 * `running` entry is the one actually in flight. A settled step is never
 * named — see the ambiguity note on {@link WorkingIndicator}.
 */
export function runningStepLabel(steps?: readonly TurnStep[]): string | undefined {
  if (!steps?.length) return undefined;
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    const step = steps[i];
    if (step.status === "running") {
      const label = step.label?.trim();
      return label ? label : undefined;
    }
  }
  return undefined;
}

/**
 * Whether the viewer asked for reduced motion, kept live.
 *
 * Reads `false` where `matchMedia` is unavailable (jsdom without a stub, an
 * older embedded webview): a pulse for a viewer who never asked for stillness
 * is the lesser failure.
 */
function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    const mql = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    if (!mql) return;
    setReduced(mql.matches);
    const onChange = () => setReduced(mql.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);
  return reduced;
}
