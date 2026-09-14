import { Check } from "lucide-react";

import type { TopicStanding } from "@/lib/hive/episode";
import { TOPIC_CARRIED_CLASS, TOPIC_CHIP_CLASS, standingSummary } from "@/lib/hive/tone";
import { cn } from "@/lib/utils";

/**
 * One option on the floor.
 *
 * A third form, deliberately: identity is a tile with initials and status is a
 * pill with a dot, and the design system's rule is that those two never share a
 * shape. A topic is neither — it is *which* option, not who or what state — so it
 * takes a `#`-prefixed monospace chip that cannot be mistaken for either.
 *
 * A carried topic is the one place a status colour appears, and it still carries
 * a tick so the state does not rest on colour alone.
 */
export function TopicChip({
  topic,
  standing,
  quorum,
  onSelect,
  className,
}: {
  topic: string;
  standing?: TopicStanding;
  quorum?: number;
  onSelect?: (topic: string) => void;
  className?: string;
}) {
  const carried = standing?.carried ?? false;
  const grounded = standing
    ? standing.supporters.filter((id) => !standing.ungrounded.includes(id)).length
    : 0;
  const label = (
    <>
      {carried ? <Check aria-hidden className="size-3" /> : null}#{topic}
    </>
  );
  const chip = cn(carried ? TOPIC_CARRIED_CLASS : TOPIC_CHIP_CLASS, className);

  const title =
    standing && quorum !== undefined
      ? standingSummary(grounded, quorum, carried)
      : `Topic #${topic}`;

  if (!onSelect) {
    return (
      <span className={chip} title={title}>
        {label}
      </span>
    );
  }
  return (
    <button
      type="button"
      className={cn(chip, "hover:bg-accent")}
      title={title}
      onClick={() => onSelect(topic)}
    >
      {label}
    </button>
  );
}
