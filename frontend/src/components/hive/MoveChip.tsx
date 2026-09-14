import { hiveIcon } from "@/components/hive/icons";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { MoveKind } from "@/lib/hive/grammar";
import { MOVE_CHIP_CLASS, MOVE_MARKS } from "@/lib/hive/tone";
import { cn } from "@/lib/utils";

/**
 * What a turn did, as one chip beside its author.
 *
 * Every move takes the same neutral treatment and is told apart by its icon —
 * see `lib/hive/tone.ts` for why colour is not doing this work.
 *
 * A **demoted** chip is the same shape struck through: the host journals a move
 * a seat may not make with its leading `!` removed, so the line still says what
 * its author meant and deposits no trace. Showing that is the difference between
 * a desk whose grammar is wrong and a desk whose members are unhelpful.
 */
export function MoveChip({
  kind,
  demoted = false,
  className,
}: {
  kind: MoveKind;
  demoted?: boolean;
  className?: string;
}) {
  const mark = MOVE_MARKS[kind];
  const Icon = hiveIcon(mark.icon);
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span
            className={cn(MOVE_CHIP_CLASS, demoted && "line-through opacity-60", className)}
          />
        }
      >
        <Icon aria-hidden className="size-3" />
        {mark.label}
      </TooltipTrigger>
      <TooltipContent>
        {demoted
          ? `Tried to ${mark.label.toLowerCase()}, but this seat does not hold that move — the line was recorded without it.`
          : mark.hint}
      </TooltipContent>
    </Tooltip>
  );
}
