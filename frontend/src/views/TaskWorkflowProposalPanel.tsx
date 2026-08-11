// Issue #580: the In-Review panel where an operator approves (or rejects) the
// workflow a builder pass proposed for a `workflow`-deliverable card.
//
// The road to #339's Done link. A card whose deliverable is `workflow`, dragged
// into In Progress, has the builder pass turn its plan into a proposed graph and
// settle it here — In Review, carrying a `TaskWorkflowProposal`. This panel is
// where a person reads that proposal and decides:
//
// * **Apply** runs the one workflow-authoring path host-side
//   (`POST …/workflow-proposal/apply`): it rebuilds the graph from the STORED
//   proposal, validates it exactly as a hand-authored create, links the card to
//   the created workflow, and returns it moved to Done — where #339's existing
//   output link opens the new workflow. A refused create (a name taken since, a
//   teammate off the roster) comes back rejected and the card stays In Review
//   with its proposal intact; the host's own message is shown verbatim.
// * **Reject** clears the proposal and returns the card to To-do (decision D2c),
//   keeping its `workflow` deliverable so dragging it back builds again.
//
// The diff is the #415 renderer, fed by the {@link taskProposalDiff} adapter —
// the same review an operator gets for a copilot's edit, so a built workflow is
// approved with machinery that already exists. When the adapter cannot render
// the stored graph, Apply is withheld (the host would refuse it too) and the
// reason is shown; Reject stays available so a card is never stuck.

import { useMemo, useState } from "react";
import { AlertTriangle, Check, Loader2, Workflow, X } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { applyWorkflowProposal, rejectWorkflowProposal, type Task } from "@/api/tasks";
import { ApiError } from "@/api/types";
import type { GraphDiff } from "@/api/workflow-proposal";
import { taskProposalDiff } from "@/lib/task-workflow-proposal";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ProposalDiff } from "@/views/workflows/ProposalDiff";

function generatedAt(atMillis: number): string {
  return new Date(atMillis).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function TaskWorkflowProposalPanel({
  client,
  company,
  task,
  onSaved,
  onReload,
}: {
  client: OpenCompanyClient;
  company: string | null;
  /** The In-Review card. The caller guarantees `task.workflowProposal` is set. */
  task: Task;
  /** Hand the settled card back to the board/detail for reconciliation. */
  onSaved: (t: Task) => void;
  /** Re-read the detail after a settle, so its poll picks up the new state. */
  onReload: () => Promise<void> | void;
}) {
  const proposal = task.workflowProposal!;
  const result = useMemo(() => taskProposalDiff(proposal.ops), [proposal.ops]);
  const [busy, setBusy] = useState<null | "apply" | "reject">(null);
  // Component-local so the parent's 4s detail poll cannot wipe it: a refused
  // Apply leaves the card In Review, the panel stays mounted, and the host's
  // reason must persist until the operator acts rather than blinking out on the
  // next re-render.
  const [error, setError] = useState<string | null>(null);

  const refused = "reason" in result;

  async function apply() {
    setBusy("apply");
    setError(null);
    try {
      const saved = await applyWorkflowProposal(client, company, task.id);
      onSaved(saved);
      await onReload();
      toast.success("Workflow created — the card is done.");
    } catch (e) {
      // The host's refusal, verbatim: a name taken since the pass ran, a
      // teammate no longer on the roster. The card stays In Review with its
      // proposal, so this is a state to recover from, not a dead end.
      setError(
        e instanceof ApiError
          ? e.message
          : e instanceof Error
            ? e.message
            : "The workflow could not be created.",
      );
    } finally {
      setBusy(null);
    }
  }

  async function reject() {
    setBusy("reject");
    setError(null);
    try {
      const saved = await rejectWorkflowProposal(client, company, task.id);
      onSaved(saved);
      await onReload();
      toast.success("Proposal rejected — the card is back in To-do.");
    } catch (e) {
      setError(e instanceof Error ? e.message : "The proposal could not be rejected.");
    } finally {
      setBusy(null);
    }
  }

  return (
    <div
      className="rounded-xl border bg-card p-4"
      data-testid="task-workflow-proposal"
    >
      <div className="flex items-start gap-2">
        <Workflow className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold leading-snug">{proposal.summary}</p>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            Proposed workflow · built {generatedAt(proposal.generatedAtMillis)}
          </p>
        </div>
      </div>

      {refused ? (
        // The adapter could not render the stored graph, which is what the host
        // would refuse on Apply too — so Apply is withheld rather than offered
        // and rejected. Reject stays, so the card is never stuck.
        <Alert className="mt-3 py-1.5">
          <AlertTriangle className="size-3.5" />
          <AlertDescription className="text-[11px] leading-snug">
            <span className="font-medium text-foreground">Apply withheld.</span>{" "}
            {(result as { reason: string }).reason} You can still reject it to send the card back to
            To-do and have the workflow built again.
          </AlertDescription>
        </Alert>
      ) : (
        <div className="mt-2 text-[11px] leading-snug">
          <ProposalDiff diff={(result as { diff: GraphDiff }).diff} />
        </div>
      )}

      {error && (
        <Alert variant="destructive" className="mt-3 py-1.5" data-testid="task-workflow-proposal-error">
          <AlertTriangle className="size-3.5" />
          <AlertDescription className="text-[11px] leading-snug">{error}</AlertDescription>
        </Alert>
      )}

      <div className="mt-3 flex items-center gap-2">
        {!refused && (
          <Button
            size="sm"
            className="h-8"
            disabled={busy !== null}
            onClick={() => void apply()}
            data-testid="task-workflow-proposal-apply"
          >
            {busy === "apply" ? (
              <Loader2 className="mr-1.5 size-3.5 animate-spin" />
            ) : (
              <Check className="mr-1.5 size-3.5" />
            )}
            Approve &amp; create
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          className="h-8"
          disabled={busy !== null}
          onClick={() => void reject()}
          data-testid="task-workflow-proposal-reject"
        >
          {busy === "reject" ? (
            <Loader2 className="mr-1.5 size-3.5 animate-spin" />
          ) : (
            <X className="mr-1.5 size-3.5" />
          )}
          Reject
        </Button>
      </div>
    </div>
  );
}
