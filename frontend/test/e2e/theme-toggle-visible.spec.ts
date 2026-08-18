import { expect, test, type Page } from "@playwright/test";

/**
 * Regression proof for #922 — the theme switch must render a visible glyph
 * inside its own button, in both themes, however far down Settings is scrolled.
 *
 * `ThemeToggle` stacks a Sun and a Moon and cross-fades them with the `dark:`
 * variant, so the correct glyph is on screen at the first paint rather than
 * after `useTheme()` resolves. Stacking puts the Moon in `position: absolute`,
 * and `Button` is `position: static` — so the Moon resolved against the nearest
 * positioned ancestor instead of its button. On Settings that ancestor is
 * `SidebarInset`, which sits *outside* the view's own `overflow-y-auto`
 * scroller, and an absolutely-positioned box does not scroll with a container
 * that is not its containing block. The Moon therefore stayed pinned where the
 * unscrolled layout had put it while its button scrolled away, drifting down
 * the page by exactly `scrollTop`. In dark mode the Sun is `scale-0`, so what
 * was left behind was an empty 32x32 hit target: a card reading "Switch between
 * light, dark, and system themes" with no visible control, and a stray moon
 * glyph roughly 1300px further down the document.
 *
 * The assertion is deliberately geometric rather than a class check. `relative`
 * on the trigger is today's fix, but the contract is "the glyph is inside the
 * button", and any future restyle that breaks it — a different stacking trick,
 * a `transform` moved onto an ancestor — should fail here too.
 *
 * Scrolling is the load-bearing step. At `scrollTop: 0` the drift is zero and
 * the bug is invisible, which is how it shipped twice: the Appearance card is
 * near the bottom of Settings, so nobody reaches it without scrolling, and
 * nobody testing it at the top of the page sees anything wrong.
 *
 * `ThemeToggle` also renders in the company picker and on Login, where it sits
 * in a header at the top of a document-scrolled page. Both are latent cases of
 * the same fault rather than separate bugs, and the fix is in the shared
 * component, so this spec exercises the one call site that actually reproduced.
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

const trigger = (page: Page) => page.getByRole("button", { name: "Change theme" });

/** Pin the theme before the app boots, so the first paint is the one under test. */
async function openSettings(page: Page, theme: "dark" | "light") {
  await page.addInitScript((value) => {
    window.localStorage.setItem("theme", value);
  }, theme);
  await page.goto("/#/settings");
  await expect(trigger(page)).toBeVisible();
  await expect(page.locator("html")).toHaveClass(new RegExp(`\\b${theme}\\b`));
}

/**
 * Geometry of the trigger and of whichever glyph is currently on screen.
 *
 * "On screen" is the whole point: the hidden icon is `scale-0` and collapses to
 * a 0x0 box, so a non-empty box is what distinguishes the painted glyph from
 * its counterpart without hard-coding which of the two a given theme shows.
 */
async function glyphGeometry(page: Page) {
  return page.evaluate(() => {
    const button = document.querySelector('button[aria-label="Change theme"]');
    if (!button) throw new Error("theme trigger not in the document");
    const box = button.getBoundingClientRect();
    const painted = [...button.querySelectorAll("svg")]
      .map((svg) => {
        const rect = svg.getBoundingClientRect();
        return {
          icon: svg.getAttribute("class")?.includes("lucide-sun") ? "sun" : "moon",
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height,
        };
      })
      .filter((glyph) => glyph.width > 0 && glyph.height > 0);
    return {
      button: { x: box.x, y: box.y, width: box.width, height: box.height },
      painted,
    };
  });
}

/** Scroll Settings until the Appearance card is on screen, and report by how much. */
async function scrollToAppearance(page: Page) {
  const scrolled = await page.evaluate(() => {
    const button = document.querySelector('button[aria-label="Change theme"]');
    button?.scrollIntoView({ block: "center" });
    let node = button?.parentElement ?? null;
    while (node) {
      if (node.scrollTop > 0) return node.scrollTop;
      node = node.parentElement;
    }
    return window.scrollY;
  });
  // A drift bug that only appears past the fold proves nothing if the card was
  // already in view, so fail loudly rather than passing on an untested page.
  expect(scrolled, "Appearance should sit below the fold on Settings").toBeGreaterThan(0);
  return scrolled;
}

for (const theme of ["dark", "light"] as const) {
  test(`theme switch shows a glyph inside its button on scrolled Settings (${theme})`, async ({
    page,
  }) => {
    await openSettings(page, theme);
    await scrollToAppearance(page);

    const { button, painted } = await glyphGeometry(page);

    // Exactly one of the stacked icons is painted; the other is `scale-0`.
    expect(painted).toHaveLength(1);
    expect(painted[0].icon).toBe(theme === "dark" ? "moon" : "sun");

    // And it is inside the button it belongs to — the assertion that failed on
    // `main`, where the moon landed ~1300px below its own trigger.
    const glyph = painted[0];
    expect(glyph.x).toBeGreaterThanOrEqual(button.x);
    expect(glyph.y).toBeGreaterThanOrEqual(button.y);
    expect(glyph.x + glyph.width).toBeLessThanOrEqual(button.x + button.width);
    expect(glyph.y + glyph.height).toBeLessThanOrEqual(button.y + button.height);
  });
}

test("the theme menu opens over the trigger and switches the theme", async ({ page }) => {
  await openSettings(page, "dark");
  await scrollToAppearance(page);

  // Clicking the control the card advertises, not blank space next to it.
  await trigger(page).click();
  const menu = page.locator('[data-slot="dropdown-menu-content"]');
  await expect(menu).toBeVisible();

  // The popup must be opaque: it overlaps the cards beneath it, and a
  // see-through menu is unreadable against them.
  await expect(menu).not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");

  await menu.getByRole("menuitem", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveClass(/\blight\b/);

  // The glyph swaps with the theme rather than going blank.
  const { button, painted } = await glyphGeometry(page);
  expect(painted).toHaveLength(1);
  expect(painted[0].icon).toBe("sun");
  expect(painted[0].y).toBeGreaterThanOrEqual(button.y);
  expect(painted[0].y + painted[0].height).toBeLessThanOrEqual(button.y + button.height);
});
