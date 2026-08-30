import { expect, test } from "@playwright/test";

test("sidebar utility bar", async ({ page }) => {
  await page.goto("/#/overview");
  const skip = page.getByRole("button", { name: /skip for now/i });
  await skip.first().click({ timeout: 20000 }).catch(() => {});
  await page.waitForTimeout(3000);
  await page.locator('[data-testid="sidebar-utilities"]').waitFor();
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/sidebar.png" });
  // collapsed rail
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await page.waitForTimeout(700);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/sidebar-rail.png" });
  await expect(page.getByRole("button", { name: "Expand sidebar" })).toBeVisible();
});
