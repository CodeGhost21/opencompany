import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { VIEWS, type View } from "@/lib/console-routes";
import { SETTINGS_PAGES } from "@/views/settings-pages";
import { NAMED_BY, SETTINGS_NAMED_BY, type Leaf } from "./support/routed-views";

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
 * **Empty, and that is the point.** It had two rows — `team` and `company` —
 * because `TeamView`'s first return is `<AgentDetailView …/>`. A route-level
 * exemption was the wrong instrument (codex review, #1785): it switched the
 * scan off for the whole route, so every *other* early return under it would
 * also have gone unseen. Delegation is now recognised structurally instead —
 * see `delegatesTo` — and both rows are gone.
 *
 * `handRolled` leaves are out of scope by construction: their heading is not a
 * `PageHeader`, and `HAND_ROLLED` in the adoption test carries the reason. A
 * row here would need to say why a *state* cannot be named, not why a route is
 * special.
 */
const EARLY_RETURN_OK: Partial<Record<View, string>> = {};

/**
 * Every component name this system holds to a heading — the leaves of every
 * route plus every settings page.
 *
 * Global rather than per-route on purpose. `TeamView` returns
 * `<AgentDetailView …/>` for `#/team/<id>`, and `#/company` renders the same
 * file with `sub={null}` so that branch is unreachable from there; a per-route
 * set would call the identical line delegation under `team` and an offence
 * under `company`. What the check actually asks is "does this return hand the
 * screen to a component that is itself guarded", and that has one answer.
 */
const GUARDED_COMPONENTS: string[] = [
  ...Object.values(NAMED_BY).flatMap((list) =>
    list.map((leaf) => ("pageHeader" in leaf ? leaf.pageHeader : leaf.handRolled)),
  ),
  ...Object.values(SETTINGS_NAMED_BY),
].map((file) => file.slice(file.lastIndexOf("/") + 1).replace(/\.tsx$/, ""));

/**
 * Whether a returned block is nothing but `<SomeLeaf …/>`.
 *
 * `CompanyView` and `TeamView` dispatch: their early return hands the whole
 * screen to another component this route already enumerates, which is not an
 * unnamed state — that component's own row is what holds it to a heading. Any
 * *other* early return still counts, so the route keeps its cover.
 */
function delegatesTo(block: string): boolean {
  const first = block.match(/<([A-Z][A-Za-z0-9_]*)/);
  return first !== null && GUARDED_COMPONENTS.includes(first[1]);
}

/** The file that renders this leaf's `PageHeader`, or null when it hand-rolls one. */
function headerFile(leaf: Leaf): string | null {
  return "pageHeader" in leaf ? leaf.pageHeader : null;
}

/**
 * Names of local `const`s in this component that hold a `PageHeader`.
 *
 * The fix for a multi-state page is to read the header into a const once and
 * render `{header}` in each branch — three copies of a page's own name is how
 * the console got twelve of them. A check that only recognised the literal tag
 * would call every one of those branches an offence, so it recognises the
 * binding too.
 */
function headerConsts(body: string): string[] {
  return [...body.matchAll(/const (\w+) = \(?\s*<PageHeader/g)].map((m) => m[1]);
}

/**
 * Whether a returned block is JSX at all.
 *
 * Anchored on what follows `return`, not on the block containing a `<`
 * anywhere: a multi-line `return cond ? "a" : "b";` inside the component picks
 * up a `<` from the lines the block scan swept past and was reported as an
 * unnamed render.
 *
 * Leading commentary is stripped first. Three of these returns open with a
 * comment explaining the state — `Overview`, `WorkflowsView`, `MemoryView` —
 * and a check that looked only at the first character skipped all three
 * silently, which is a hole shaped exactly like the one this file exists to
 * close. A bare `{` is still not JSX: `return { millis: … }` is an object.
 */
function isJsx(block: string): boolean {
  const after = block
    .replace(/^\s*return\s*/, "")
    .replace(/^\(\s*/, "")
    // Leading commentary, in any of the three forms these files use.
    .replace(/^(\s*(\/\/[^\n]*|\/\*[\s\S]*?\*\/|\{\/\*[\s\S]*?\*\/\})\s*)+/, "")
    .trimStart();
  return after.startsWith("<");
}

/**
 * Whether this state renders a page heading: the component, the const holding
 * it, or a hand-rolled `<h1>`.
 *
 * `<h1>` counts because the question here is whether the state has a name at
 * all. *Who* may hand-roll one, and how many, is `HAND_ROLLED`'s business in
 * `page-header-adoption.test.ts` — two rules, one each, rather than both
 * half-enforced in two places.
 */
function hasHeading(block: string, consts: string[]): boolean {
  return (
    block.includes("<PageHeader") ||
    block.includes("<h1") ||
    consts.some((name) => block.includes(`{${name}}`))
  );
}

/**
 * Every JSX `return` in the component that owns the header, and whether that
 * return carries a heading.
 *
 * **This checks every return, not the ones above the first header.** The
 * earlier version stopped at the first `<PageHeader` occurrence, which is a
 * per-*file* question — "does this file contain a header" — while the defect
 * is per-*state*. Five findings in a row were a state inside a file that
 * already contained a header somewhere: `SearchView`, then Finances and
 * People, then Chat, Team and Company, then `TaskDetailView`, whose
 * `notFound` return got one while its main return — loading, and a non-404
 * failure that leaves `detail` null — did not. Patching the named file each
 * time produced the next finding, because the check could not tell the two
 * questions apart.
 *
 * The component is found by walking back from the first header to the nearest
 * top-level declaration, so returns inside helper components defined elsewhere
 * in the file are not counted — they are not this page's states.
 *
 * `return () =>` is an effect cleanup, and a `return` with no `<` is a value,
 * not a render.
 */
function returnsWithoutHeader(source: string): { line: number; text: string }[] {
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
  // The component ends where the next top-level declaration begins.
  let end = lines.length;
  for (let i = Math.max(start + 1, header + 1); i < lines.length; i++) {
    if (/^(export )?(function|const) [A-Z][A-Za-z0-9_]*/.test(lines[i])) {
      end = i;
      break;
    }
  }

  const consts = headerConsts(lines.slice(start, end).join("\n"));

  const found: { line: number; text: string }[] = [];
  let i = start;
  while (i < end) {
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
      while (j < end) {
        block.push(lines[j]);
        if (new RegExp(`^\\s{${indent}}\\);\\s*$`).test(lines[j])) break;
        j += 1;
      }
    }
    const text = block.join("\n");
    if (isJsx(text) && !hasHeading(text, consts) && !delegatesTo(text)) {
      const first = block.slice(1).find((l) => l.trim()) ?? line;
      found.push({ line: i + 1, text: first.trim().slice(0, 80) });
    }
    i = j + 1;
  }
  return found;
}

describe("every state of a routed view renders a heading (#1785)", () => {
  it("checks one file per routed view, and finds all of them", () => {
    // Without this a typo in a path would silently check nothing, which is the
    // failure mode every guard in this directory is written against.
    const files = VIEWS.flatMap((v) => NAMED_BY[v].map(headerFile)).filter(
      (f): f is string => f !== null,
    );
    expect(files.length).toBeGreaterThan(12);
    for (const file of files) {
      expect(() => readFileSync(`${VIEWS_DIR}/${file}`, "utf8"), file).not.toThrow();
    }
  });

  it("has no routed view returning JSX without a heading in it", () => {
    const offenders = VIEWS.flatMap((view) => {
      if (view in EARLY_RETURN_OK) return [];
      return NAMED_BY[view].flatMap((leaf) => {
        const file = headerFile(leaf);
        if (file === null) return [];
        const source = readFileSync(`${VIEWS_DIR}/${file}`, "utf8");
        return returnsWithoutHeader(source).map(
          (r) => `${view} (${file}:${r.line}) returns with no heading in it: ${r.text}`,
        );
      });
    });

    expect(
      offenders,
      `A routed view has a state that renders no page header, so that state ` +
        `has no h1 and a screen reader cannot announce the page.\n` +
        `Read the header into a const above the conditionals and render it in ` +
        `each branch — see SearchView or FinancesView for the shape. Guard any ` +
        `prop that needs data which has not arrived (WalletView's environment ` +
        `badge).\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has no settings page returning JSX without a heading in it", () => {
    // Settings is one routed view over ten bookmarkable addresses, so the
    // routed-view sweep above cannot see any of them — `#/settings/people`
    // was one of the two this review found, and it is not a `View`.
    const offenders = SETTINGS_PAGES.flatMap(({ id }) => {
      const file = SETTINGS_NAMED_BY[id];
      const source = readFileSync(`${VIEWS_DIR}/${file}`, "utf8");
      return returnsWithoutHeader(source).map(
        (r) => `settings/${id} (${file}:${r.line}) returns with no heading in it: ${r.text}`,
      );
    });

    expect(
      offenders,
      `A settings page has a state that renders no page header. ` +
        `Same fix as the routed views above.\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has every exemption naming a view that still exists and still returns early", () => {
    // An exemption that stopped applying is a rule nobody is being held to.
    const stale = Object.keys(EARLY_RETURN_OK)
      .map((view) => {
        const early = NAMED_BY[view as View].flatMap((leaf) => {
          const file = headerFile(leaf);
          if (file === null) return [];
          return returnsWithoutHeader(readFileSync(`${VIEWS_DIR}/${file}`, "utf8"));
        });
        return early.length > 0
          ? null
          : `${view} is exempt but every state now renders a heading — drop the row`;
      })
      .filter((line): line is string => line !== null);

    expect(stale, stale.join("\n")).toEqual([]);
  });
});
