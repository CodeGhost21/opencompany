import { expect, test } from "@playwright/test";

import { expectHostMenuGone } from "./host-switcher";

/**
 * The "Manage hosts" page, and why this file no longer drives it.
 *
 * It used to cover the wiring that page is: the menu item that opens it, the
 * roster it draws from context, renaming a host, re-addressing one that moved,
 * refusing an address with no scheme, refusing a move onto a host this console
 * already holds, and forgetting a host — including the property underneath all
 * of it, that a connection id is the namespace every browser-local key hangs
 * off (`scopedKey`), so "this host moved" must be expressible without minting a
 * new one.
 *
 * None of that is reachable now. While the product is scoped to one company per
 * install (`src/product-scope.ts`, `HOSTS_HIDDEN`) the switcher opens nothing,
 * and its menu was the only entry point to that page — the page component is
 * still mounted, so turning the flag off restores both it and the cases above.
 *
 * Retired rather than deleted or skipped: what is left is the one assertion
 * that still has a subject, which is the absence of the way in. A skipped spec
 * would report green while covering nothing.
 */

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("the console offers no way into host management", async ({ page }) => {
  await page.goto("/#/company");

  await expectHostMenuGone(page);
  await expect(page.getByTestId("manage-hosts-page")).toHaveCount(0);
});
