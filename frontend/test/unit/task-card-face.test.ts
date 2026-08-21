// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { initials, notePreview } from "@/views/TaskCard";

/**
 * What a card is allowed to say about itself — two findings from a design audit
 * of the Work surface.
 *
 * Both are about the card *face*, which is the densest surface in the console:
 * one title, one secondary line, one avatar. Anything wrong there is wrong on
 * every card at once, which is how both of these went unnoticed — they read as
 * "that is just what the board looks like".
 */
describe("the note preview on a card", () => {
  it("drops the runtime's own bookkeeping", () => {
    // The line three of eight To-do cards were showing on a healthy board.
    expect(
      notePreview("[system] the dispatch cycle ended without settling this attempt"),
    ).toBeNull();
  });

  it("keeps what a person wrote, even beside a system line", () => {
    expect(
      notePreview("[system] dispatched\nWaiting on the vendor's security review."),
    ).toBe("Waiting on the vendor's security review.");
  });

  it("is null rather than empty for a card with no note", () => {
    // The card renders no line at all, rather than an empty one holding space.
    expect(notePreview(undefined)).toBeNull();
    expect(notePreview("   ")).toBeNull();
  });

  it("leaves an ordinary note alone", () => {
    expect(notePreview("Blocked on the Northwind contract.")).toBe(
      "Blocked on the Northwind contract.",
    );
  });
});

describe("the avatar's initials", () => {
  it("tells snake_case teammates apart", () => {
    // Splitting on whitespace alone returned one letter for every agent on the
    // board, so these three shared a glyph.
    expect(initials("docs_writer")).toBe("DW");
    expect(initials("devrel")).toBe("D");
    expect(initials("designer")).toBe("D");
    expect(initials("backend_engineer")).toBe("BE");
    expect(initials("frontend_engineer")).toBe("FE");
  });

  it("still reads a human name the way it always did", () => {
    expect(initials("Ada Okonkwo")).toBe("AO");
  });

  it("handles a hyphenated desk id", () => {
    expect(initials("go-to-market")).toBe("GT");
  });
});
