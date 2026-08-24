import type { NotificationDto } from "@/api/types";
import { MAIN_THREAD_ID } from "@/lib/chat";

/**
 * Mirrors the host's `is_general_chat` (`src/server/chat_history.rs`, issue
 * #65): the console addresses its default thread as `"main"`, an unaddressed
 * chat route stores the default desk `"General"`, and older events carry `""`.
 * All four spellings are one desk, and a notification context that names it has
 * to badge the *rendered* main channel — the rail is built from real desk ids,
 * none of which is `"General"`.
 */
function isGeneralChat(context: string | null | undefined): boolean {
  if (context === undefined || context === null) return false;
  const folded = context.toLowerCase();
  return context === "" || folded === MAIN_THREAD_ID || folded === "general";
}

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
  mainChannelId = MAIN_THREAD_ID,
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
    if (n.context === undefined || n.context === null) continue;
    // Every spelling of the General desk is the console's default thread, so
    // it badges the first rendered desk channel; anything else is a channel id
    // verbatim.
    const channelId = isGeneralChat(n.context) ? mainChannelId : n.context;
    out[channelId] = (out[channelId] ?? 0) + 1;
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
  mainChannelId = MAIN_THREAD_ID,
  visibleThreadIds: ReadonlySet<string> = new Set([channelId]),
): string[] {
  return notifications
    .filter((n) => {
      if (n.readAt !== undefined || n.kind !== "mention" || n.context === undefined) {
        return false;
      }
      if (n.context === channelId) {
        // Clear only once the main channel's history is actually on screen —
        // a mention is durable, and clearing it before the named message has
        // loaded would lose the summons for good.
        return channelId !== mainChannelId || visibleThreadIds.has(channelId);
      }
      // A general-chat spelling names the main channel: opening it clears
      // those mentions too.
      return channelId === mainChannelId && isGeneralChat(n.context);
    })
    .map((n) => n.id);
}
