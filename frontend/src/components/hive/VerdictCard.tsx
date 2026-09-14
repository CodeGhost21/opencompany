import { hiveIcon } from "@/components/hive/icons";
import { TopicChip } from "@/components/hive/TopicChip";
import type { Episode } from "@/lib/hive/episode";
import { ENDING_MARKS, TONE_CLASSES } from "@/lib/hive/tone";
import { cn } from "@/lib/utils";

/**
 * How a room ended.
 *
 * The closing `hive-report` row rendered as a verdict rather than a grey system
 * line — it is the answer to the question the operator asked, and it arrived
 * looking like plumbing.
 *
 * **The host's sentence is rendered verbatim and is the authority.** The spec is
 * deliberate that the summary says what happened rather than restating the
 * decision's content, because the argument is in the transcript directly above
 * it and a paraphrase would be a second, unattributed account of a conversation
 * that already has one. So this card never rewrites it — it frames it, and where
 * the console's own fold reached a different reading it says so in a footnote
 * instead of quietly showing its own.
 */
export function VerdictCard({
  episode,
  onSelectTopic,
  className,
}: {
  episode: Episode;
  onSelectTopic?: (topic: string) => void;
  className?: string;
}) {
  const ending = episode.ending;
  const mark = ending ? ENDING_MARKS[ending.kind] : undefined;
  const tone = mark ? TONE_CLASSES[mark.tone] : TONE_CLASSES.running;
  const Icon = hiveIcon(mark?.icon ?? "scale");

  return (
    <div
      className={cn(
        "rounded-lg border border-border bg-card px-4 py-3 text-sm",
        className,
      )}
    >
      <div className="flex items-center gap-2">
        <span className={cn("flex size-6 items-center justify-center rounded-md", tone.soft)}>
          <Icon aria-hidden className={cn("size-3.5", tone.text)} />
        </span>
        <span className={cn("text-xs font-semibold uppercase tracking-wide", tone.text)}>
          {mark?.label ?? "Still deliberating"}
        </span>
        {ending ? (
          <span className="text-xs text-muted-foreground">
            {ending.turns} {ending.turns === 1 ? "turn" : "turns"}
          </span>
        ) : null}
      </div>

      {/* Verbatim. See the component's own note. */}
      {episode.reportText ? (
        <p className="mt-2 text-sm text-foreground">{episode.reportText}</p>
      ) : (
        <p className="mt-2 text-sm text-muted-foreground">
          The room has not reported yet.
        </p>
      )}

      {ending?.kind === "converged" && ending.supporters.length > 0 ? (
        <div className="mt-2 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <TopicChip
            topic={ending.topic}
            standing={episode.topics.find((t) => t.id === ending.topic)}
            quorum={episode.quorum}
            onSelect={onSelectTopic}
          />
          <span>backed by {ending.supporters.join(", ")}</span>
        </div>
      ) : null}

      {ending?.kind === "deadlocked" ? (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {ending.topics.map((topic) => (
            <TopicChip
              key={topic}
              topic={topic}
              standing={episode.topics.find((t) => t.id === topic)}
              quorum={episode.quorum}
              onSelect={onSelectTopic}
            />
          ))}
        </div>
      ) : null}

      {(episode.failed.length > 0 || episode.disagrees || episode.ambiguous) && (
        <ul className="mt-3 space-y-1 border-t border-border pt-2 text-xs text-muted-foreground">
          {episode.failed.length > 0 && (
            <li>
              {episode.failed.length}{" "}
              {episode.failed.length === 1 ? "turn" : "turns"} did not finish; the room
              continued without {episode.failed.length === 1 ? "it" : "them"}.
            </li>
          )}
          {episode.disagrees && (
            <li>
              The console read this transcript differently. The desk&rsquo;s own report
              above is what happened.
            </li>
          )}
          {episode.ambiguous && (
            <li>
              The message that opened this room is outside the loaded history, so its
              first turns may be missing.
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
