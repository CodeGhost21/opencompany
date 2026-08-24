import { expect, test } from "@playwright/test";

test("diag skip link focus", async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
  await page.goto("/#/overview");
  await page.getByRole("link", { name: "Skip to content", exact: true }).waitFor();

  const before = await page.evaluate(() => ({
    hasFocus: document.hasFocus(),
    activeElement: document.activeElement
      ? `${document.activeElement.tagName}#${document.activeElement.id}`
      : "null",
    skipInDom: !!document.querySelector('a[href="#main-content"]'),
  }));
  console.log("DIAG BEFORE TAB:", JSON.stringify(before));

  await page.keyboard.press("Tab");

  const after = await page.evaluate(() => {
    const skip = document.querySelector('a[href="#main-content"]');
    return {
      hasFocus: document.hasFocus(),
      activeElement: document.activeElement
        ? `${document.activeElement.tagName}#${document.activeElement.id}.${String(document.activeElement.className).slice(0, 60)}`
        : "null",
      skipIsFocused: skip === document.activeElement,
    };
  });
  console.log("DIAG AFTER TAB:", JSON.stringify(after));

  for (let i = 0; i < 5; i++) {
    await page.keyboard.press("Tab");
    const ae = await page.evaluate(() =>
      document.activeElement
        ? `${document.activeElement.tagName}#${document.activeElement.id}.${String(document.activeElement.className).slice(0, 60)}`
        : "null",
    );
    console.log("DIAG AFTER TAB", i + 2, ":", ae);
  }
  await expect(page.locator("body")).toBeVisible();
});
