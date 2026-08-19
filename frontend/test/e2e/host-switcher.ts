import { expect, type Page } from "@playwright/test";

/**
 * Driving the host switcher, which replaced the icon rail (issue #1142).
 *
 * The rail was permanently on screen, so a spec could read any host's status
 * straight off it. A dropdown cannot be read until it is open — so the trigger
 * carries the two things worth knowing without opening it, and this module
 * holds the one gesture that gets at the rest.
 */

/** The switcher's trigger: on screen in every phase, sidebar or not. */
export function hostSwitcher(page: Page) {
  return page.getByTestId("host-switcher");
}

/**
 * Opens the menu and waits for it to be there.
 *
 * Waits on "Add a host" rather than on a host row, because it is the one item
 * present at every count — including the zero a desktop with no host has, which
 * is exactly when the menu matters most.
 */
export async function openHostMenu(page: Page): Promise<void> {
  const trigger = hostSwitcher(page);
  await expect(trigger).toBeVisible({ timeout: 30_000 });
  await trigger.click();
  await expect(page.getByTestId("host-switcher-add")).toBeVisible();
}

/** A host's row inside the open menu. */
export function hostRow(page: Page, id: string) {
  return page.getByTestId(`host-row-${id}`);
}
