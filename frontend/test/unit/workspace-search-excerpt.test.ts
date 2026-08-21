import { describe, expect, it } from "vitest";

import { centerExcerpt, highlightRuns, SEARCH_LIMIT } from "@/api/workspace";

/**
 * Issue #1375: a search excerpt could clip its own highlight out of view.
 *
 * The host returns a window of context around the match; the console renders it
 * into a two-line clamp about 250px wide. A match sitting past the first dozen
 * words is therefore below the fold of its own excerpt, and the operator reads
 * two lines of arbitrary prose with nothing marked in them.
 */

describe("centerExcerpt", () => {
  it("leaves an early match exactly where the host put it", () => {
    const text = "Pagination is decided per registry, not per call.";
    expect(centerExcerpt(text, "Pagination")).toBe(text);
    // No decorative ellipsis on text that was already fine.
    expect(centerExcerpt(text, "Pagination").startsWith("…")).toBe(false);
  });

  it("pulls a late match forward so it lands in the first line", () => {
    const text =
      "This document opens with a long preamble about scope and ownership before it ever mentions pagination at all.";
    const centred = centerExcerpt(text, "pagination");

    expect(centred.startsWith("…")).toBe(true);
    expect(centred).toContain("pagination");
    // The reported defect: the match was ~100 characters in, well past a
    // two-line clamp. It must now be near the front.
    expect(centred.toLowerCase().indexOf("pagination")).toBeLessThan(40);
  });

  it("starts on a whole word rather than mid-word", () => {
    const text =
      "aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm nnn ooo target ppp";
    const centred = centerExcerpt(text, "target");

    // Every fragment after the leading ellipsis is one of the source's words.
    const words = centred.replace("…", "").split(" ").filter(Boolean);
    for (const word of words) expect(text).toContain(word);
    expect(text.split(" ")).toContain(words[0]);
  });

  it("returns the text untouched when the query does not appear", () => {
    const text = "Nothing in here matches.";
    expect(centerExcerpt(text, "pagination")).toBe(text);
  });

  it("is a no-op for an empty or whitespace query", () => {
    const text = "Some prose.";
    expect(centerExcerpt(text, "")).toBe(text);
    expect(centerExcerpt(text, "   ")).toBe(text);
  });

  it("keeps the highlight resolvable after centring", () => {
    const text =
      "A long preamble that goes on for a while before it finally says pagination out loud.";
    const centred = centerExcerpt(text, "pagination");
    const runs = highlightRuns(centred, "pagination");

    expect(runs.some((r) => r.hit)).toBe(true);
    expect(runs.find((r) => r.hit)?.text).toBe("pagination");
  });

  it("does not lowercase or otherwise rewrite the prose it keeps", () => {
    const text =
      "Mixed Case Preamble Running On And On And On Before The Word Pagination Appears.";
    expect(centerExcerpt(text, "pagination")).toContain("Pagination");
  });

  it("survives multibyte context without splitting a character", () => {
    const text = `${"héllo wörld ".repeat(6)}pagination`;
    const centred = centerExcerpt(text, "pagination");

    expect(centred).toContain("pagination");
    expect(centred).not.toContain("�");
  });
});

describe("SEARCH_LIMIT", () => {
  it("is the host's own ceiling, not its default", () => {
    // MAX_SEARCH_RESULTS = 50, DEFAULT_SEARCH_LIMIT = 20. Naming no limit is
    // what capped the console at 20 (issue #1457).
    expect(SEARCH_LIMIT).toBe(50);
  });
});
