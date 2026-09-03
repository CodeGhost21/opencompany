import { expect, test } from "@playwright/test";

/**
 * The low-traffic surfaces below have no visible title bar, but each is still a
 * complete page. Keep its document outline usable even when it is reached by a
 * direct hash URL rather than the sidebar (issue #1392).
 */
const PAGES = [
  { hash: "memory", heading: "Brain" },
  { hash: "feedback", heading: "Feedback" },
  { hash: "inbox", heading: "Inbox" },
  { hash: "finances", heading: "Finances" },
  // Retired. Room replaced it, and the address names no channel to rewrite it
  // onto — so it must reach the explanation, not a blank page and not a silent
  // landing on whichever channel Room defaults to.
  { hash: "conversation", heading: "Page not found" },
] as const;

test.beforeEach(async ({ page }) => {
  // The first-run tour makes the console beneath it aria-hidden. Its state is
  // irrelevant to the document outline, so keep it out of this assertion.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

for (const { hash, heading } of PAGES) {
  test(`the ${heading} route has one level-one heading`, async ({ page }) => {
    await page.goto(`/#/${hash}`);

    const main = page.getByRole("main");
    await expect(main.getByRole("heading", { name: heading, level: 1 })).toBeVisible({
      timeout: 30_000,
    });
    await expect(main.locator("h1")).toHaveCount(1);
  });
}
