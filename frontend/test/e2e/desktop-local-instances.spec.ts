import { expect, test } from "@playwright/test";

import { expectHostMenuGone } from "./host-switcher";

/**
 * Hosts on this computer, and why this file no longer drives them.
 *
 * It covered the desktop's local-instance roster end to end, through the UI
 * that fronts the shell commands: a stopped instance listed and startable
 * rather than shown as a broken row, a host started here stopped again without
 * losing the others, a second company created on this computer, a
 * desktop-created company deleted after an explicit confirmation, and the
 * default instance refusing deletion because its root is the application data
 * directory itself.
 *
 * All of it is reached through the "Add a host" screen's local tab, and that
 * screen has no entry point on the desktop while the product is scoped to one
 * company per install (`src/product-scope.ts`, `HOSTS_HIDDEN`): the switcher
 * opens nothing, and the zero-host recovery is now a single "start the host on
 * this computer" action rather than a chooser.
 *
 * **This is the largest coverage loss in this change.** The Tauri commands
 * underneath (`oc_local_instances`, `oc_create_local_instance`,
 * `oc_start_local_instance`, `oc_stop_local_instance`,
 * `oc_delete_local_instance`) are untouched and still unit-tested in Rust; what
 * is no longer exercised is the console wiring in front of them. Turning the
 * flag off restores the screen and every case above.
 *
 * Retired rather than deleted or skipped, so the gap is recorded where the next
 * reader will look for it.
 */

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("the console offers no way onto the add-a-host screen", async ({ page }) => {
  await page.goto("/#/company");

  await expectHostMenuGone(page);
  await expect(page.getByTestId("add-host-page")).toHaveCount(0);
});
