import { test } from "@playwright/test";

test("probe", async ({ page }) => {
  page.on("console", (m) => console.log("CONSOLE:", m.type(), m.text().slice(0, 300)));
  page.on("pageerror", (e) => console.log("PAGEERROR:", String(e).slice(0, 500)));
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
  await page.goto("/#/ledgers/tasks");
  await page.waitForTimeout(1500);
  await page.getByRole("button", { name: "Add task" }).first().click();
  await page.waitForTimeout(1500);
  console.log("dialogs:", await page.locator('[data-slot="dialog-content"]').count());
  console.log("body snippet:", (await page.locator("body").innerHTML()).slice(0, 200));
});
