// How the Workflows console reads a failed run POST (issues #528, #514).
//
// Pure and React-free so the triage can be unit-tested on its own: the
// `run()` catch in `WorkflowsView` is otherwise the only place this logic
// lives, and it is exactly the part where a wrong branch turns a fixable
// refusal into a vanishing toast, or a survivable dropped connection into a
// "the run failed" lie.

import { ApiError } from "@/api/types";

/**
 * The host refusal codes that mean "the run never started, and the operator can
 * fix it" (issue #514).
 *
 * `inference_required` — the company has never configured an inference source,
 * so there is no brain to run the workflow. `restart_required` — a saved
 * inference config is stranded behind a boot decision and a restart would pick
 * it up. Both are `409`s from the host's own `{error, code}` envelope.
 *
 * `not_wired` is DELIBERATELY absent. On a genuinely inference-less build the
 * host answers that as a `404`, and treating it as a run-refusal would tell the
 * operator to configure a provider the deployment cannot use — issue #514
 * exists precisely to split those two, so re-merging them here would undo it.
 * `lifecycle_conflict` (issue B-037) is also deliberately absent, for the
 * opposite reason to `not_wired`: it IS a `409` pre-execution refusal, but it
 * is not cleared from Settings → Inference, and this set exists to raise a
 * banner that points there. It classifies as `other`, which opens the failure
 * panel — and `failureDisposition` gives it a `refusal-lifecycle` arm so the
 * panel says the run never started and names the real fix.
 *
 * The set is keyed on the structured `code` ONLY; never string-match the prose.
 */
export const INFERENCE_RUN_CODES: ReadonlySet<string> = new Set([
  "inference_required",
  "restart_required",
]);

/**
 * What a rejected run POST actually was:
 *
 * * `refusal-inference` — the host considered the run and refused it for a
 *   reason the operator clears from Settings (no inference / restart owed). Gets
 *   a persistent banner, not a toast.
 * * `connection-lost` — nothing was refused; the connection dropped, but the run
 *   is still walking its graph on the host (issue #383's spawned server task
 *   survives a severed request). Only claimed when this view actually saw its
 *   own run's start frame, so it is honest about "still going".
 * * `other` — a real failure to report as-is.
 */
export type RunErrorClass = "refusal-inference" | "connection-lost" | "other";

/**
 * Classifies a rejected `runWorkflow` POST.
 *
 * `sawOwnRunStart` is whether a NON-scheduled `workflow_run_started` frame for
 * the workflow on screen has arrived since this run was dispatched — i.e. the
 * run reached the engine before the connection died. Without it a status-0 or
 * upstream-5xx failure is just a failure: we cannot promise "it continues on the
 * host" for a run we never saw begin.
 */
export function classifyRunError(e: unknown, sawOwnRunStart: boolean): RunErrorClass {
  if (!(e instanceof ApiError)) return "other";
  // Refusal first: it is the one branch keyed on a host envelope (`fromHost`)
  // AND a specific code, so it can never be shadowed by the transport checks.
  if (e.fromHost && INFERENCE_RUN_CODES.has(e.code)) return "refusal-inference";
  // A status-0 ApiError is a transport failure the client synthesised; a
  // `fromHost:false` non-zero status is a proxy/upstream that gave up (an HTML
  // 502/504 with no host envelope). Either way the host may still be running the
  // job — but only say so if we watched it start.
  if ((e.status === 0 || !e.fromHost) && sawOwnRunStart) return "connection-lost";
  return "other";
}
