import type { TurnStep } from "@/api/types";

/** A live row: a {@link TurnStep} plus the transient key that pairs a result to
 * its call so the row flips `running → ok/error` in place. */
export type LiveRow = TurnStep & { toolCallId?: string };

/** The live-frame shape this fold reads — the fields common to `tool_call`,
 * `tool_result` and `thinking` on {@link CompanyStreamEvent}. */
export interface LiveFrame {
  type: "tool_call" | "tool_result" | "thinking";
  toolCallId?: string;
  label?: string;
  detail?: string;
  result?: string;
  status?: string;
  elapsedMs?: number;
}

/**
 * Folds one live frame into a turn's rows, or returns `null` to drop it.
 *
 * Extracted from `AppShell.onTurnEvent` so the two maps that hold live rows —
 * `liveStepsByThread` and `liveStepsByMessage` — fold identically. A second
 * copy is how the two would drift, and a drifted fold is invisible: both keep
 * rendering, just differently.
 *
 * It also exists so the test can call the rule instead of restating it. The
 * review on #2068 caught exactly that: `keyFor` duplicated `onTurnEvent`'s
 * conditional, so it would have kept passing through a regression in the branch
 * the shell actually runs.
 *
 * `null` rather than an unchanged array is the "drop this frame" answer, so a
 * caller can bail out of its `setState` with the previous object identity and
 * let React skip the re-render.
 */
export function foldLiveFrame(rows: readonly LiveRow[], frame: LiveFrame): LiveRow[] | null {
  const next = [...rows];
  if (frame.type === "tool_call") {
    const idx = frame.toolCallId
      ? next.findIndex((r) => r.toolCallId === frame.toolCallId)
      : -1;
    const row = {
      kind: "tool_call" as const,
      status: "running" as const,
      label: frame.label ?? "Working",
      toolCallId: frame.toolCallId,
    };
    if (idx >= 0) next[idx] = { ...next[idx], ...row };
    else next.push(row);
    return next;
  }
  if (frame.type === "tool_result") {
    let idx = frame.toolCallId
      ? next.findIndex((r) => r.toolCallId === frame.toolCallId)
      : -1;
    // A result whose call is not in these rows belongs to another bucket.
    // Dropping it is deliberate: adopting it would invent a row with no start,
    // and pairing it with an unrelated `running` row would mark the wrong call
    // finished. The keying above is what keeps this from happening.
    if (idx < 0 && frame.toolCallId) return null;
    if (idx < 0) idx = next.findIndex((r) => r.status === "running");
    const status = frame.status === "error" ? ("error" as const) : ("ok" as const);
    if (idx >= 0) {
      next[idx] = {
        ...next[idx],
        status,
        detail: frame.detail ?? next[idx].detail,
        // `result` is what came back — the summary `StepTimeline` renders under
        // the label. Carried for the same reason `detail` is: the live row and
        // the folded step it is replaced by should not say different amounts
        // about the same call. It was dropped while only the built-in harness
        // streamed (its rows lean on `detail`, derived from the arguments); an
        // ACP tool call carries its summary in `result` and nothing else, so a
        // dropped `result` is the whole of what the row could have said.
        result: frame.result ?? next[idx].result,
        elapsedMs: frame.elapsedMs,
      };
    } else {
      next.push({
        kind: "tool_call",
        status,
        label: frame.label ?? "Working",
        detail: frame.detail,
        result: frame.result,
        elapsedMs: frame.elapsedMs,
        toolCallId: frame.toolCallId,
      });
    }
    return next;
  }
  // The backend already coalesces a thinking run into one frame, so each
  // arrival is a distinct row (mirrors the folded "Thinking" step).
  next.push({ kind: "thinking", status: "ok", label: "Thinking" });
  return next;
}
