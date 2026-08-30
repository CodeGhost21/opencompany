// The one step in `Move to…` that stops and says something (issue #1465).
//
// `secrets/` is the only part of the workspace tree the company's agents cannot
// reach, and `Move to…` was the only control that changed a note's membership
// of it — in both directions, on a single click, with a plain "moved" toast
// either way. The tree marker added by the same issue is passive; this is the
// moment the audience actually changes, so this is where the sentence belongs.
//
// Split out of `WorkspaceView.tsx` for the reason `SearchResults` was: that file
// is past 2,300 lines and a confirmation panel has nothing to do with the tree
// or the editor it would otherwise sit among. Presentational only — it renders
// the `MoveAudienceWarning` it is handed and reports the two buttons.

import { EyeOff, TriangleAlert } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import type { MoveAudienceWarning } from "@/lib/workspace";

export function MoveAudienceConfirm({
  warning,
  onCancel,
  onConfirm,
}: {
  warning: MoveAudienceWarning;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  // The two directions are not the same event. Moving a note *into* `secrets/`
  // is almost always what the operator meant, and the panel is there to confirm
  // it worked rather than to talk them out of it. Moving one *out* publishes
  // whatever is in it to every agent in the company, and cannot be un-published
  // — so that one wears the destructive treatment, on the alert and on the
  // button alike.
  const exposing = warning.change === "exposed";

  return (
    <div className="space-y-4" data-testid="workspace-move-audience">
      <Alert
        variant={exposing ? "destructive" : "default"}
        data-audience-change={warning.change}
      >
        {exposing ? <TriangleAlert /> : <EyeOff />}
        <AlertTitle>{warning.title}</AlertTitle>
        <AlertDescription>{warning.body}</AlertDescription>
      </Alert>
      <DialogFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        {/* The confirm never reads a bare "Move": the button is the last thing
            read before the click, so it repeats the direction rather than
            relying on the operator having read the paragraph above it. */}
        <Button
          variant={exposing ? "destructive" : "default"}
          onClick={onConfirm}
          data-testid="workspace-move-audience-confirm"
        >
          {warning.confirmLabel}
        </Button>
      </DialogFooter>
    </div>
  );
}
