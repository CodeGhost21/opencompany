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
