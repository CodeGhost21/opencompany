import { describe, expect, it } from "vitest";

import { bestScore, excerptAround, matchRanges, matches, score } from "@/search/rank";

/**
 * Matching and ordering, away from anything that renders them.
 *
 * Highlighting is offsets rather than markup on purpose — a search box that
 * rebuilds user input as HTML is one paste away from being an injection — so
 * the offsets have to be right against the ORIGINAL string, not a lowercased
 * copy of it.
 */
describe("where a term matched", () => {
  it("gives offsets into the original text, not a lowercased copy", () => {
    const text = "Autumn Box";
    const [range] = matchRanges(text, "box");
    expect(text.slice(range[0], range[1])).toBe("Box");
  });

  it("finds every occurrence", () => {
    expect(matchRanges("box in a box", "box")).toEqual([
      [0, 3],
      [9, 12],
    ]);
  });

  it("treats an empty term as nothing asked rather than everything matched", () => {
    expect(matchRanges("anything", "")).toEqual([]);
    expect(matchRanges("", "box")).toEqual([]);
  });

  it("answers whether there is a match at all", () => {
    expect(matches("Autumn Box", "box")).toBe(true);
    expect(matches("Autumn Box", "candle")).toBe(false);
    // No term is not a filter.
    expect(matches("anything", "")).toBe(true);
  });
});

describe("which hit deserves to be first", () => {
  it("puts an exact name above everything", () => {
    expect(score("general", "general")).toBeGreaterThan(score("general-notes", "general"));
  });

  it("puts a prefix above a match buried in the middle", () => {
    // Typing "gen" should read along with `general`, not surface a channel
    // that merely contains the letters.
    expect(score("general", "gen")).toBeGreaterThan(score("autumn-general-notes", "gen"));
  });

  it("prefers an earlier match among substrings", () => {
    expect(score("a box here", "box")).toBeGreaterThan(score("a much longer line box", "box"));
  });

  it("prefers the shorter text when the match sits in the same place", () => {
    // Pinned because a reviewer read this tier's arithmetic backwards
    // (tinysweeper on #2245): the length term is subtracted, so a shorter text
    // subtracts less and ends up ahead. Position is held equal here so the only
    // thing under test is the length tie-break.
    const short = `${"x".repeat(10)}box`;
    const long = `${"x".repeat(10)}box${"y".repeat(300)}`;
    expect(score(short, "box")).toBeGreaterThan(score(long, "box"));
  });

  it("scores no match as zero, so callers can drop it", () => {
    expect(score("autumn", "candle")).toBe(0);
  });

  it("takes the best of several fields rather than the sum", () => {
    // One strong hit beats two weak ones: a channel whose NAME is `general`
    // outranks one that merely mentions it twice in its description.
    const exact = bestScore(["general", "nothing relevant"], "general");
    const buried = bestScore(["autumn", "general chat about general things"], "general");
    expect(exact).toBeGreaterThan(buried);
  });

  it("ignores empty fields", () => {
    expect(bestScore([null, undefined, "", "general"], "general")).toBeGreaterThan(0);
  });
});

describe("the snippet shown for a body match", () => {
  const long = `${"lorem ipsum ".repeat(40)}the autumn box ships friday${" and more text".repeat(20)}`;

  it("windows around the match rather than from the start", () => {
    const { excerpt } = excerptAround(long, "autumn box");
    // The whole reason this exists: a snippet that does not contain what was
    // searched for tells the operator nothing.
    expect(excerpt.toLowerCase()).toContain("autumn box");
  });

  it("marks where it cut", () => {
    const { excerpt } = excerptAround(long, "autumn box");
    expect(excerpt.startsWith("…")).toBe(true);
    expect(excerpt.endsWith("…")).toBe(true);
  });

  it("returns ranges that index into the excerpt it returned", () => {
    const { excerpt, ranges } = excerptAround(long, "autumn box");
    expect(ranges.length).toBeGreaterThan(0);
    const [range] = ranges;
    // The offsets must survive the leading ellipsis, or the highlight lands one
    // character off — which is exactly the bug this test exists to catch.
    expect(excerpt.slice(range[0], range[1]).toLowerCase()).toBe("autumn box");
  });

  it("collapses whitespace so a snippet is one readable line", () => {
    const { excerpt } = excerptAround("a\n\n  line   with\tgaps", "line");
    expect(excerpt).toBe("a line with gaps");
  });

  it("returns a short text whole, with no ellipsis and no ranges for no term", () => {
    expect(excerptAround("short enough", "")).toEqual({ excerpt: "short enough", ranges: [] });
  });
});

/**
 * Case folding that changes length is how a highlight lands on the wrong
 * letters (Codex P2 on #2245).
 */
describe("matching text that does not fold to the same length", () => {
  it("keeps offsets pointing at the match in the original string", () => {
    // `"İ".toLowerCase()` is two code units. Lowercasing the haystack shifts
    // every index after it by one, and the highlight slides off the match.
    const text = "İstanbul box";
    const [range] = matchRanges(text, "box");
    expect(text.slice(range[0], range[1])).toBe("box");
  });

  it("windows the excerpt on the match, not one character past it", () => {
    const { excerpt, ranges } = excerptAround("İstanbul box of candles", "box");
    const [range] = ranges;
    expect(excerpt.slice(range[0], range[1])).toBe("box");
  });
});

describe("a query whose spacing does not match the text's", () => {
  it("still finds and marks the term the score said was there", () => {
    // `score` sees the raw text and matches; the excerpt collapses whitespace,
    // so without collapsing the term too the snippet came back unmarked and
    // windowed from the start.
    const { excerpt, ranges } = excerptAround("keep the  foo  bar together", "foo  bar");
    expect(ranges.length).toBeGreaterThan(0);
    const [range] = ranges;
    expect(excerpt.slice(range[0], range[1])).toBe("foo bar");
  });
});

/**
 * Folding has to do both jobs at once: match regardless of case, and hand back
 * offsets into the string that will be rendered. The first attempt at the
 * Unicode fix did the second by giving up the first (CodeRabbit on #2245).
 */
describe("a term whose own case folding expands", () => {
  it("still matches when the match itself contains the character", () => {
    // `"İ".toLowerCase()` is two code units. Refusing to fold it keeps offsets
    // honest and makes the word case-SENSITIVE, so this stopped matching.
    expect(score("İstanbul", "istanbul")).toBeGreaterThan(0);
    expect(matches("İstanbul", "istanbul")).toBe(true);
  });

  it("marks the match in the original string, not one character off", () => {
    const text = "İstanbul";
    const [range] = matchRanges(text, "istanbul");
    expect(text.slice(range[0], range[1])).toBe("İstanbul");
  });

  it("keeps offsets right for a match after an expanding character", () => {
    const text = "İstanbul box";
    const [range] = matchRanges(text, "BOX");
    expect(text.slice(range[0], range[1])).toBe("box");
  });

  it("does not measure a surrogate pair in halves", () => {
    const text = "🕯 candle";
    const [range] = matchRanges(text, "candle");
    expect(text.slice(range[0], range[1])).toBe("candle");
  });
});

describe("accents, which normalising makes free", () => {
  it("finds an accented word from an unaccented query, and marks it whole", () => {
    expect(score("café", "cafe")).toBeGreaterThan(0);
    const text = "the café is open";
    const [range] = matchRanges(text, "cafe");
    // The offsets still index the original, accent and all.
    expect(text.slice(range[0], range[1])).toBe("café");
  });

  it("works in the other direction too", () => {
    expect(matches("cafe society", "café")).toBe(true);
  });
});
