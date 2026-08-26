import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { VIEWS, type View } from "@/lib/console-routes";
import { SETTINGS_PAGES } from "@/views/settings-pages";
import { NAMED_BY, SETTINGS_NAMED_BY, type Names } from "./support/routed-views";

/**
 * A routed view's header is the first thing it can render — in every state
 * (codex review on #1785).
 *
 * # The defect this stops
 *
 * `page-header-adoption.test.ts` asks whether a view's file *contains* a
 * `PageHeader`. That is satisfied by a file that returns early for its loading
 * or error state above the header, and eight files did:
 *
 *   Search, Hosting, Wallet, Invoicing, Finances, People, Ledgers, Manage lists
 *
 * Each rendered at least one state with no `h1` at all. Finances and the two
 * settings pairs were the serious ones: their error states are **terminal** —
 * nothing retries the read — so a screen reader got a page with no accessible
 * name and no way out of it, permanently, not for a moment during load.
 *
 * `settings-page-named-in-every-state.test.ts` catches this by rendering. It
 * is the better evidence and it cannot be the whole answer: it enumerates a
 * hand-written list of pages, and a hand-written list is exactly what keeps
 * missing whatever is not on it. Codex's phrasing — "the four settings pages
 * currently tested" — was the point.
 *
 * So this generalises, off the same routed-view set the adoption guard uses:
 * every view the router can reach, automatically, the day the route is added.
 *
 * # Why a scan is legitimate *here* specifically
 *
 * `page-header-adoption.test.ts` says not to teach it to follow branches, and
 * that still holds — a scan that guessed which branch runs would be wrong
 * invisibly. This asks a strictly weaker, decidable question: **is there any
 * `return` of JSX textually above the component's `PageHeader`?** It does not
 * care what the branch does or whether it can fire. A view that satisfies it
 * cannot have an unnamed state, because it has no return that reaches the DOM
 * before its header.
 *
 * It is therefore conservative in the safe direction: a header rendered in
 * every branch passes, and anything else needs a documented row below.
 */

const VIEWS_DIR = new URL("../../src/views", import.meta.url).pathname;

/**
 * Views whose named file legitimately returns before rendering a header.
 *
 * `handRolled` views are out of scope by construction — their heading is not a
 * `PageHeader` and `HAND_ROLLED` in the adoption test carries the reason.
 * Anything else needs a row here saying why, in the same register.
 */
const EARLY_RETURN_OK: Partial<Record<View, string>> = {
  team:
    "TeamView's first return is `<AgentDetailView …/>` — the teammate profile " +
    "route, which names itself with the heading `HAND_ROLLED` already allows. " +
    "It delegates rather than rendering an unnamed state.",
  company:
    "Same file as `team`, and the same delegating return.",
};

/** The file that renders this view's `PageHeader`, or null when it hand-rolls one. */
function headerFile(how: Names): string | null {
  return "pageHeader" in how ? how.pageHeader : null;
}

/**
 * Every `return` of JSX above the first `<PageHeader` in the component that
 * owns it.
 *
 * The component is found by walking back from the header to the nearest
 * top-level declaration, so returns inside helper components defined *earlier*
 * in the file are not counted — they are not this page's states.
 *
 * `return () =>` is an effect cleanup, not a render.
 */
function returnsAboveHeader(source: string): { line: number; text: string }[] {
  const lines = source.split("\n");
  const header = lines.findIndex((l) => l.includes("<PageHeader"));
  if (header < 0) return [];

  let start = 0;
  for (let i = header; i >= 0; i--) {
    if (/^(export )?(function|const) [A-Z][A-Za-z0-9_]*/.test(lines[i])) {
      start = i;
      break;
    }
  }

  const found: { line: number; text: string }[] = [];
  let i = start;
  while (i < header) {
    const line = lines[i];
    if (!/^\s{2,6}return\b/.test(line) || line.includes("return () =>")) {
      i += 1;
      continue;
    }
    const indent = line.length - line.trimStart().length;
    const block = [line];
    let j = i;
    if (!line.trimEnd().endsWith(";")) {
      j = i + 1;
      while (j < lines.length) {
        block.push(lines[j]);
        if (new RegExp(`^\\s{${indent}}\\);\\s*$`).test(lines[j])) break;
        j += 1;
      }
    }
    const text = block.join("\n");
    if (text.includes("<") && !text.includes("<PageHeader")) {
      const first = block.slice(1).find((l) => l.trim()) ?? line;
      found.push({ line: i + 1, text: first.trim().slice(0, 80) });
    }
    i = j + 1;
  }
  return found;
}

describe("a routed view's header precedes every return it can make (#1785)", () => {
  it("checks one file per routed view, and finds all of them", () => {
    // Without this a typo in a path would silently check nothing, which is the
    // failure mode every guard in this directory is written against.
    const files = VIEWS.map((v) => headerFile(NAMED_BY[v])).filter((f): f is string => f !== null);
    expect(files.length).toBeGreaterThan(12);
    for (const file of files) {
      expect(() => readFileSync(`${VIEWS_DIR}/${file}`, "utf8"), file).not.toThrow();
    }
  });

  it("has no routed view returning JSX before it renders its header", () => {
    const offenders = VIEWS.flatMap((view) => {
      const file = headerFile(NAMED_BY[view]);
      if (file === null || view in EARLY_RETURN_OK) return [];
      const source = readFileSync(`${VIEWS_DIR}/${file}`, "utf8");
      return returnsAboveHeader(source).map(
        (r) => `${view} (${file}:${r.line}) returns before its header: ${r.text}`,
      );
    });

    expect(
      offenders,
      `A routed view returns something before it renders its page header, so ` +
        `that state has no h1 and a screen reader cannot announce the page.\n` +
        `Read the header into a const above the conditionals and render it in ` +
        `each branch — see SearchView or FinancesView for the shape. Guard any ` +
        `prop that needs data which has not arrived (WalletView's environment ` +
        `badge).\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has no settings page returning JSX before it renders its header", () => {
    // Settings is one routed view over ten bookmarkable addresses, so the
    // routed-view sweep above cannot see any of them — `#/settings/people`
    // was one of the two this review found, and it is not a `View`.
    const offenders = SETTINGS_PAGES.flatMap(({ id }) => {
      const file = SETTINGS_NAMED_BY[id];
      const source = readFileSync(`${VIEWS_DIR}/${file}`, "utf8");
      return returnsAboveHeader(source).map(
        (r) => `settings/${id} (${file}:${r.line}) returns before its header: ${r.text}`,
      );
    });

    expect(
      offenders,
      `A settings page returns something before it renders its page header. ` +
        `Same fix as the routed views above.\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has every exemption naming a view that still exists and still returns early", () => {
    // An exemption that stopped applying is a rule nobody is being held to.
    const stale = Object.keys(EARLY_RETURN_OK)
      .map((view) => {
        const file = headerFile(NAMED_BY[view as View]);
        if (file === null) return `${view} is exempt but hand-rolls its heading`;
        const source = readFileSync(`${VIEWS_DIR}/${file}`, "utf8");
        return returnsAboveHeader(source).length > 0
          ? null
          : `${view} is exempt but no longer returns before its header — drop the row`;
      })
      .filter((line): line is string => line !== null);

    expect(stale, stale.join("\n")).toEqual([]);
  });
});
