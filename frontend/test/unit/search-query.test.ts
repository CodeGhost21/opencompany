import { describe, expect, it } from "vitest";

import {
  isScopedMessageSearch,
  parseSearchQuery,
  scopeIsNamed,
  withScopeName,
} from "@/search/query";

/**
 * The search grammar, away from the modal that renders it.
 *
 * Every rule here is one an operator meets within seconds of pressing the key,
 * and each fails silently if it is wrong: a scope that does not open leaves the
 * picker shut for no visible reason, and a term that swallows the scope name
 * searches the whole company for the word "priya".
 */
describe("parsing what was typed", () => {
  it("treats a bare term as a search of everything", () => {
    const q = parseSearchQuery("autumn box");
    expect(q.scope).toBeNull();
    expect(q.term).toBe("autumn box");
    expect(q.isEmpty).toBe(false);
  });

  it("opens a scope on the prefix alone, before a name is typed", () => {
    // The picker has to appear on the keypress. Waiting for a first letter is
    // what makes an `@` feel broken for the moment before it works.
    const q = parseSearchQuery("@");
    expect(q.scope).toEqual({ kind: "person", name: "" });
    expect(scopeIsNamed(q)).toBe(false);
  });

  it("reads @ as a person, # as a channel and / as a file", () => {
    expect(parseSearchQuery("@priya").scope).toEqual({ kind: "person", name: "priya" });
    expect(parseSearchQuery("#general").scope).toEqual({ kind: "channel", name: "general" });
    expect(parseSearchQuery("/brief").scope).toEqual({ kind: "file", name: "brief" });
  });

  it("gives the whole remainder to a file scope rather than splitting it", () => {
    // `/autumn brief` is looking for a file called something like "autumn
    // brief" — not for "brief" inside a file called "autumn". A file scope
    // names the things themselves, so there is nothing to narrow within.
    const q = parseSearchQuery("/autumn brief");
    expect(q.scope).toEqual({ kind: "file", name: "autumn brief" });
    expect(q.term).toBe("");
    expect(isScopedMessageSearch(q)).toBe(false);
  });

  it("splits the scope's name from the term at the first space", () => {
    const q = parseSearchQuery("@priya autumn box");
    expect(q.scope).toEqual({ kind: "person", name: "priya" });
    // Multi-word terms survive: only the FIRST space divides name from term.
    expect(q.term).toBe("autumn box");
  });

  it("has a named scope but no term while the name is still being typed", () => {
    const q = parseSearchQuery("@priya");
    expect(scopeIsNamed(q)).toBe(true);
    expect(q.term).toBe("");
    // Choosing a person is not yet a search — every message they ever sent is
    // not what was asked for.
    expect(isScopedMessageSearch(q)).toBe(false);
  });

  it("asks for messages only once a scope and a term are both present", () => {
    expect(isScopedMessageSearch(parseSearchQuery("@priya box"))).toBe(true);
    expect(isScopedMessageSearch(parseSearchQuery("#autumn-launch box"))).toBe(true);
    expect(isScopedMessageSearch(parseSearchQuery("@ box"))).toBe(false);
  });

  it("lowercases what it matches on, and keeps the raw string intact", () => {
    const q = parseSearchQuery("  @Priya Autumn BOX  ");
    expect(q.scope?.name).toBe("priya");
    expect(q.term).toBe("autumn box");
    // The input restores exactly what was typed, spaces and all.
    expect(q.raw).toBe("  @Priya Autumn BOX  ");
  });

  it("reads whitespace as nothing typed rather than as a term", () => {
    for (const raw of ["", "   ", "\t"]) {
      const q = parseSearchQuery(raw);
      expect(q.isEmpty, `${JSON.stringify(raw)} should be empty`).toBe(true);
      expect(q.scope).toBeNull();
      expect(q.term).toBe("");
    }
  });

  it("leaves a prefix in the middle of a term alone", () => {
    // Only a LEADING prefix opens a scope. An email address or a "#1" is a
    // term, and scoping on it would search a channel nobody named.
    const q = parseSearchQuery("ada@example.com");
    expect(q.scope).toBeNull();
    expect(q.term).toBe("ada@example.com");
  });
});

describe("choosing from the picker", () => {
  it("rewrites the input to the chosen name, ready for a term", () => {
    // The trailing space is load-bearing: it is what moves the caret out of the
    // name and into the term, so the next keystroke searches rather than
    // renaming.
    expect(withScopeName(parseSearchQuery("@pri"), "priya")).toBe("@priya ");
    expect(withScopeName(parseSearchQuery("#aut"), "autumn-launch")).toBe("#autumn-launch ");
  });

  it("keeps the scope kind that was open", () => {
    expect(withScopeName(parseSearchQuery("#gen"), "general").startsWith("#")).toBe(true);
    expect(withScopeName(parseSearchQuery("@gen"), "general").startsWith("@")).toBe(true);
  });
});
