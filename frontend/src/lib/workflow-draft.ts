import type { WorkflowDraftFromDescription } from "@/api/workflows";

/**
 * The create-time copilot's draft, reduced to the three banner slots the
 * New-workflow dialog renders (issue #813).
 *
 * A pure reducer so the branch logic — a drafted graph shows a summary and any
 * host corrections; a one-off shows a reason — is unit-testable without standing
 * up the dialog. The dialog itself only wires these strings into `<Alert>`s.
 */
export interface DraftBanners {
  /** The summary line to show above the hydrated form; `null` for a one-off. */
  summary: string | null;
  /**
   * Host corrections the operator should see (issue #813) — e.g. "Assigned the
   * … step to teammate `qa_engineer`". Empty when the draft needed none, or when
   * the work was a one-off. Blank entries are dropped.
   */
  notes: string[];
  /** The "better done once" reason; `null` when a graph was drafted. */
  reason: string | null;
}

/**
 * Maps a copilot draft response onto its banner slots. A drafted graph
 * (`automatable` with a `workflow`) yields a summary plus any correction notes;
 * anything else yields the not-automatable reason, with a sensible default when
 * the host sent none.
 */
export function draftBanners(drafted: WorkflowDraftFromDescription): DraftBanners {
  if (drafted.automatable && drafted.workflow) {
    return {
      summary: drafted.summary
        ? `Drafted: ${drafted.summary} — review below, then Create.`
        : "Drafted — review below, then Create.",
      notes: (drafted.notes ?? []).filter((note) => note.trim().length > 0),
      reason: null,
    };
  }
  return {
    summary: null,
    notes: [],
    reason: drafted.reason ?? "This is better done once than built into a workflow.",
  };
}

/**
 * What a landed copilot draft may do to the form it came back to (issue #1052).
 *
 * - `drop` — the dialog moved on (closed, reopened, re-hydrated) while the
 *   request was in flight. The answer is about a form that no longer exists, so
 *   it is discarded silently: nothing was asked, so nothing needs explaining.
 * - `confirm` — the form holds work the operator did, so replacing it is a
 *   question, not a side effect.
 * - `apply` — nothing would be lost; hydrate.
 */
export type DraftLanding = "drop" | "confirm" | "apply";

/**
 * Decides what a draft response is allowed to do **at the moment it lands**.
 *
 * The bug this exists for: `runDraft` used to take its `window.confirm` *before*
 * the request. A model call takes seconds, and the operator is invited to keep
 * typing while it runs — so the consent was granted over a form that no longer
 * existed by the time the answer arrived, and a draft could replace work the
 * operator started *after* agreeing. Consent has to be taken against the state
 * being replaced, which means after the await, which means here.
 *
 * `epoch` is the identity of the dialog's contents. The reset effect bumps it on
 * every open, so a draft abandoned by cancelling cannot hydrate the *next* open —
 * the failure mode where reopening the dialog filled itself in with a draft the
 * operator had walked away from.
 *
 * Staleness is checked **before** dirtiness on purpose: a response for a form
 * that is gone must not raise a confirm about a form the operator is no longer
 * looking at. Asking a question nobody can place is worse than silence.
 *
 * Pure so the three outcomes can be proved without a slow request and a rendered
 * dialog — the same reason `draftBanners` above lives here rather than inline.
 */
export function draftLanding(args: {
  /** The dialog's epoch when the request was issued. */
  requestedEpoch: number;
  /** The dialog's epoch now that the response has landed. */
  currentEpoch: number;
  /** Whether the form holds operator work **right now**, not at request time. */
  dirtyNow: boolean;
}): DraftLanding {
  if (args.requestedEpoch !== args.currentEpoch) return "drop";
  return args.dirtyNow ? "confirm" : "apply";
}
