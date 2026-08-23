import type { NotificationDto } from "@/api/types";

/**
 * The mention badge: how many unread mentions of **you** sit in each channel.
 *
 * # Why this is not the unread count
 *
 * They answer different questions and come from different places, and the whole
 * value of the mention badge is that it does *not* inherit the unread badge's
 * caveat:
 *
 * | | Unread | Mentions |
 * |---|---|---|
 * | Derived | in this browser, from what this tab has seen | by the host, from who was named |
 * | Survives a reload | only via the read-state floor | yes, it is a stored row |
 * | Means | "you have not looked here" | "somebody asked *you* something" |
 *
 * Merging them would take the durable, per-person fact and give it the
 * best-effort one's meaning. So the rail renders two badges, and only one of
 * them carries the "this tab only" tooltip.
 */
export function mentionCountsByChannel(
  notifications: readonly NotificationDto[],
): Record<string, number> {
  const out: Record<string, number> = {};
  // Defensive against a caller handing us something that is not a list. The
  // types say it cannot happen; a host returning an unexpected shape says
  // otherwise, and the consequence of being wrong here is a render-time throw
  // that blanks the console rather than a missing badge.
  if (!Array.isArray(notifications)) return out;
  for (const n of notifications) {
    // Unread only. A mention you have already dealt with is not a summons.
    if (n.readAt !== undefined) continue;
    // `kind` rather than `subjectKind`: a future notification about a message
    // that is not a mention (a reply, a reaction) must not silently start
    // badging as one.
    if (n.kind !== "mention") continue;
    // A row with no channel cannot be placed on the rail. Counted nowhere
    // rather than counted somewhere arbitrary.
    if (!n.context) continue;
    out[n.context] = (out[n.context] ?? 0) + 1;
  }
  return out;
}

/**
 * The ids to mark read when a channel is opened.
 *
 * Only that channel's, and only the unread ones — opening `#engineering` must
 * not silently clear a mention waiting in `#design`, which is exactly what a
 * bare "mark all" would do and exactly the summons somebody would then miss.
 */
export function mentionsToClear(
  notifications: readonly NotificationDto[],
  channelId: string,
): string[] {
  return notifications
    .filter(
      (n) =>
        n.readAt === undefined && n.kind === "mention" && n.context === channelId,
    )
    .map((n) => n.id);
}
