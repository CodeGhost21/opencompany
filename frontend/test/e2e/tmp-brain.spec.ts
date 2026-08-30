import { expect, test } from "@playwright/test";

test("brain has its own nav row and both old addresses land on it", async ({ page }) => {
  await page.goto("/#/settings/brain");
  const skip = page.getByRole("button", { name: /skip for now/i });
  await skip.first().click({ timeout: 20000 }).catch(() => {});
  await expect(page).toHaveURL(/#\/brain$/);
  await page.goto("/#/memory");
  await expect(page).toHaveURL(/#\/brain$/);
  await page.waitForTimeout(2500);
  const nav = page.getByRole("navigation", { name: "Main navigation" });
  await expect(nav.getByRole("button", { name: "Brain", exact: true })).toBeVisible();
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/brain.png" });
});
