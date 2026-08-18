import { expect, test } from "@playwright/test";

/**
 * Issue #933 — the "Starting the product tour." toast that never dismissed.
 *
 * Only a browser can hold this. sonner's auto-dismiss is a timer it *pauses* on
 * three pieces of its own state, and the latch that produced the report is
 * reachable only through real input: sonner ships an Alt+T hotkey that expands
 * the toaster and focuses the toast list, and `expanded` is cleared only by a
 * `mouseleave` — which a pointer that was never over the toaster cannot deliver.
 * With one toast up nothing changes the toast array either, so the timer stayed
 * paused. The reporter saw it survive nine minutes and four navigations with a
 * working × and nothing else that cleared it.
 *
 * The decision the console's guard makes is pinned fast in
 * `test/unit/toast-lifetime.test.ts`. What is only true here is that the latch
 * is real, that the guard beats it, and that it does not cost us the one pause
 * worth keeping — hovering to read.
 */

type Page = import("@playwright/test").Page;

/** The bottom-right toast, whichever view it is riding over. */
const TOAST = "[data-sonner-toast]";

/**
 * Land on Settings with no tour running, and no welcome dialog in the way.
 *
 * The dialog is not cosmetic here: it is a Radix dialog, so while it is open
 * every element under it is `aria-hidden` and invisible to `getByRole` — the
 * "Replay tour" button included. A first run always offers it.
 */
async function settings(page: Page): Promise<void> {
  await page.goto("/#/settings");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await expect(skip).toBeVisible();
  await skip.click();
  await expect(page.getByRole("button", { name: "Replay tour" })).toBeVisible();
}

/** Raise the toast the issue is about, and park the pointer far from it. */
async function replayTour(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Replay tour" }).click();
  await expect(page.locator(TOAST)).toBeVisible();
  await page.mouse.move(20, 20);
}

test("the tour toast dismisses itself", async ({ page }) => {
  await settings(page);
  await replayTour(page);
  // Generous: sonner's own 4s timer should do this, and the console's ceiling is
  // a second line at 6s. Either way it must not need a click.
  await expect(page.locator(TOAST)).toHaveCount(0, { timeout: 15_000 });
});

test("a toast cannot be pinned open by sonner's expand hotkey", async ({ page }) => {
  await settings(page);
  await replayTour(page);

  // The latch. Before the fix this toast stayed up indefinitely — verified at
  // twenty seconds, and the report had it at nine minutes.
  await page.keyboard.press("Alt+KeyT");
  await page.mouse.move(20, 20);

  await expect(page.locator(TOAST)).toHaveCount(0, { timeout: 20_000 });
});

test("a toast does not follow the operator across views", async ({ page }) => {
  await settings(page);
  await replayTour(page);
  await page.keyboard.press("Alt+KeyT");

  // The shape of the report: raise it, then walk the console. The toaster is
  // mounted at the app root, outside the routed tree, so nothing about changing
  // view clears a toast — the ceiling is the only reason this ends.
  await page.mouse.move(20, 20);
  await page.goto("/#/overview");
  await page.goto("/#/approvals");

  await expect(page.locator(TOAST)).toHaveCount(0, { timeout: 20_000 });
});

test("hovering a toast still holds it open to be read", async ({ page }) => {
  await settings(page);
  await page.getByRole("button", { name: "Replay tour" }).click();
  const toast = page.locator(TOAST);
  await expect(toast).toBeVisible();

  // The pause the fix must not take away: a toast under the pointer is one
  // somebody is reading.
  await toast.hover();
  await page.waitForTimeout(9_000);
  await expect(toast).toBeVisible();

  // ...and it goes as soon as they look away, without a click.
  await page.mouse.move(20, 20);
  await expect(toast).toHaveCount(0, { timeout: 10_000 });
});
