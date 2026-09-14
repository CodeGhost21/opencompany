// Matching and ordering, with no idea what it is matching.
//
// Pure and total: strings in, ranges and scores out. The two jobs here are the
// ones a search gets judged on and the ones easiest to get quietly wrong —
// where a term matched (so it can be highlighted without building HTML) and
// which hit deserves to be first.

import type { MatchRange } from "./types";

/**
 * Every place `term` occurs in `text`, case-insensitively.
 *
 * Offsets into the ORIGINAL string, so the caller slices the text the operator
 * will actually read rather than a lowercased copy of it. That is what keeps
 * highlighting honest: the component renders `text.slice(...)` inside a
 * `<mark>`, and never re-inserts the term as markup — a search box that builds
 * HTML out of user input is one paste away from being an injection.
 *
 * Empty for an empty term: "everything matched" and "nothing was asked" are
 * different answers, and highlighting the whole string is neither.
 */
export function matchRanges(text: string, term: string): MatchRange[] {
  if (!term || !text) return [];
  const haystack = fold(text);
  const needle = fold(term).folded;
  const ranges: MatchRange[] = [];
  let from = 0;
  for (;;) {
    const at = haystack.folded.indexOf(needle, from);
    if (at === -1) break;
    // Mapped back, so the caller slices the string it will actually render.
    ranges.push([haystack.at[at], haystack.at[at + needle.length]]);
    from = at + needle.length;
    // A pathological term (one character, long text) would otherwise build a
    // range per character. Nothing on screen shows more than a few.
    if (ranges.length >= MAX_RANGES) break;
  }
  return ranges;
}

/** Enough to highlight a line; past this the extra marks say nothing. */
const MAX_RANGES = 20;

/**
 * Case folded, with the way back to the original's offsets.
 *
 * `String.prototype.toLowerCase` is not length-preserving — `"İ".toLowerCase()`
 * is two code units — so an index into the folded string is not an index into
 * the text the operator reads. Left alone that drifts a highlight one character
 * late (`İstanbul box` marking `ox`); *refusing* to fold those characters, as
 * this first did, instead makes them case-sensitive, so `İstanbul` stops
 * matching `istanbul` altogether.
 *
 * So neither: fold everything, and carry `at[i]` — the original offset that
 * folded character `i` came from. One trailing entry for the end, so a range's
 * exclusive end maps too.
 */
function fold(value: string): { folded: string; at: number[] } {
  let folded = "";
  const at: number[] = [];
  let index = 0;
  // By code point, not by unit: a surrogate pair is one character and must not
  // be folded or measured in halves.
  for (const character of value) {
    // Lowercased, then decomposed, then stripped of the combining marks that
    // decomposition exposes. All three are needed for one word to match
    // itself: `"İ".toLowerCase()` is `i` plus a combining dot, so `İstanbul`
    // never matched `istanbul` — not here, and not under the plain
    // `toLowerCase` this replaced. Accents fall out of the same pass, so
    // `cafe` now finds `café`, which is the behaviour a search box is expected
    // to have anyway.
    const normalized = character
      .toLowerCase()
      .normalize("NFD")
      .replace(COMBINING_MARKS, "");
    for (let i = 0; i < normalized.length; i += 1) at.push(index);
    folded += normalized;
    index += character.length;
  }
  at.push(value.length);
  return { folded, at };
}

/**
 * The marks NFD splits off a composed character.
 *
 * Dropped rather than kept so that a character and its decomposition compare
 * equal — which is the whole point of normalising before a substring search.
 */
const COMBINING_MARKS = /\p{M}/gu;

/** Whether `text` contains `term` at all, case-insensitively. */
export function matches(text: string, term: string): boolean {
  if (!term) return true;
  return fold(text).folded.includes(fold(term).folded);
}

/**
 * How well `text` answers `term`, higher being better.
 *
 * Three tiers, because they are three genuinely different qualities of hit and
 * an operator can feel the difference:
 *
 * - **Exact** — they typed the whole name. Nothing should outrank it.
 * - **Prefix** — they are typing the name. `gen` should put `general` above
 *   `autumn-general-notes`, which is what makes type-ahead feel like it is
 *   reading along rather than guessing.
 * - **Substring** — it is in there somewhere, and the earlier the better.
 *
 * Shorter text breaks ties within a tier: given two substring hits, the one
 * where the term is more of the whole is the more likely answer.
 *
 * Returns 0 for no match, which callers use as "drop it" — never as a weak hit.
 */
export function score(text: string, term: string): number {
  if (!term) return 1;
  const haystack = fold(text).folded;
  const needle = fold(term).folded;
  const at = haystack.indexOf(needle);
  if (at === -1) return 0;

  if (haystack === needle) return 1000;
  if (at === 0) return 800 - Math.min(haystack.length, 200);
  // Earlier is better, and each character of distance costs one point — so
  // position never outweighs the tier above it.
  return 600 - Math.min(at, 200) - Math.min(haystack.length, 100) / 100;
}

/**
 * The best score across several fields, so a hit on any of them counts.
 *
 * A channel matches on its name or its description; an agent on their name or
 * their role. Taking the max rather than the sum keeps one strong hit ahead of
 * two weak ones.
 */
export function bestScore(fields: readonly (string | null | undefined)[], term: string): number {
  let best = 0;
  for (const field of fields) {
    if (!field) continue;
    const value = score(field, term);
    if (value > best) best = value;
  }
  return best;
}

/**
 * A window of `text` around its first match, for a message or a file body.
 *
 * The match is the point, so the window is placed around it rather than taken
 * from the start — a 400-character message whose only mention of the term is at
 * the end, excerpted from character zero, shows the operator a snippet that
 * does not contain what they searched for.
 *
 * Returns the slice and the ranges *within that slice*, already rebased, so the
 * caller never has to do offset arithmetic to highlight it.
 */
export function excerptAround(
  text: string,
  term: string,
  width = 160,
): { excerpt: string; ranges: MatchRange[] } {
  const collapsed = text.replace(/\s+/g, " ").trim();
  // The term is collapsed too, or a query carrying a double space matches in
  // `score` (which sees the raw text) and then cannot be found here, leaving an
  // unrelated leading excerpt with nothing marked in it.
  term = term.replace(/\s+/g, " ").trim();
  if (!term) {
    return {
      excerpt: collapsed.length > width ? `${collapsed.slice(0, width)}…` : collapsed,
      ranges: [],
    };
  }

  // Found in the folded string, then mapped back: every slice below indexes
  // `collapsed`, and the two only share offsets while nothing expanded.
  const haystack = fold(collapsed);
  const foundAt = haystack.folded.indexOf(fold(term).folded);
  const at = foundAt === -1 ? -1 : haystack.at[foundAt];
  if (at === -1) {
    return {
      excerpt: collapsed.length > width ? `${collapsed.slice(0, width)}…` : collapsed,
      ranges: [],
    };
  }

  // A third of the window before the match, so there is context on both sides
  // and the match is not flush against the left edge.
  const lead = Math.floor(width / 3);
  const start = Math.max(0, at - lead);
  const end = Math.min(collapsed.length, start + width);
  const body = collapsed.slice(start, end);
  const excerpt = `${start > 0 ? "…" : ""}${body}${end < collapsed.length ? "…" : ""}`;
  // Ranges come back relative to `body`, and the only thing between `body` and
  // `excerpt` is the leading ellipsis — so that is the whole of the shift.
  const shift = start > 0 ? 1 : 0;
  const ranges = matchRanges(body, term).map(
    ([from, to]) => [from + shift, to + shift] as MatchRange,
  );
  return { excerpt, ranges };
}
