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
/**
 * The fixtures below are in the host's real note format, not a paraphrase of
 * it: `append_result` (`src/runtime/advance.rs:71`) writes `[<who>] <what>` and
 * joins blocks with a blank line, which its own test pins as
 * `"[maya] draft\n\n[system] gone"`.
 */
describe("the note preview on a card", () => {
  it("drops the runtime's own bookkeeping", () => {
    // The block three of eight To-do cards were showing on a healthy board.
    expect(
      notePreview("[system] the dispatch cycle ended without settling this attempt"),
    ).toBeNull();
  });

  it("drops an agent's attribution too, not just the host's", () => {
    // The card already says who owns it, as an avatar and a name. Repeating it
    // in the body is the same fact again in the noisiest place on the card.
    expect(
      notePreview("[frontend_engineer] __MOCK_LLM__ mock inference backend reply."),
    ).toBe("__MOCK_LLM__ mock inference backend reply.");
  });

  it("keeps what a person wrote, even beside a system block", () => {
    expect(
      notePreview("[system] dispatched\n\nWaiting on the vendor's security review."),
    ).toBe("Waiting on the vendor's security review.");
  });

  it("shows the most recent block, not the oldest", () => {
    // The journal is append-only, so a card that showed the top of the field
    // would freeze on the first thing that ever happened to it.
    expect(notePreview("[maya] draft\n\n[devrel] shipped the changelog")).toBe(
      "shipped the changelog",
    );
  });

  it("looks past a trailing system block to the last real one", () => {
    expect(
      notePreview(
        "[maya] draft\n\n[devrel] shipped the changelog\n\n[system] the host restarted",
      ),
    ).toBe("shipped the changelog");
  });

  it("keeps a multi-line block whole", () => {
    expect(notePreview("[maya] first line\nsecond line")).toBe("first line\nsecond line");
  });

  it("is null when the attribution was the whole block", () => {
    expect(notePreview("[system] gone\n\n[maya]")).toBeNull();
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
