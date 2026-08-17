import { expect, test } from "@playwright/test";

test("probe", async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
  await page.goto("/#/ledgers/tasks");
  await page.waitForTimeout(2000);
  console.log("BUTTONS:", await page.getByRole("button").allInnerTexts());
  const add = page.getByRole("button", { name: "Add task" });
  console.log("ADD COUNT:", await add.count());
  await add.first().click();
  await page.waitForTimeout(1000);
  console.log("AFTER CLICK, headings:", await page.getByRole("heading").allInnerTexts());
  console.log("dialog content:", await page.locator('[data-slot="dialog-content"]').count());
  console.log("new-prompt:", await page.locator("#new-prompt").count());
});
