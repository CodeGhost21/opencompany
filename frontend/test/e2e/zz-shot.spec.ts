import { test } from "@playwright/test";

test("screenshot the runs section", async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
  await page.setViewportSize({ width: 1100, height: 1400 });
  await page.goto("/#/team/engineer");
  await page.waitForTimeout(6000);
  await page.screenshot({ path: "/tmp/claude-1000/agent-runs.png", fullPage: true });

  // …and one attempt opened.
  const row = page.locator('[data-testid^="agent-run-"]').first();
  if (await row.isVisible().catch(() => false)) {
    await row.click();
    await page.waitForTimeout(2500);
    await page.screenshot({ path: "/tmp/claude-1000/agent-run-detail.png", fullPage: true });
  }
});
