// The pure rules behind presence and typing indicators.
//
// Kept out of the hooks on purpose: every rule here is a timing rule, and a
// timing rule tested through a rendered component is tested by sleeping. These
// take a clock as an argument instead, so the awkward cases — a frame that
// arrives already stale, an indicator racing the message that should cancel it
// — are checked directly.

/** How often a live console announces itself. Mirrors the host constant. */
export const PRESENCE_HEARTBEAT_MS = 60_000;

/**
 * How long a peer stays present without a fresh frame.
 *
 * Three times the heartbeat, matching the host's lease. The console has to
 * agree with the host here or the two disagree about who is online — and the
 * console is the one drawing the dot.
 */
export const PRESENCE_TTL_MS = 3 * PRESENCE_HEARTBEAT_MS;

/** No document input for this long means the human has left the machine. */
export const IDLE_AWAY_MS = 10 * 60_000;

/** How often one console may say it is typing, per channel. */
export const TYPING_SEND_THROTTLE_MS = 3_000;

/** How long a typing indicator survives without a renewal. */
export const TYPING_RECEIVE_TTL_MS = 8_000;

/** How often expired typers are pruned — only while there are any. */
export const TYPING_PRUNE_INTERVAL_MS = 1_000;

/** How long after somebody's message to ignore their typing frames. */
export const TYPING_POST_MESSAGE_SUPPRESS_MS = 2_000;

export type PresenceStatus = "online" | "away" | "offline";

/** What the viewer has chosen to appear as. */
export type PresencePreference = "auto" | "away" | "offline";

export interface Peer {
  status: PresenceStatus;
  atMillis: number;
}

/** One person typing in one place. */
export interface Typer {
  userId: string;
  chatId: string;
  parentId?: string;
  /** When this indicator stops being true. */
  expiresAt: number;
  /** When this person was first seen typing here — the stable sort key. */
  firstSeenAt: number;
}

/**
 * What to announce, given the viewer's preference and how long they have been
 * idle.
 *
 * `away` means "the human is not at the machine" — Slack and Discord's meaning.
 * It deliberately does **not** mean "the window is unfocused": people read a
 * channel in a background tab constantly, and marking them away for it makes
 * the dot mean nothing. A browser has no OS-idle API, so document-level input
 * idleness is the honest analogue.
 */
export function presenceToAnnounce(
  preference: PresencePreference,
  idleForMs: number,
): PresenceStatus {
  if (preference === "offline") return "offline";
  if (preference === "away") return "away";
  return idleForMs >= IDLE_AWAY_MS ? "away" : "online";
}

/**
 * Applies one presence frame to the peer map, or removes the peer when they
 * went offline.
 *
 * Returns a new map only when something changed, so an unchanged frame does not
 * re-render every consumer.
 */
export function applyPresence(
  peers: ReadonlyMap<string, Peer>,
  frame: { userId: string; status: PresenceStatus; atMillis: number },
): Map<string, Peer> | null {
  const current = peers.get(frame.userId);
  // Out-of-order delivery is possible on a reconnect, and an older frame must
  // not undo a newer one.
  if (current && current.atMillis > frame.atMillis) return null;
  if (frame.status === "offline") {
    if (!current) return null;
    const next = new Map(peers);
    next.delete(frame.userId);
    return next;
  }
  if (current && current.status === frame.status) return null;
  const next = new Map(peers);
  next.set(frame.userId, { status: frame.status, atMillis: frame.atMillis });
  return next;
}

/**
 * Merges a `GET /presence` snapshot into the peers already held, instead of
 * replacing the map outright.
 *
 * The snapshot request and the live SSE stream race: a status change or a
 * disconnect can arrive over the stream *while the GET is in flight*, and a
 * plain replace on the response would silently undo it — reverting a status
 * change to what the snapshot saw a moment earlier, or resurrecting somebody
 * whose `offline` frame already deleted them, for up to the next refresh
 * interval. Reconciled here the same way a live frame is: per user, whichever
 * `atMillis` is newer wins.
 *
 * `tombstones` (userId → the `atMillis` of the last known `offline` frame) is
 * what makes a disconnect win even though a departed peer has no entry left in
 * `peers` to compare against — without it, any snapshot row for them would be
 * applied unconditionally, since there is nothing in the *map* to compare its
 * timestamp to. `requestSentAt` (`Date.now()` when the GET was issued) plays
 * the same role for a peer the snapshot doesn't mention at all: they are only
 * dropped if what we already knew about them predates the request, so an
 * arrival whose live frame raced ahead of the snapshot is not undone by the
 * snapshot's silence about them.
 */
export function reconcilePresenceSnapshot(
  peers: ReadonlyMap<string, Peer>,
  tombstones: ReadonlyMap<string, number>,
  snapshot: ReadonlyArray<{ userId: string; status: PresenceStatus; atMillis: number }>,
  requestSentAt: number,
): Map<string, Peer> {
  const next = new Map(peers);
  const seen = new Set<string>();
  // The type says this is an array; a host says whatever it says. An older
  // host, a proxy, or a stub that answers `[]` for unrecognised routes makes
  // this `undefined`, and iterating it throws during render — which blanks the
  // whole console, not just the presence roster. A missing snapshot means "no
  // news", which is exactly an empty one.
  if (!Array.isArray(snapshot)) return next;
  for (const row of snapshot) {
    seen.add(row.userId);
    const current = next.get(row.userId);
    if (current && current.atMillis > row.atMillis) continue;
    const tombstone = tombstones.get(row.userId);
    if (tombstone !== undefined && tombstone > row.atMillis) continue;
    next.set(row.userId, { status: row.status, atMillis: row.atMillis });
  }
  for (const [userId, peer] of next) {
    if (seen.has(userId)) continue;
    // Missing from the snapshot. Only take that as "gone" when what we
    // already knew predates the request — otherwise a live frame arrived
    // after the GET was issued and simply raced ahead of its response.
    if (peer.atMillis < requestSentAt) next.delete(userId);
  }
  return next;
}

/** Peers whose lease is still good. */
export function livePeers(
  peers: ReadonlyMap<string, Peer>,
  now: number,
): Map<string, Peer> {
  const next = new Map<string, Peer>();
  for (const [id, peer] of peers) {
    if (now - peer.atMillis <= PRESENCE_TTL_MS) next.set(id, peer);
  }
  return next;
}

/** The key one typing indicator is held under. */
export function typerKey(frame: { userId: string; chatId: string; parentId?: string }): string {
  return `${frame.userId}:${frame.chatId}:${frame.parentId ?? ""}`;
}

/**
 * Whether a typing frame should be shown at all.
 *
 * Two rules, both of which exist because a typing indicator that lies is worse
 * than none:
 *
 * 1. **A frame at or before that author's last message here is stale.** Their
 *    message and their final typing ping race constantly, and the ping loses:
 *    somebody who has just spoken is not still typing.
 * 2. **A short window after their message, ignore them entirely.** Rule 1 only
 *    catches frames stamped before the message; one stamped a few milliseconds
 *    after it is the same in-flight ping and reads the same way to a viewer.
 *
 * There used to be a third rule here — drop a frame already older than its own
 * TTL — but it compared `frame.atMillis` (the *server's* clock) against `now`
 * (the *browser's*), so any viewer whose workstation clock ran more than
 * [`TYPING_RECEIVE_TTL_MS`] ahead of the host saw every typing frame arrive
 * "already expired" and typing broke outright for them. `atMillis` is kept
 * for the two rules above — both compare it only to `lastMessageAt`, another
 * server timestamp, so they never cross clock domains — and freshness is
 * [`typingExpiry`]'s job now, anchored to local receipt time instead.
 */
export function shouldShowTyping(
  frame: { atMillis: number },
  lastMessageAt: number | undefined,
): boolean {
  if (lastMessageAt === undefined) return true;
  if (frame.atMillis <= lastMessageAt) return false;
  return frame.atMillis - lastMessageAt > TYPING_POST_MESSAGE_SUPPRESS_MS;
}

/**
 * When a typing indicator should disappear: a TTL from the moment *this*
 * console received it.
 *
 * Deliberately not derived from `frame.atMillis` (the server's clock) at all
 * — see [`shouldShowTyping`]'s header. Anchoring to local receipt time instead
 * means a skewed workstation clock can no longer make an indicator vanish
 * instantly or, in the other direction, linger past its host-side lease; the
 * only clock this reads is the one the caller is already using for `now`.
 */
export function typingExpiry(now: number): number {
  return now + TYPING_RECEIVE_TTL_MS;
}

/** Drops every typer whose indicator has expired. */
export function pruneTypers(typers: readonly Typer[], now: number): Typer[] {
  return typers.filter((t) => t.expiresAt > now);
}

/**
 * The typers to show in one channel, in a stable order.
 *
 * Sorted by when each was first seen rather than by their latest frame, so the
 * list does not reshuffle every three seconds as renewals arrive — which reads
 * as flicker even though nobody has started or stopped.
 */
export function typersIn(
  typers: readonly Typer[],
  chatId: string,
  parentId: string | undefined,
  now: number,
): Typer[] {
  return typers
    .filter(
      (t) =>
        t.chatId === chatId &&
        (t.parentId ?? undefined) === parentId &&
        t.expiresAt > now,
    )
    .sort((a, b) => a.firstSeenAt - b.firstSeenAt || a.userId.localeCompare(b.userId));
}

/**
 * "Jane is typing…", "Jane and Ada are typing…", "Several people are typing…".
 *
 * Names at most two. Three names is already longer than the line it sits on,
 * and past that the count is what a reader actually takes in.
 */
export function typingLabel(names: string[]): string | null {
  if (names.length === 0) return null;
  if (names.length === 1) return `${names[0]} is typing…`;
  if (names.length === 2) return `${names[0]} and ${names[1]} are typing…`;
  return "Several people are typing…";
}
