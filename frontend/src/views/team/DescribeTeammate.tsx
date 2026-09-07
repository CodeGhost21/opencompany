import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

/**
 * The reduced Add-teammate dialog's whole content (issue #1989): a name and one
 * box.
 *
 * Shared by both Add-teammate dialogs — the roster grid's
 * (`TeamView.AddMemberDialog`, which also offers a budget) and the chat/org
 * chart's (`views/chat/AddMemberDialog`) — because the reduction is the same
 * reduction, and two copies of it would drift the way the two full forms
 * already have. What each dialog keeps to itself is what it does with the
 * result: the org chart also places the teammate on a desk, and the roster grid
 * also refetches.
 *
 * ## Why a Name field beside the box, when the brief said one box
 *
 * Because nothing can derive a name. The copilot's `DraftableField` is
 * `description | instructions` and excludes `name` deliberately, so there is no
 * model in this path to name anybody; the only alternative is splitting the
 * sentence, and a teammate's name is not a phrase. "Runs paid acquisition"
 * would then be the name on every roster card, in every member list, and beside
 * every message that teammate sends. The role is not split out of the sentence
 * either — it is designed from it, by the model, in the same pass as the
 * mandate and the persona. See `team-add-surface.ts` for why a split cannot do
 * that job at all.
 *
 * ## Why there is no copilot control in here
 *
 * Because the copilot is not a control on this dialog — it is what Create does.
 * `POST {scope}/team/design` turns this box into a role, a mandate and a
 * persona in one pass, and the teammate is written from that; there is no field
 * for an operator to ask about separately, because there is only one box and it
 * is theirs. The redirect afterwards lands them on all three fields, editable,
 * so a designed role is read before it can matter — which is the whole reason
 * the redirect is load-bearing rather than a nicety.
 */
export function DescribeTeammate({
  idPrefix,
  name,
  description,
  onNameChange,
  onDescriptionChange,
  disabled,
}: {
  /** Namespaces the DOM ids, so two of these can be mounted at once. */
  idPrefix: string;
  name: string;
  description: string;
  onNameChange: (value: string) => void;
  onDescriptionChange: (value: string) => void;
  /** Held while a create is in flight. */
  disabled?: boolean;
}) {
  const nameId = `${idPrefix}-describe-name`;
  const descriptionId = `${idPrefix}-describe-text`;
  return (
    <div className="grid gap-4">
      <div className="grid gap-2">
        <Label htmlFor={nameId}>Name</Label>
        <Input
          id={nameId}
          value={name}
          disabled={disabled}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder="e.g. Nova"
          data-testid="team-describe-name"
        />
      </div>
      <div className="grid gap-2">
        <Label htmlFor={descriptionId}>What should they do?</Label>
        <Textarea
          id={descriptionId}
          rows={4}
          value={description}
          disabled={disabled}
          onChange={(e) => onDescriptionChange(e.target.value)}
          placeholder="e.g. Runs paid acquisition and reports on ROAS every week."
          data-testid="team-describe-box"
        />
        {/* Says what actually happens next, because the fields this dialog
            stopped asking for have not gone away — the copilot writes them from
            this sentence before the teammate is created, and the operator lands
            on them. An earlier wording promised a copilot that "can draft their
            instructions" on the page they land on, which was true only in the
            sense that a button was enabled: nothing drafted until they noticed
            it and prompted it themselves, and until they did, the teammate held
            no persona at all. */}
        <p className="text-2xs text-muted-foreground" data-testid="team-describe-hint">
          The copilot writes their role, what they do and their instructions from
          this, then opens their profile so you can change any of it.
        </p>
      </div>
    </div>
  );
}
