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
  await page.goto("/#/company");

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
  await expect(page).toHaveURL(/#\/company$/);

  const navigation = page.getByRole("navigation", { name: "Main navigation", exact: true });
  await expect(navigation).toBeVisible();
  // Four sections, and the four are the whole list. Asserted by count as well
  // as by name: a tenth row creeping back in is the thing this restructure
  // exists to stop, and four `toBeVisible` calls would not notice it.
  for (const name of ["Room", "Company", "Connections", "Flows"]) {
    await expect(navigation.getByRole("button", { name, exact: true })).toBeVisible();
  }
  await expect(
    page.locator("[data-slot=sidebar-content] [data-sidebar=menu-button]"),
  ).toHaveCount(4);
  // Overview and Approvals are not among them: they are chrome in the window's
  // title row now, not destinations in a list of destinations.
  for (const name of ["Overview", "Approvals", "Observatory"]) {
    await expect(navigation.getByRole("button", { name, exact: true })).toHaveCount(0);
  }

  // Settings, Feedback and Discord are utilities: things you do to the console
  // rather than the places an operator works out of. They keep a named group of
  // their own, and it is now in the sidebar's FOOTER — the header the bar used
  // to sit in is gone, along with the host switcher that shared it, both moved
  // into the window's title row.
  //
  // THREE, not four. The collapse control left this group entirely and now sits
  // on the content card's leading seam, outside the sidebar; it is pinned there
  // by `sidebar-toggle-reachable.spec.ts`, and asserted below only to the extent
  // that it is no longer here.
  const utilities = page.getByRole("group", { name: "Console utilities", exact: true });
  await expect(utilities).toBeVisible();
  for (const name of ["Settings", "Feedback", "Join our Discord"]) {
    await expect(
      utilities.getByRole(name === "Join our Discord" ? "link" : "button", { name, exact: true }),
    ).toBeVisible();
  }
  await expect(
    utilities.getByRole("button", { name: /sidebar$/ }),
    "the collapse control is not one of the console's utilities any more",
  ).toHaveCount(0);

  // And they are not interleaved with the destinations. The three now carry
  // visible labels and `aria-current`, so they belong INSIDE the navigation
  // landmark in a way the old icon-only bar did not — what still has to hold is
  // that they are a separate, separately named group under the list of places
  // you go, rather than three more rows in it. `sidebar-content` is that list;
  // `sidebar-footer` is the group.
  const destinations = page.locator("[data-slot=sidebar-content]");
  await expect(destinations.getByRole("button", { name: "Room", exact: true })).toBeVisible();
  await expect(destinations.getByRole("button", { name: "Settings", exact: true })).toHaveCount(0);
  await expect(destinations.getByRole("button", { name: "Feedback", exact: true })).toHaveCount(0);
  await expect(
    page.locator("[data-slot=sidebar-footer]").getByTestId("sidebar-utilities"),
  ).toHaveCount(1);
});
