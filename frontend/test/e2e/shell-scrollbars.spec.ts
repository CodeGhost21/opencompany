import { expect, test, type Page } from "@playwright/test";

/**
 * The console's scrollbars are themed, thin, and only bright while a pane is
 * moving (issues #1109 and #1178).
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
    const panes = [...document.querySelectorAll<HTMLElement>("*")].filter((el) => {
      const overflow = getComputedStyle(el).overflowY;
      return (
        (overflow === "auto" || overflow === "scroll") && el.scrollHeight > el.clientHeight + 40
      );
    });
    // Deepest first: an outer pane that also scrolls would let the assertion
    // about *which* element gets marked pass for the wrong reason.
    const pane = panes.at(-1);
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

test("scroll panes carry the console's own thin gutter, not the platform's", async ({ page }) => {
  const scroller = await openSettings(page);

  const gutter = await page.locator(scroller).evaluate((el: HTMLElement) => el.offsetWidth - el.clientWidth);

  // 10px is `--scrollbar-size`. Chromium's own bar is 15px, so this fails if
  // the `::-webkit-scrollbar` block is dropped — or disabled, which is what
  // setting `scrollbar-width`/`scrollbar-color` alongside it does.
  expect(gutter).toBe(10);
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
