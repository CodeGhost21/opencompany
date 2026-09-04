import { expect, test } from "@playwright/test";

import { expectHostMenuGone } from "./host-switcher";

/**
 * Switching hosts is a navigation, and why this file no longer drives one.
 *
 * It covered issue #1358: with two hosts and the second one down, picking the
 * dead host from the switcher pushed a history entry, so Back undid the switch
 * instead of silently spending the working host's route stack; a copied address
 * reopened the host it named, failure and all; and a host that could not be
 * reached could be forgotten from the failure screen itself.
 *
 * All of it starts by picking a host from the switcher, and the switcher opens
 * nothing while the product is scoped to one company per install
 * (`src/product-scope.ts`, `HOSTS_HIDDEN`). The `?host=` routing and its
 * history behaviour are untouched in the source — turning the flag off restores
 * the entry point and these cases with it.
 *
 * Retired rather than deleted or skipped, so the coverage that was lost is
 * written down where the next reader will look for it.
 */

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("there is no host to switch to, so no switch to undo", async ({ page }) => {
  await page.goto("/#/ledgers/tasks");

  await expectHostMenuGone(page);
  // The address keeps naming the page, not a host — nothing scoped it to one.
  await expect(page).toHaveURL(/#\/ledgers\/tasks/);
});
