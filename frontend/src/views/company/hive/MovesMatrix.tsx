import { hiveIcon } from "@/components/hive/icons";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { DeskHiveSeatDto } from "@/api/types";
import { GATEABLE_KINDS, UNGATED_KINDS, type MoveKind } from "@/lib/hive/grammar";
import { MOVE_MARKS } from "@/lib/hive/tone";
import { cn } from "@/lib/utils";

/**
 * Which moves each seat may open a line with.
 *
 * The knob that turns a vote back into a deliberation. A room whose members may
 * all `!propose` produces independent proposals and a commit — which is what a
 * live six-seat run produced in every one of nine episodes — because a proposal
 * already counts as its own author's support, so agreement is reached without
 * anybody engaging with anybody else's reasoning.
 *
 * # Three seat states, not a boolean
 *
 * "Not named" and "named with an empty list" both mean *every move*, and the
 * host reads them identically — an empty list is a table somebody started and
 * never filled in far more often than it is a vow of silence. But only one of
 * them is a decision somebody made, so the matrix shows the difference: a seat
 * the table does not govern reads "every move", not nine ticked boxes that
 * imply somebody chose them.
 *
 * # The ungated three are shown, not hidden
 *
 * `commit`, `question` and `defer` can never be taken away, and rendering them
 * as absent would read as "this seat cannot commit". A six-member desk once
 * reached quorum, handed the Commit floor to three seats barred from
 * committing, and reported itself exhausted on an answer it had already
 * decided — so the column exists precisely to say it cannot happen.
 */
export function MovesMatrix({
  seats,
  moves,
  onToggle,
  disabled,
}: {
  seats: DeskHiveSeatDto[];
  /** The authored table. A seat absent from it is ungoverned. */
  moves: Record<string, string[]>;
  onToggle: (agentId: string, kind: MoveKind, next: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr>
            <th className="sticky left-0 bg-background py-2 pr-3 text-left text-xs font-medium text-muted-foreground">
              Seat
            </th>
            {GATEABLE_KINDS.map((kind) => (
              <th key={kind} className="px-1 pb-2 text-center">
                <MoveHeader kind={kind} />
              </th>
            ))}
            <th className="px-2 pb-2 text-center">
              <Tooltip>
                <TooltipTrigger
                  render={
                    <span className="text-2xs font-medium text-muted-foreground underline decoration-dotted" />
                  }
                >
                  always
                </TooltipTrigger>
                <TooltipContent>
                  {UNGATED_KINDS.join(", ")} belong to every seat however narrow its
                  entry. Recording a decision the room already carried re-derives
                  nothing, and a member with nothing to add must always have something
                  to say that is not prose.
                </TooltipContent>
              </Tooltip>
            </th>
          </tr>
        </thead>
        <tbody>
          {seats.map((seat) => {
            // The response describes the saved grammar, while `moves` is the
            // operator's draft. A first edit must be possible when neither has
            // a row yet: toggling then materialises the narrowed row.
            const governed =
              seat.governed || Object.prototype.hasOwnProperty.call(moves, seat.agentId);
            const held = new Set(moves[seat.agentId] ?? []);
            return (
              <tr key={seat.agentId} className="border-t border-border">
                <td className="sticky left-0 bg-background py-2 pr-3">
                  <div className="font-medium">{seat.label}</div>
                  <div className="text-2xs text-muted-foreground">
                    {governed ? seat.role : "every move — not narrowed"}
                  </div>
                </td>
                {GATEABLE_KINDS.map((kind) => (
                  <td key={kind} className="px-1 py-2 text-center">
                    <input
                      type="checkbox"
                      className="size-4 accent-primary"
                      aria-label={`${seat.label} may ${MOVE_MARKS[kind].label.toLowerCase()}`}
                      // An ungoverned seat holds everything, so its boxes are
                      // ticked — but the row says the table has not narrowed it,
                      // which is the fact the ticks alone cannot carry.
                      checked={governed ? held.has(kind) : true}
                      // An ungoverned seat's checked=true is not a value the
                      // operator can toggle away from here — the row does not
                      // narrow it — so the box must not accept a click that
                      // would fire `onToggle` for a seat this table does not
                      // govern.
                      disabled={disabled}
                      onChange={(e) => onToggle(seat.agentId, kind, e.target.checked)}
                    />
                  </td>
                ))}
                <td className="px-2 py-2 text-center text-2xs text-muted-foreground">
                  on
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function MoveHeader({ kind }: { kind: MoveKind }) {
  const mark = MOVE_MARKS[kind];
  const Icon = hiveIcon(mark.icon);
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span className={cn("flex flex-col items-center gap-0.5")} />}
      >
        <Icon aria-hidden className="size-3.5 text-muted-foreground" />
        <span className="text-3xs text-muted-foreground">{mark.label}</span>
      </TooltipTrigger>
      <TooltipContent>{mark.hint}</TooltipContent>
    </Tooltip>
  );
}
