import { expect, test } from "@playwright/test";

/**
 * The Settings → Brain (operator memory) surface, end to end against the real FactStore.
 *
 * This flow exercises `…/memory` and needs no inference, so it runs on the
 * default Console E2E lane rather than the live-brain lane.
 */

// The first-run product tour opens a Radix dialog over the console; every
// element beneath it is `aria-hidden` while it shows. Same suppression the
// other specs use.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("operator adds a Brain memory that persists across reload and can be deleted", async ({
  page,
}) => {
  // Settings → Brain reads the real FactStore over `…/memory`; adding a note must
  // survive a reload (proving it hit the backend, not localStorage) and delete
  // must remove it. The legacy `#/memory` address must land here too.
  await page.goto("/#/memory");
  await expect(page.getByRole("heading", { name: "Brain", level: 1 })).toBeVisible();

  const title = `e2e memory ${Date.now()}`;
  await page.getByTestId("memory-add").click();
  await page.getByTestId("memory-title").fill(title);
  await page.getByTestId("memory-body").fill("recall me on the next turn");
  await page.getByTestId("memory-save").click();

  const card = page.getByTestId("memory-card").filter({ hasText: title });
  await expect(card).toBeVisible({ timeout: 30_000 });

  // Reload: a localStorage stub would survive too, so also assert the health
  // strip counts a real backend item.
  await page.reload();
  await page.goto("/#/settings/brain");
  await expect(page.getByTestId("memory-card").filter({ hasText: title })).toBeVisible({
    timeout: 30_000,
  });

  // Delete removes it.
  const persisted = page.getByTestId("memory-card").filter({ hasText: title });
  await persisted.getByRole("button", { name: "Delete memory" }).click();
  await expect(page.getByTestId("memory-card").filter({ hasText: title })).toHaveCount(0, {
    timeout: 30_000,
  });
});
