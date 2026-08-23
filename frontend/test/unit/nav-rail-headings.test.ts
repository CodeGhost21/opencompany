import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * A navigation rail labels itself; it does not put headings in the page outline.
 *
 * The Settings and Finance rails each captioned themselves with an `<h2>` and,
 * in Settings' case, an `<h3>` per link group. Both rails render *before* the
 * sub-page they navigate, so on `#/settings/connections` or `#/finances` a
 * screen reader walking the outline met a section-level heading before the
 * page's own `h1` — a malformed order that counting `h1`s cannot detect, since
 * the count is right and only the position is wrong (issue #1392).
 *
 * Nothing was lost by demoting them. Each `<nav>` already carries an
 * `aria-label`, which is what names it for assistive technology, and the
 * Settings groups are named by `aria-labelledby` — that resolves against any
 * element, so a `div` names the group exactly as the `h3` did. The captions stay
 * visible; they simply stop being part of the document outline they sit ahead
 * of.
 *
 * A source guard rather than a render test, in the idiom of
 * `dialog-width-override`: the failure is invisible below the level of the whole
 * page. Each rail is unimpeachable read on its own — an `h2` captioning a rail
 * is only wrong once you know an `h1` renders after it.
 */
const SRC = new URL("../../src", import.meta.url).pathname;

/** Every `.tsx` under `src/`. */
function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) sources(path, out);
    else if (name.endsWith(".tsx")) out.push(path);
  }
  return out;
}

/**
 * The `<h1>`–`<h6>` opened between a `<nav>` and its close.
 *
 * Deliberately shallow: it does not track nesting, so a `<nav>` holding another
 * element that closes first would end the span early. No rail in this codebase
 * nests one, and erring toward a short span only ever under-reports — it cannot
 * invent an offender.
 */
function headingsInsideNav(source: string): string[] {
  const found: string[] = [];
  for (const open of source.matchAll(/<nav\b/g)) {
    const close = source.indexOf("</nav>", open.index);
    if (close === -1) continue;
    for (const [, level] of source.slice(open.index, close).matchAll(/<h([1-6])[\s>]/g)) {
      found.push(`h${level}`);
    }
  }
  return found;
}

describe("Navigation rails", () => {
  it("carry no headings, so none of them precedes a page's h1", () => {
    const offenders: string[] = [];
    for (const path of sources(SRC)) {
      const inside = headingsInsideNav(readFileSync(path, "utf8"));
      if (inside.length > 0) {
        offenders.push(`${relative(SRC, path)} puts ${inside.join(", ")} inside a <nav>`);
      }
    }

    expect(
      offenders,
      `a rail renders before the page it navigates, so a heading in one lands ahead of that page's h1:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("still name themselves, so demoting those captions cost nothing", () => {
    for (const name of ["SettingsSection", "finance/FinanceSection"]) {
      const source = readFileSync(`${SRC}/views/${name}.tsx`, "utf8");
      expect(source, `${name} must name its <nav>`).toMatch(/<nav\s+aria-label="/);
    }

    // The Settings groups are named by reference rather than by a heading.
    const settings = readFileSync(`${SRC}/views/SettingsSection.tsx`, "utf8");
    expect(settings).toContain("aria-labelledby={`settings-group-${group.id}`}");
    expect(settings).toContain("id={`settings-group-${group.id}`}");
  });
});
