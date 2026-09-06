import { expect, test } from "@playwright/test";

/**
 * Issue #582 — the Connections page must say one thing about a provider.
 *
 * The page carried two provider lists fed by two routes that applied different
 * rules to the same Composio state, so Gmail could render as connected in one
 * and offer an actionable Connect button in the other, on one screen. An
 * operator who trusted the second one would start a sign-in for an account
 * already connected — the shape #396 had to clean up once.
 *
 * ## What this spec can and cannot prove here
 *
 * The acceptance criteria are structural, and two of the three are checkable
 * against any host: a provider appears exactly once, and no connect action is
 * offered for one already connected. Those are asserted below over whatever the
 * host actually serves, so they hold for a harness with no Composio and for a
 * tenant with a full catalogue alike.
 *
 * The third — that the *status* comes from one place — is the half that needed a
 * live Composio to exercise, because the contradiction was between
 * `GET …/connections` (which discarded Composio state unless the company granted
 * the `composio` namespace) and `GET …/composio/connections` (which never
 * consulted the grant). The harness binary carries no `composio` feature, so
 * that divergence cannot be reproduced from here at all. It is pinned where it
 * is decidable instead: `test/unit/provider-grid.test.ts` for the merge, and
 * `src/server/ops/connections_read.rs` for the gate itself.
 *
 * Drives a running host (see `playwright.config.ts` — the harness brings it up,
 * there is no `webServer`). CI does not run Playwright.
 */

type Page = import("@playwright/test").Page;

/**
 * Open the page with the first-run tour out of the way.
 *
 * The dismissal is not cosmetic and the wait is not padding: the welcome dialog
 * is a Radix dialog, so while it is open every other element is `aria-hidden`
 * and therefore invisible to `getByRole`. It also mounts a beat *after* the
 * navigation resolves — so an `isVisible()` check taken immediately races it and
 * usually wins, leaving the dialog to open over the assertions that follow.
 * Every spec here starts from the same shared storage state, which was captured
 * before any tour was skipped, so this happens once per test rather than once
 * per run.
 */
async function openConnections(page: Page): Promise<void> {
  await page.goto("/#/connections/apps");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already dismissed in this context — nothing to close */
    });
  await expect(skip).toBeHidden({ timeout: 10_000 });
  await expect(page.getByRole("heading", { name: "Providers" })).toBeVisible({ timeout: 30_000 });
}

test("a provider appears exactly once on the page", async ({ page }) => {
  await openConnections(page);

  const slugs = await page.locator("[data-testid^='provider-']").evaluateAll((nodes) =>
    nodes.map((n) => n.getAttribute("data-testid") ?? ""),
  );
  expect(slugs.length, "the page should render provider tiles").toBeGreaterThan(0);

  const duplicated = slugs.filter((slug, i) => slugs.indexOf(slug) !== i);
  expect(duplicated, `these providers rendered more than one tile: ${duplicated.join(", ")}`).toEqual(
    [],
  );
});

test("the page holds one provider grid, not two", async ({ page }) => {
  await openConnections(page);

  // The second grid was a categorised one whose sections were headings named
  // after the category. Categories survive as filter chips on the one grid, so a
  // *heading* by that name means the old grid came back.
  await expect(page.getByRole("heading", { name: "Providers" })).toHaveCount(1);
  for (const category of ["Communication", "Productivity", "Developer", "Finance", "Storage"]) {
    await expect(
      page.getByRole("heading", { name: category, exact: true }),
      `"${category}" is rendered as a section heading — the categorised second grid is back`,
    ).toHaveCount(0);
  }
});

test("no connect action is offered for an already-connected provider", async ({ page }) => {
  await openConnections(page);

  // The tile is the affordance only when there is something to do with it, so a
  // connected tile must not be a button. This holds vacuously on a host with
  // nothing connected, which is the honest outcome — it is an invariant over
  // whatever is connected, not a claim that something is.
  const connectable = page.locator("[data-testid^='provider-'] button[aria-label^='Connect ']");
  for (const tile of await connectable.all()) {
    await expect(tile).not.toHaveAttribute("aria-label", /\. connected/i);
  }
});
