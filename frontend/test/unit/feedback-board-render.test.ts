// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError, type BoardItem, type BoardPage } from "@/api/types";
import { FeedbackBoard } from "@/views/feedback/FeedbackBoard";

/**
 * The board as an operator meets it.
 *
 * This file earns the unit runner's render exception the same way
 * `workflow-run-board` does: the two facts under test are only true on screen.
 * A host with no TinyHumans credential must render **nothing** — not an empty
 * board, which reads as "nobody has ever asked for anything" — and a vote must
 * fill its arrow before the round trip answers, which is the whole reason the
 * optimistic path exists. `feedback-board.test.ts` covers the arithmetic those
 * two rely on.
 */

const ROW: BoardItem = {
  id: "one",
  kind: "feature",
  title: "Weekly digest",
  body: "Send one on Mondays",
  status: "planned",
  author: "rin",
  upvotes: 3,
  downvotes: 1,
  score: 2,
  comment_count: 0,
  my_vote: 0,
  issue_url: null,
  created_at: "2026-01-02T00:00:00.000Z",
};

function page(items: BoardItem[]): BoardPage {
  return { items, total: items.length, page: 1, limit: 20 };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // React only suppresses its "not configured to support act(...)" warning when
  // this flag is set; every other render suite here sets it the same way.
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  act(() => {
    root = createRoot(container);
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(client: Partial<OpenCompanyClient>, onAvailability = vi.fn()): void {
  act(() => {
    root.render(
      createElement(FeedbackBoard, {
        client: client as OpenCompanyClient,
        company: "acme",
        refreshKey: 0,
        onAvailability,
      }),
    );
  });
}

/** Lets every already-resolved promise settle and React commit the result. */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("FeedbackBoard", () => {
  it("renders what the hub returned", async () => {
    render({ feedbackBoard: vi.fn().mockResolvedValue(page([ROW])) });
    await settle();

    expect(container.textContent).toContain("Weekly digest");
    // The status reads in the console's words, not the hub's tokens.
    expect(container.textContent).toContain("Planned");
    expect(container.textContent).toContain("2");
  });

  it("disappears entirely on a host with no board", async () => {
    const onAvailability = vi.fn();
    render(
      {
        feedbackBoard: vi
          .fn()
          .mockRejectedValue(new ApiError(404, "tinyhumans_no_board", "no board", true)),
      },
      onAvailability,
    );
    await settle();

    expect(container.textContent).toBe("");
    expect(onAvailability).toHaveBeenCalledWith(false);
  });

  it("reports a real failure instead of hiding it", async () => {
    render({
      feedbackBoard: vi
        .fn()
        .mockRejectedValue(new ApiError(502, "tinyhumans_http_500", "the hub is down", true)),
    });
    await settle();

    expect(container.textContent).toContain("the hub is down");
  });

  it("fills the arrow before the vote round-trips, and keeps the server's answer", async () => {
    let resolveVote: (item: BoardItem) => void = () => {};
    const voteFeedbackBoard = vi.fn(
      () =>
        new Promise<BoardItem>((resolve) => {
          resolveVote = resolve;
        }),
    );
    render({ feedbackBoard: vi.fn().mockResolvedValue(page([ROW])), voteFeedbackBoard });
    await settle();

    const upvote = container.querySelector<HTMLButtonElement>('button[aria-label="Upvote"]');
    expect(upvote).not.toBeNull();
    act(() => upvote!.click());

    // Optimistic: the score moved and the arrow is pressed while the call hangs.
    // Scoped to the row's own controls — the sort buttons carry `aria-pressed`
    // too, and the active sort is pressed from the first render.
    expect(container.querySelector('button[aria-label="Take your upvote back"]')).not.toBeNull();
    expect(container.querySelector("li")?.textContent).toContain("3");
    expect(voteFeedbackBoard).toHaveBeenCalledWith("one", 1, "acme");

    // The server is the final word — including when it disagrees with the guess.
    await act(async () => {
      resolveVote({ ...ROW, upvotes: 9, score: 8, my_vote: 1 });
      await Promise.resolve();
    });
    expect(container.textContent).toContain("8");
  });

  it("puts the row back when the vote fails", async () => {
    render({
      feedbackBoard: vi.fn().mockResolvedValue(page([ROW])),
      voteFeedbackBoard: vi
        .fn()
        .mockRejectedValue(new ApiError(503, "tinyhumans_unreachable", "hub unreachable", true)),
    });
    await settle();

    act(() => {
      container.querySelector<HTMLButtonElement>('button[aria-label="Upvote"]')!.click();
    });
    await settle();

    expect(container.textContent).toContain("hub unreachable");
    // Back to the untouched row: the upvote is on offer again, not standing.
    expect(container.querySelector('button[aria-label="Take your upvote back"]')).toBeNull();
    expect(container.querySelector('button[aria-label="Upvote"]')).not.toBeNull();
  });
});
