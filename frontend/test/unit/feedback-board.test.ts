import { describe, expect, it } from "vitest";

import { ApiError, type BoardItem } from "@/api/types";
import {
  BOARD_STATUS_FILTERS,
  applyVote,
  boardTimeMillis,
  isBoardUnavailable,
  matchesBoardFilters,
  nextVote,
} from "@/lib/feedback-board";

/**
 * The feedback board's rules, away from its pixels.
 *
 * The board is a proxy of the TinyHumans hub, so the console cannot be trusted
 * to "just re-fetch and see": between the click and the answer it renders an
 * optimistic row, and every bug that matters lives in that gap — a vote counted
 * twice, a retraction that adds instead of removes, a row that survives a
 * filter it no longer matches.
 */

const ITEM: BoardItem = {
  id: "one",
  kind: "feature",
  title: "Weekly digest",
  body: "Send one on Mondays",
  status: "open",
  author: "rin",
  upvotes: 3,
  downvotes: 1,
  score: 2,
  comment_count: 0,
  my_vote: 0,
  issue_url: null,
  created_at: "2026-01-02T00:00:00.000Z",
};

describe("applyVote", () => {
  it("adds a first vote to the right tally", () => {
    const up = applyVote(ITEM, 1);
    expect(up.upvotes).toBe(4);
    expect(up.downvotes).toBe(1);
    expect(up.score).toBe(3);
    expect(up.my_vote).toBe(1);

    const down = applyVote(ITEM, -1);
    expect(down.downvotes).toBe(2);
    expect(down.score).toBe(1);
  });

  it("retracts the standing vote before applying the new one", () => {
    const voted = applyVote(ITEM, 1);
    // Switching sides moves the vote across rather than counting it on both.
    const flipped = applyVote(voted, -1);
    expect(flipped.upvotes).toBe(3);
    expect(flipped.downvotes).toBe(2);
    expect(flipped.score).toBe(1);
    expect(flipped.my_vote).toBe(-1);

    // And retracting outright puts the row back exactly where it started.
    const retracted = applyVote(voted, 0);
    expect(retracted.upvotes).toBe(ITEM.upvotes);
    expect(retracted.downvotes).toBe(ITEM.downvotes);
    expect(retracted.score).toBe(ITEM.score);
    expect(retracted.my_vote).toBe(0);
  });

  it("never renders a negative tally when the server disagrees", () => {
    // A row claiming `my_vote: 1` with no upvotes on it is a stale row, not an
    // invitation to draw "-1 votes".
    const stale = applyVote({ ...ITEM, upvotes: 0, my_vote: 1 }, 0);
    expect(stale.upvotes).toBe(0);
  });
});

describe("nextVote", () => {
  it("treats clicking the arrow you already chose as a retraction", () => {
    expect(nextVote(1, 1)).toBe(0);
    expect(nextVote(-1, -1)).toBe(0);
  });

  it("casts otherwise", () => {
    expect(nextVote(0, 1)).toBe(1);
    expect(nextVote(1, -1)).toBe(-1);
  });
});

describe("matchesBoardFilters", () => {
  it("admits everything under the open filters", () => {
    expect(matchesBoardFilters(ITEM, "all", "all")).toBe(true);
  });

  it("drops a row the active filter no longer covers", () => {
    expect(matchesBoardFilters(ITEM, "bug", "all")).toBe(false);
    expect(matchesBoardFilters(ITEM, "all", "planned")).toBe(false);
    expect(matchesBoardFilters({ ...ITEM, status: "planned" }, "feature", "planned")).toBe(true);
  });
});

describe("isBoardUnavailable", () => {
  it("recognizes the host that has no board", () => {
    expect(
      isBoardUnavailable(new ApiError(404, "tinyhumans_no_board", "no board", true)),
    ).toBe(true);
  });

  it("leaves every real failure to be reported", () => {
    expect(isBoardUnavailable(new ApiError(502, "tinyhumans_http_500", "upstream", true))).toBe(
      false,
    );
    expect(isBoardUnavailable(new ApiError(404, "not_found", "gone", true))).toBe(false);
    expect(isBoardUnavailable(new Error("offline"))).toBe(false);
    expect(isBoardUnavailable(null)).toBe(false);
  });
});

describe("boardTimeMillis", () => {
  it("parses the hub's ISO stamps", () => {
    expect(boardTimeMillis("2026-01-02T00:00:00.000Z")).toBe(Date.UTC(2026, 0, 2));
  });

  it("returns null rather than a nonsense date", () => {
    expect(boardTimeMillis("")).toBeNull();
    expect(boardTimeMillis("last tuesday")).toBeNull();
  });
});

describe("the status filter", () => {
  it("never offers `closed`, which the hub refuses as a filter", () => {
    expect(BOARD_STATUS_FILTERS.map((option) => option.value)).not.toContain("closed");
  });
});
