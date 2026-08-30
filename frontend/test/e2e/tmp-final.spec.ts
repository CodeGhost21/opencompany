import { expect, test } from "@playwright/test";

test("sidebar after the utility bar work", async ({ page }) => {
  await page.goto("/#/overview");
  const skip = page.getByRole("button", { name: /skip for now/i });
  await skip.first().click({ timeout: 20000 }).catch(() => {});
  await page.waitForTimeout(4000);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/final.png" });
  // The browser must not get the desktop's window chrome.
  await expect(page.locator("[data-tauri-drag-region]")).toHaveCount(0);
  await page.getByTestId("host-switcher").click();
  await page.waitForTimeout(500);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/final-menu.png" });
});
