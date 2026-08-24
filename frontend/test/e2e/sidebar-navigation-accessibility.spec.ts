import { expect, test } from "@playwright/test";

// The first-run tour is modal and correctly receives focus while it is open;
// skip it here so this spec can exercise the shell's ordinary tab order.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("the skip link reaches main content and the sidebar is the primary navigation", async ({
  page,
}) => {
  await page.goto("/#/overview");

  const skip = page.getByRole("link", { name: "Skip to content", exact: true });
  const main = page.getByRole("main");

  // The console boots through a "Connecting…" phase that has no shell and so
  // no skip link; a Tab pressed against that phase moves focus nowhere. The
  // skip link exists only once the shell (and its sidebar) has mounted, so
  // waiting for it is the app-ready signal — and the sidebar's chrome renders
  // in the same commit, so nothing focusable appears between them.
  await skip.waitFor();

  // This is the first tab stop, ahead of the sidebar's host switcher and its
  // destination rows, even though the fixed sidebar renders before main.
  await page.keyboard.press("Tab");
  await expect(skip).toBeFocused();
  await expect(skip).toBeVisible();

  // Hash routing owns `window.location.hash`; the skip link must focus main
  // without turning its conventional fragment into a route change.
  await page.keyboard.press("Enter");
  await expect(main).toBeFocused();
  await expect(main).toHaveAttribute("id", "main-content");
  await expect(page).toHaveURL(/#\/overview$/);

  const navigation = page.getByRole("navigation", { name: "Main navigation", exact: true });
  await expect(navigation).toBeVisible();
  await expect(navigation.getByRole("button", { name: "Overview", exact: true })).toBeVisible();
  await expect(navigation.getByRole("button", { name: "Feedback", exact: true })).toBeVisible();
  await expect(navigation.getByRole("link", { name: "Join our Discord", exact: true })).toBeVisible();
});
