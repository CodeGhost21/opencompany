// The review surface: which message a settled card's verdict hangs off, and the
// budget-pause markers that live in the same transcript.
//
// Split out of the old `model.ts` (issue: room store / P2). Everything here is
// about a *card's* lifecycle showing up inside a conversation, which is a
// different subject from what a channel is (`channels.ts`) or how its rows are
// grouped (`timeline.ts`).

// The chat workspace's data model: channels, direct messages, and the grouping
// rules the timeline reads. Everything here is pure — the view owns the state.

import { isAnyBudgetPauseNotice, parseBudgetPauseAgent } from "@/hooks/use-events";
import {
  type ChatMessage,
} from "@/lib/chat";
import type { TaskStatus } from "@/api/tasks";
import type { Transcripts } from "./timeline";

/**
 * A host desk (`GET .../desks`), shaped into the console's `Desk`. The host
 * has no separate channel-slug or blurb field, so the slug is derived from
 * the desk's name and the blurb falls back to its description — the id is
 * the one field that must survive untouched, since it doubles as the chat
 * thread id `send` addresses.
 *
 * `members` / `overlayMembers` come through as the host sent them, order
 * included — `members[0]` is the desk's lead, and the rest is the hierarchy the
 * company declared. Dropping them here is what made every channel show the
 * whole company (issue #369).
 */
/**
 * The id of the most recent system settle pill carrying each `taskId`, last
 * occurrence wins.
 *
 * Shared by {@link buildTimeline} (which stamps `isLatestSettlePill` on every
 * row) and {@link reviewCardIdForThread} (which must apply the identical
 * latest-pill gate to a reply anchor, not just to the row's Approve button) —
 * one definition of "latest" for both surfaces.
 */
export function latestSettlePillIdByTaskId(messages: readonly ChatMessage[]): Map<string, string> {
  const latest = new Map<string, string>();
  for (const m of messages) {
    if (m.from === "system" && m.taskId !== undefined) latest.set(m.taskId, m.id);
  }
  return latest;
}

/**
 * The in-review dispatch card a chat thread is reviewing, or `undefined` when
 * `parent` is not a review surface.
 *
 * A parent is a review surface when it is the card's settle pill — a system
 * marker carrying its `taskId` — or the relay bubble that followed it: the
 * pill's *first* company line with no `taskId`, mirroring the backend's
 * `is_relay_bubble_for`. A later, ordinary company reply is not a review
 * surface even though it has the same shape. Either way the card must still
 * be in `in_review`; one already approved or re-running is no longer open
 * for review.
 *
 * A card that finished, was revised, and is `in_review` again mints a NEW
 * settle pill while the old one stays in history under the same `taskId`.
 * Only the newest pill (or its relay) is a live review surface — the same
 * {@link latestSettlePillIdByTaskId} gate {@link buildTimeline} uses for the
 * Approve control — so opening an old pill's thread and replying there does
 * not silently apply feedback to, and re-dispatch, the latest attempt.
 */
export function reviewCardIdForThread(
  parent: ChatMessage,
  messages: readonly ChatMessage[],
  statusByTaskId: Readonly<Record<string, TaskStatus>>,
): string | undefined {
  const inReview = (taskId: string | undefined): taskId is string =>
    taskId !== undefined && statusByTaskId[taskId]?.column === "in_review";
  const latestPillIdByTaskId = latestSettlePillIdByTaskId(messages);
  const isLatestPill = (pill: ChatMessage): boolean =>
    pill.taskId !== undefined && latestPillIdByTaskId.get(pill.taskId) === pill.id;
  if (parent.from === "system") {
    return inReview(parent.taskId) && isLatestPill(parent) ? parent.taskId : undefined;
  }
  if (parent.from !== "company" || parent.taskId) return undefined;
  const index = messages.findIndex((m) => m.id === parent.id);
  if (index < 0) return undefined;
  for (let i = index - 1; i >= 0; i--) {
    const prior = messages[i];
    if (prior.from === "company" && !prior.taskId) return undefined;
    if (prior.from !== "system" || !prior.taskId) continue;
    return inReview(prior.taskId) && isLatestPill(prior) ? prior.taskId : undefined;
  }
  return undefined;
}

/**
 * Every distinct in-review card a thread anchors to, as `{taskId, anchorId}`
 * pairs — one entry per card, newest-checked-first: `parent` itself, then
 * `replies` from most to least recent.
 *
 * A thread usually anchors at most one card, but a second can be dispatched
 * from inside it before the first is settled (Codex #3906594069), leaving
 * both live in the same thread at once. Each stays its own entry here —
 * {@link reviewCardIdForThread}'s stale-pill gate already keeps a superseded
 * pass of the SAME card out of this list, so only genuinely distinct cards
 * collect, never two anchors for one taskId.
 */
export function reviewAnchorsForThread(
  parent: ChatMessage,
  replies: readonly ChatMessage[],
  messages: readonly ChatMessage[],
  statusByTaskId: Readonly<Record<string, TaskStatus>>,
): { taskId: string; anchorId: string }[] {
  const seen = new Set<string>();
  const anchors: { taskId: string; anchorId: string }[] = [];
  const candidates: readonly ChatMessage[] = [parent, ...[...replies].reverse()];
  for (const candidate of candidates) {
    const taskId = reviewCardIdForThread(candidate, messages, statusByTaskId);
    if (taskId === undefined || seen.has(taskId)) continue;
    seen.add(taskId);
    anchors.push({ taskId, anchorId: candidate.id });
  }
  return anchors;
}

/**
 * Where a thread's review feedback should be anchored, or `undefined` when
 * the thread is not reviewing anything.
 *
 * The newest of {@link reviewAnchorsForThread}'s cards — the thread's one
 * composer can only ever target a single card with a typed reply, so when
 * more than one is live this is the one it targets. `parent` itself is the
 * review surface for a thread opened directly on a settle pill or its relay.
 * But when the card that produced the pill was itself sent inside an
 * existing thread, the pill and its relay land as replies under that
 * thread's own root — `parent` is neither of them, so
 * {@link reviewCardIdForThread} on `parent` alone finds nothing. Falls back
 * to scanning `replies` (newest first) for the review surface among them,
 * and anchors there instead.
 */
export function reviewAnchorForThread(
  parent: ChatMessage,
  replies: readonly ChatMessage[],
  messages: readonly ChatMessage[],
  statusByTaskId: Readonly<Record<string, TaskStatus>>,
): { taskId: string; anchorId: string } | undefined {
  return reviewAnchorsForThread(parent, replies, messages, statusByTaskId)[0];
}

/**
 * Whether `taskId`'s Approve/Revise click should go out right now.
 *
 * `reviewingCardIds` is keyed per card, not a single global slot — since
 * {@link reviewAnchorsForThread} (`a99b39e87`) made every distinct in-review
 * card in a thread independently actionable, a click on one card's control
 * must not be silently dropped just because a DIFFERENT card's verdict is
 * still in flight (Codex #3906779123). Only a click repeated on the SAME
 * card while its own verdict is outstanding is refused. The host is safe to
 * take both at once: `runtime.task_writes` (`3ab934918`) serializes review
 * verdicts per company, so a second card's write simply queues behind the
 * first instead of racing it.
 */
export function canSubmitReview(
  reviewingCardIds: ReadonlySet<string>,
  activeThreadId: string | undefined,
  taskId: string,
): boolean {
  return activeThreadId !== undefined && !reviewingCardIds.has(taskId);
}

/**
 * The message id of the MOST RECENT budget-pause notice per agent,
 * COMPANY-WIDE — not scoped to one channel (issue #1846 review, Codex
 * #3865395879).
 *
 * The backend keeps at most one parked marker per agent, full stop: a pause
 * in channel A followed by a pause for the SAME agent in channel B overwrites
 * A's marker with B's, regardless of which channel either happened in. This
 * used to be computed from a single channel's `items` inside `MessageTimeline`
 * — correct for the channel that is actually open, but wrong for any OTHER
 * channel holding an older notice for an agent who has since paused again
 * elsewhere: reopening that channel, its own last notice still reads as the
 * newest ONE IT has seen, so the "Add credits & resend" button stayed
 * enabled — and clicking it silently redeemed the newer, different-channel
 * marker and resent that unrelated message under the stale card.
 *
 * Folding over every channel in `transcripts` (which `AppShell` keeps live
 * for the whole company over SSE, not merely the currently open one) is what
 * actually matches the backend's one-marker-per-agent truth. Sorted by each
 * message's own `at` timestamp rather than by scan order, because iterating
 * one channel's array to completion before moving to the next would NOT
 * yield cross-channel chronological order on its own.
 *
 * Scanned with `isAnyBudgetPauseNotice`, NOT `isBudgetPauseNotice` (issue
 * #1906). The narrow check is the render-time one — "does this notice get an
 * Add-credits button" — and filtering the supersession map through it made
 * every NO-RESEND notice invisible here, while the marker it parked still
 * overwrote the previous one on the host. maya pauses on an interactive turn
 * (redeemable notice N1, marker M1); an approval for maya is then approved,
 * its continuation runs through `run_steered_background`, pauses, and parks M2
 * (`background: true`) over M1 with a no-resend notice. N1 was still this
 * map's answer for maya, so its CTA stayed enabled and clicking it sent
 * `?id=M1.id` — a marker that no longer exists — for a `RedeemMatch::Stale`
 * 409 that refreshing could never clear. A pause is a pause for supersession
 * purposes whether or not the operator gets a button for it.
 */
export function latestBudgetPauseMessageIdByAgent(transcripts: Transcripts): Map<string, string> {
  const latest = new Map<string, { messageId: string; at: number }>();
  for (const messages of Object.values(transcripts)) {
    for (const message of messages) {
      if (!isAnyBudgetPauseNotice(message.text)) continue;
      const agentId = parseBudgetPauseAgent(message.text);
      if (agentId == null) continue;
      const seen = latest.get(agentId);
      if (seen == null || message.at >= seen.at) {
        latest.set(agentId, { messageId: message.id, at: message.at });
      }
    }
  }
  const out = new Map<string, string>();
  for (const [agentId, { messageId }] of latest) out.set(agentId, messageId);
  return out;
}

/**
 * Folds one `GET …/budget-pause` read into `RoomView`'s
 * `budgetPauseMarkerByNotice` cache (issue #1846 review, Codex #3868962374)
 * — the notice-render-time read that replaces redeeming off a live re-read
 * at click time. Pure so it can be pinned directly; there is no
 * component-test harness in this project to mount the effect it backs (see
 * `budget-pause-notice.test.ts`'s header doc for the same constraint).
 *
 * Never overwrites an id already cached for `messageId` — the FIRST
 * successful read for a given notice is the one that landed closest to when
 * the card actually appeared, which is exactly the property this cache
 * exists to buy: a later read racing a background re-park would otherwise
 * silently replace the correct id with a newer, unrelated one.
 */
export function mergeBudgetPauseMarkerRead(
  prev: Map<string, string>,
  messageId: string,
  markerId: string,
): Map<string, string> {
  if (prev.has(messageId)) return prev;
  const next = new Map(prev);
  next.set(messageId, markerId);
  return next;
}

/**
 * The marker id `redeemBudgetPause` should send for a click on `noticeMessageId`
 * (issue #1846 review, Codex #3868962374): the notice-render-time cached read
 * when one landed in time, else `liveFallback` — a live read performed AT
 * click time, for the narrow case the cache has nothing yet (a click landing
 * faster than the render-time `GET` resolved). Falling back rather than
 * refusing the click keeps this no worse than the pre-fix behaviour, which
 * always read live at click time.
 */
export function budgetPauseRedeemId(
  noticeMessageId: string,
  markerByNotice: Map<string, string>,
  liveFallback: string | undefined,
): string | undefined {
  return markerByNotice.get(noticeMessageId) ?? liveFallback;
}

