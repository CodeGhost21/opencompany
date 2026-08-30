import { expect, test } from "@playwright/test";

test("the graph draws for a company with no desks", async ({ page }) => {
  await page.route("**/desks", async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    await route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
  });
  await page.goto("/#/overview");
  const skip = page.getByRole("button", { name: /skip for now/i });
  await skip.first().click({ timeout: 20000 }).catch(() => {});
  await expect(skip).toHaveCount(0, { timeout: 10000 }).catch(() => {});
  await page.waitForTimeout(7000);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/graph-nodesk.png" });
  // The canvas is rendered, not suppressed.
  await expect(page.locator("svg").first()).toBeVisible();
});
