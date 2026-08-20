import { expect, test, type Page } from "@playwright/test";

/**
 * The console's scrollbars are themed, thin, and only bright while a pane is
 * moving (issue #1109).
 *
 * # This is characterisation cover, and it passes on `main`
 *
 * Nothing here is new behaviour. #1109 landed the themed scrollbars and the
 * `data-scrolling` mark; what it did not land was a browser test, and every
 * property below is one only a browser can see. It is written now because
 * issue #1178 rebuilt the shell those panes live in and asked for the
 * scrollbars to be verified — so the honest thing is a test that describes what
 * already ships, rather than one that claims to prove a change it did not make.
 *
 * Three properties, and each one is invisible to a unit test:
 *
 *   1. **The gutter is the console's, not the platform's.** Chromium paints a
 *      15px system scrollbar; `index.css` narrows every scroller in the app to
 *      10px through `::-webkit-scrollbar`. That rule set is silently dropped in
 *      Chromium the moment anything sets the standard `scrollbar-width` /
 *      `scrollbar-color` properties on the same element — the trap the
 *      stylesheet warns about at length — and nothing but a real browser can
 *      tell you it happened.
 *   2. **The geometry never changes.** Only the thumb's *colour* animates. If a
 *      future change hides the bar by collapsing its width, every scroll would
 *      reflow the content beside it. Measured here rather than argued.
 *   3. **`data-scrolling` lands on the pane that actually scrolled.** `scroll`
 *      does not bubble, so one capture-phase listener at the document covers
 *      every pane including ones mounted later (`lib/scroll-activity.ts`). The
 *      failure mode this rules out is `:hover`'s: a mark that spreads to every
 *      ancestor up to `<html>`, lighting every nested scroller at once.
 */

/** The first-run tour opens a modal over a fresh console and eats every click. */
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

/** Mark the deepest pane on the page that can actually scroll, and report it. */
async function findScroller(page: Page): Promise<string> {
  const id = await page.evaluate(() => {
    // Scoped to the content column, and that is load-bearing. A document-wide
    // query walks the sidebar first, and `SidebarContent` is `.no-scrollbar` —
    // which sets `scrollbar-width: none`, the very standard property this spec
    // asserts is left alone. The moment the nav column overflowed, the test
    // would fail on a pane that is *meant* to opt out.
    const root = document.querySelector('[data-slot="sidebar-inset"]');
    if (!root) return null;
    const panes = [...root.querySelectorAll<HTMLElement>("*")].filter((el) => {
      const overflow = getComputedStyle(el).overflowY;
      return (
        (overflow === "auto" || overflow === "scroll") && el.scrollHeight > el.clientHeight + 40
      );
    });
    // The innermost one — a pane that contains another candidate is an outer
    // wrapper, and marking it would let the assertion about *which* element
    // gets the mark pass for the wrong reason.
    const pane = panes.find((el) => !panes.some((other) => other !== el && el.contains(other)));
    if (!pane) return null;
    pane.setAttribute("data-e2e-scroller", "");
    return "[data-e2e-scroller]";
  });
  expect(id, "Settings should have a pane with more content than fits").not.toBeNull();
  return id!;
}

async function openSettings(page: Page) {
  await page.goto("/#/settings");
  await expect(page.getByRole("button", { name: "Change theme" })).toBeVisible({ timeout: 30_000 });
  return findScroller(page);
}

test("scroll panes are styled by the console, not by the platform", async ({ page }) => {
  const scroller = await openSettings(page);

  const styling = await page.evaluate((selector) => {
    // Every rule in the cascade, flattened out of the `@layer` / `@media` /
    // `@supports` blocks Tailwind v4 emits them inside.
    const selectors: string[] = [];
    const declarations = new Map<string, string>();
    const walk = (rules: CSSRuleList) => {
      for (const rule of rules) {
        if (rule instanceof CSSStyleRule) {
          selectors.push(rule.selectorText);
          if (rule.selectorText.includes("::-webkit-scrollbar")) {
            for (const property of rule.style) {
              declarations.set(`${rule.selectorText}|${property}`, rule.style.getPropertyValue(property));
            }
          }
        }
        const nested = (rule as CSSGroupingRule).cssRules;
        if (nested) walk(nested);
      }
    };
    for (const sheet of document.styleSheets) {
      try {
        walk(sheet.cssRules);
      } catch {
        // A cross-origin sheet cannot be read. None of ours are.
      }
    }

    const pane = document.querySelector(selector)!;
    const paneStyle = getComputedStyle(pane);
    const root = getComputedStyle(document.documentElement);
    return {
      selectors,
      declarations: [...declarations],
      // The standard properties, which must stay untouched — see below.
      scrollbarWidth: paneStyle.scrollbarWidth,
      scrollbarColor: paneStyle.scrollbarColor,
      size: root.getPropertyValue("--scrollbar-size").trim(),
      rest: root.getPropertyValue("--scrollbar-thumb-rest").trim(),
      active: root.getPropertyValue("--scrollbar-thumb-active").trim(),
    };
  }, scroller);

  // The gutter is 10px wide, and the width comes from a token rather than a
  // literal. Deliberately not measured as `offsetWidth - clientWidth`: macOS
  // Chromium paints an overlay scrollbar that reserves no layout space, so that
  // measurement reads 0 there and 10 on Linux CI — a test that only fails on
  // the machine nobody runs it on.
  expect(styling.size).toBe("10px");
  expect(styling.declarations).toContainEqual(["::-webkit-scrollbar|width", "var(--scrollbar-size)"]);

  // The rule that makes the bar lift while the pane is moving. Without it the
  // `data-scrolling` mark the next test asserts would be stamped on nothing.
  expect(styling.selectors).toContain("[data-scrolling]::-webkit-scrollbar-thumb");

  // Rest and active are different weights of the same colour. Equal values
  // would mean the bar never changes, which is the always-on platform bar this
  // replaced.
  expect(styling.rest).not.toBe("");
  expect(styling.active).not.toBe("");
  expect(styling.rest).not.toBe(styling.active);

  // The trap `index.css` warns about at length: Chromium honours
  // `::-webkit-scrollbar` ONLY while the standard properties are untouched. Set
  // either one on a scroller and it throws the custom scrollbar away and paints
  // its own, silently — every rule asserted above would still be in the sheet
  // and none of it would reach the screen.
  expect(styling.scrollbarWidth).toBe("auto");
  expect(styling.scrollbarColor).toBe("auto");
});

test("scrolling marks the pane that moved, and only until it settles", async ({ page }) => {
  const scroller = await openSettings(page);
  const pane = page.locator(scroller);

  await expect(pane).not.toHaveAttribute("data-scrolling", "");

  const before = await pane.evaluate((el: HTMLElement) => ({ client: el.clientWidth, offset: el.offsetWidth }));

  await pane.evaluate((el) => el.scrollBy(0, 240));
  await expect(pane).toHaveAttribute("data-scrolling", "");

  // The mark is on the pane that scrolled and nowhere else. `:hover` — the
  // usual substitute for a "is scrolling" state CSS does not have — would light
  // every ancestor of the pointer instead.
  const marked = await page.evaluate(() =>
    [...document.querySelectorAll("[data-scrolling]")].map(
      (el) => el.getAttribute("data-e2e-scroller") !== null,
    ),
  );
  expect(marked).toEqual([true]);

  // Nothing moved but the colour: the gutter is reserved at rest and reserved
  // while scrolling, so the content beside it never reflows.
  const during = await pane.evaluate((el: HTMLElement) => ({ client: el.clientWidth, offset: el.offsetWidth }));
  expect(during).toEqual(before);

  // And it fades: the mark is cleared once the pane has been still. The idle
  // beat is 700ms (`startScrollActivity`); the wait is generous so a slow CI
  // machine does not read as a broken timer.
  await expect(pane).not.toHaveAttribute("data-scrolling", "", { timeout: 5_000 });
});
