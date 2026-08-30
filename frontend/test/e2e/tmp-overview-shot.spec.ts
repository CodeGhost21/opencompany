import { expect, test } from "@playwright/test";

test("overview paints the graph", async ({ page }) => {
  await page.goto("/#/overview");
  const skip = page.getByRole("button", { name: /skip for now/i });
  if (await skip.count()) await skip.first().click();
  await expect(page.locator(".oc-kg")).toBeVisible({ timeout: 30000 });
  await page.waitForTimeout(6000);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/overview.png", fullPage: false });
});
