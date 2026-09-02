// Where a card's origin conversation lives, for the card → chat half of the
// #246 round trip.
//
// A card created out of a conversation carries `originChatId`, a **host thread
// id**. Chats are served by the Room tab, addressed `#/chat/<channelId>`, so
// the id on the card is not itself an address: it has to be resolved through
// the shell's thread → channel map first, and that resolution can come back
// empty (a thread whose desk is gone, a map not yet loaded). Deciding here
// rather than at the button keeps "is there somewhere to go" and "go there"
// from drifting apart.

import { channelForThread } from "@/views/chat/model";

export type OriginConversation =
  /** The card was not opened from a conversation; the row renders nothing. */
  | { kind: "none" }
  /** It was, but no channel carries that thread — state the origin, offer no jump. */
  | { kind: "unreachable" }
  /** It was, and the Room channel below renders it. */
  | { kind: "channel"; channelId: string };

/**
 * Resolved through {@link channelForThread}, not a bare `map[originChatId]`:
 * the host compares the General spellings case-insensitively and echoes back
 * whichever one it was addressed with, so a direct index misses a card opened
 * from `MAIN`.
 */
export function originConversation(
  originChatId: string | undefined | null,
  chatChannelByThread: Readonly<Record<string, string>> | undefined,
): OriginConversation {
  if (!originChatId) return { kind: "none" };
  const channelId = chatChannelByThread
    ? channelForThread(chatChannelByThread, originChatId)
    : null;
  return channelId ? { kind: "channel", channelId } : { kind: "unreachable" };
}
