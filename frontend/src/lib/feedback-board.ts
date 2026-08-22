import { ApiError } from "@/api/types";
import type { BoardItem, BoardKind, BoardSort, BoardStatus, BoardVote } from "@/api/types";

/**
 * The pure half of the feedback board (`FeedbackView`).
 *
 * Everything here is a function of its arguments: which rows a filter admits,
 * what a vote does to a row's tallies, how a hidden board is told apart from an
 * empty one. The view owns the fetching and the pixels; this owns the rules,
 * because the rules are what break quietly — a vote that double-counts, a row
 * that lingers after a filter change — and a browser walk reports those as
 * "the board looked wrong".
 */

/** The orderings, in the order the segmented control shows them. */
export const BOARD_SORTS: { value: BoardSort; label: string; hint: string }[] = [
  { value: "hot", label: "Hot", hint: "Rising now" },
  { value: "top", label: "Top", hint: "Most wanted overall" },
  { value: "new", label: "New", hint: "Newest first" },
];

/** Plain-language names for what the hub files things under. */
export const BOARD_KIND_LABELS: Record<BoardKind, string> = {
  feature: "Idea",
  bug: "Bug",
};

/** Plain-language names for where an item sits in triage. */
export const BOARD_STATUS_LABELS: Record<BoardStatus, string> = {
  open: "Open",
  planned: "Planned",
  completed: "Shipped",
  closed: "Closed",
};

/** The type filter's options, "all" first. */
export const BOARD_KIND_FILTERS: { value: BoardKind | "all"; label: string }[] = [
  { value: "all", label: "Everything" },
  { value: "feature", label: "Ideas" },
  { value: "bug", label: "Bugs" },
];

/**
 * The status filter's options.
 *
 * `closed` is deliberately absent: the hub refuses it as a filter (it only
 * accepts open/planned/completed), even though it will happily *return* a
 * closed item, which is why {@link BOARD_STATUS_LABELS} still names it.
 */
export const BOARD_STATUS_FILTERS: { value: BoardStatus | "all"; label: string }[] = [
  { value: "all", label: "Any status" },
  { value: "open", label: "Open" },
  { value: "planned", label: "Planned" },
  { value: "completed", label: "Shipped" },
];

/**
 * Whether a row still belongs in the current view.
 *
 * Used after a mutation, to decide between patching the row in place and
 * refetching: a vote never changes membership, but a status change can push a
 * row out of an active filter, and a row that stays behind is a row the
 * operator will try to interact with after the server stopped listing it.
 */
export function matchesBoardFilters(
  item: BoardItem,
  kind: BoardKind | "all",
  status: BoardStatus | "all",
): boolean {
  return (
    (kind === "all" || item.kind === kind) && (status === "all" || item.status === status)
  );
}

/**
 * The row as it will look once `next` is cast, computed locally so the arrow
 * fills the instant it is clicked instead of after a round trip.
 *
 * Applies the same arithmetic the hub does — retract the standing vote, then
 * add the new one — so an optimistic row and the row the server returns agree,
 * and clicking the same arrow twice retracts rather than double-counting.
 */
export function applyVote(item: BoardItem, next: BoardVote): BoardItem {
  let { upvotes, downvotes } = item;
  if (item.my_vote === 1) upvotes -= 1;
  if (item.my_vote === -1) downvotes -= 1;
  if (next === 1) upvotes += 1;
  if (next === -1) downvotes += 1;
  return {
    ...item,
    upvotes: Math.max(0, upvotes),
    downvotes: Math.max(0, downvotes),
    score: Math.max(0, upvotes) - Math.max(0, downvotes),
    my_vote: next,
  };
}

/** What clicking `direction` means given the vote already standing on a row. */
export function nextVote(current: BoardVote, direction: 1 | -1): BoardVote {
  return current === direction ? 0 : direction;
}

/**
 * Whether this failure means "this host has no board" rather than "the board
 * failed to load".
 *
 * The host answers `404 tinyhumans_no_board` when it holds no TinyHumans
 * credential. That is not an error to show an operator — there is simply
 * nothing to show — so the view hides the whole section instead of parking a
 * red banner on a surface that will never work on this instance.
 */
export function isBoardUnavailable(error: unknown): boolean {
  return error instanceof ApiError && error.code === "tinyhumans_no_board";
}

/**
 * Epoch millis for an ISO timestamp, for {@link timeAgo}.
 *
 * The board carries the hub's ISO strings rather than millis (the host only
 * forwards them), and an unparseable one must not render as "56 years ago" —
 * `null` means "say nothing about when".
 */
export function boardTimeMillis(iso: string): number | null {
  const parsed = Date.parse(iso);
  return Number.isNaN(parsed) ? null : parsed;
}
