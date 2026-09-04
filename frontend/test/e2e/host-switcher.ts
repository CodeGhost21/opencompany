import { expect, type Page } from "@playwright/test";

/**
 * Driving the host switcher, which replaced the icon rail (issue #1142) and is
 * now a nameplate.
 *
 * The rail was permanently on screen, so a spec could read any host's status
 * straight off it. A dropdown could not be read until it was open, so the
 * trigger carried the two things worth knowing without opening it
 * (`data-host-count` and `data-worst-status`) and this module held the gesture
 * that got at the rest.
 *
 * That gesture is gone. While the product is scoped to one company per install
 * (`src/product-scope.ts`) the trigger opens nothing at all: no roster, no
 * "Add a host", no "Manage hosts", no company rows and no "New company". The
 * two trigger attributes survive, and are still what a spec reads.
 *
 * `openHostMenu` is deliberately NOT kept as a no-op. Every caller of it was
 * driving a flow that no longer has an entry point, and a helper that silently
 * succeeded would have let those specs go green while testing nothing.
 */

/**
 * Asserts the trigger is a name rather than a control.
 *
 * Clicks it first, because the trap worth guarding is not "the roster is
 * hidden" but "the trigger still opens" — onto a menu with every group hidden,
 * which is a chevron over an empty popup rather than a company's name.
 */
export async function expectHostMenuGone(page: Page): Promise<void> {
  const trigger = page.getByTestId("host-switcher");
  await expect(trigger).toBeVisible({ timeout: 30_000 });
  await trigger.click();

  // Nothing opened, and nothing that would have.
  await expect(page.locator('[role="menu"]')).toHaveCount(0);
  await expect(page.getByTestId("host-switcher-add")).toHaveCount(0);
  await expect(page.getByTestId("host-switcher-manage")).toHaveCount(0);
  await expect(page.getByTestId("switcher-new-company")).toHaveCount(0);
  await expect(page.locator('[data-testid^="host-row-"]')).toHaveCount(0);

  // An absent menu is not the same fact as a trigger that stopped behaving like
  // one. A control still announcing itself as a button with a popup — reachable
  // by Tab, actionable by Enter, described to a screen reader as something that
  // opens — is a promise the console no longer keeps, whatever the popup does.
  await expect(trigger.locator("button")).toHaveCount(0);
  await expect(trigger).not.toHaveAttribute("aria-haspopup", /.*/);
  await expect(trigger).not.toHaveAttribute("aria-expanded", /.*/);
}
