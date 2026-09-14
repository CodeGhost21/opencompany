import { EyeOff, Hand, ShieldX } from "lucide-react";

import { TopicChip } from "@/components/hive/TopicChip";
import type { Episode } from "@/lib/hive/episode";
import { standingSummary } from "@/lib/hive/tone";
import { cn } from "@/lib/utils";

/**
 * Where every option stands, for the room currently on screen.
 *
 * The transcript says what was said; this says what it added up to. Without it an
 * operator has to hold six turns of citations in their head to answer "is this
 * decided yet", which is the question they actually came with.
 *
 * Collapses to nothing when the room put nothing on the floor — an episode of
 * questions and deferrals has no standings, and an empty rail claiming otherwise
 * would be furniture.
 */
export function StandingsRail({
  episode,
  onSelectTopic,
  className,
}: {
  episode: Episode;
  onSelectTopic?: (topic: string) => void;
  className?: string;
}) {
  if (episode.topics.length === 0) return null;

  return (
    <aside className={cn("rounded-lg border border-border bg-card p-3", className)}>
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          On the floor
        </h3>
        <span className="text-2xs text-muted-foreground">
          quorum {episode.quorum}
          {episode.quorumDerived ? " (derived)" : ""}
        </span>
      </div>

      <ul className="mt-2 space-y-2">
        {episode.topics.map((standing) => {
          const grounded = standing.supporters.filter(
            (id) => !standing.ungrounded.includes(id),
          );
          return (
            <li key={standing.id} className="space-y-1">
              <div className="flex items-center gap-1.5">
                <TopicChip
                  topic={standing.id}
                  standing={standing}
                  quorum={episode.quorum}
                  onSelect={onSelectTopic}
                />
                <span className="text-2xs text-muted-foreground">
                  {standingSummary(grounded.length, episode.quorum, standing.carried)}
                </span>
              </div>

              {grounded.length > 0 && (
                <p className="pl-1 text-2xs text-muted-foreground">
                  Backed by {grounded.join(", ")}
                </p>
              )}

              {/*
                Each of the three below is a fact the transcript contains and a
                reader would otherwise have to reconstruct by hand. They are the
                reason the rail exists rather than a supporter count.
              */}
              {standing.ungrounded.length > 0 && (
                <p className="flex items-center gap-1 pl-1 text-2xs text-muted-foreground">
                  <EyeOff aria-hidden className="size-3" />
                  {standing.ungrounded.join(", ")} backed it without citing anything
                </p>
              )}
              {standing.silenced.length > 0 && (
                <p className="flex items-center gap-1 pl-1 text-2xs text-muted-foreground">
                  <Hand aria-hidden className="size-3" />
                  {standing.silenced.join(", ")} silenced by an objection
                </p>
              )}
              {standing.refuters.length > 0 && (
                <p className="flex items-center gap-1 pl-1 text-2xs text-muted-foreground">
                  <ShieldX aria-hidden className="size-3" />
                  Refuted by {standing.refuters.join(", ")}
                </p>
              )}
            </li>
          );
        })}
      </ul>
    </aside>
  );
}
