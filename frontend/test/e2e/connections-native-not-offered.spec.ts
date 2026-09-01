import { expect, test } from "@playwright/test";

/**
 * Issue #822 — the Connections page stops offering the native OAuth catalog.
 *
 * The route it offered was not broken, which is what made it worth removing.
 * `POST …/connections/{provider}/start` completes a real handshake against a
 * provider application the operator registered themselves, and the callback
 * stores `oauth/{provider}` — which is read by no agent tool anywhere under
 * `src/harness/` (#396). So a self-hoster could do everything the page asked,
 * watch the tile go green, and have given their agents nothing. #599 removed the
 * Connect buttons that *failed*; this is the one that succeeded and bought
 * nothing.
 *
 * ## Which tiles this actually removes, on this host
 *
 * Worth being precise, because the harness is not the empty-catalog case it
 * looks like. `GET …/composio` answers `200` even in a build carrying no
 * `composio` feature: `inBuild: false`, `catalogSource: "fallback"`, and a
 * built-in starter list of eight slugs. So the grid here is eight backend tiles
 * plus — before this change — the five `CONNECTION_PROVIDERS` entries that list
 * does not carry (Dropbox, Stripe, HubSpot, X, LinkedIn), appended from console
 * metadata alone. Those five are what goes, and they are exactly the issue's
 * "a provider the backend catalog does not carry no longer appears solely
 * because the console has a logo for it".
 *
 * The eight stay, because the host offers them — and they say "not available
 * here", because this build cannot authorize any of them.
 *
 * ## What is stubbed, and why only that
 *
 * Two host answers this harness cannot produce are stubbed at the wire, each
 * for a reason that is about the host rather than about convenience:
 *
 *  - **A stored native credential.** Making this host hold one needs a
 *    registered provider application and a live provider to hand back a code.
 *    The console's rendering of the resulting row is what is under test.
 *  - **A host with no catalog at all.** The fallback list above means the
 *    backend always answers with something; a host predating that route (or one
 *    whose probe times out) leaves `effectiveCatalog` empty, and that is the
 *    case this change turns from "eleven tiles" into "none".
 *
 * Everything else — session, status, route, rendering — is the live host.
 * `test/unit/provider-grid.test.ts` pins the same rules on the merge itself.
 *
 * Drives a running host (see `playwright.config.ts` — the harness brings it up,
 * there is no `webServer`). CI does not run Playwright.
 */

type Page = import("@playwright/test").Page;

/** The tiles this console used to append from its own metadata alone. */
const CONSOLE_ONLY = ["dropbox", "stripe", "hubspot", "twitter", "linkedin"];

/**
 * Open the page with the first-run tour out of the way.
 *
 * The welcome dialog is a Radix dialog, so while it is open every other element
 * is `aria-hidden` and invisible to `getByRole`; it also mounts a beat after the
 * navigation resolves, so an immediate `isVisible()` check races it and wins.
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

test("a provider the backend does not offer is no longer listed", async ({ page }) => {
  await openConnections(page);

  for (const slug of CONSOLE_ONLY) {
    await expect(
      page.getByTestId(`provider-${slug}`),
      `${slug} is still listed, and only the console's own metadata says it exists`,
    ).toHaveCount(0);
  }

  // The host's own eight are untouched — this removes an invented list, not the
  // page. Slack is the one that matters most: it is in the backend's list AND
  // is one of the four ids `well_known` can run a native handshake for, so if
  // anything were going to keep a native offer alive it would be this tile.
  await expect(page.getByTestId("provider-slack")).toBeVisible();
  await expect(page.getByTestId("provider-slack")).toContainText(/not available here/i);
});

test("no Connect is offered anywhere on a host that cannot authorize one", async ({ page }) => {
  await openConnections(page);

  // The tile IS the button when a tile is connectable, so this is the
  // affordance in its rendered form. This host has no Composio credential, and
  // the native hatch is no longer an offer — so there is nothing to click, which
  // before #822 was true of every tile EXCEPT one the operator had configured
  // `OPENCOMPANY_OAUTH_*` for.
  await expect(
    page.locator("[data-testid^='provider-'] button[aria-label^='Connect ']"),
  ).toHaveCount(0);
});

test("a provider already connected natively keeps its tile and its Disconnect", async ({
  page,
}) => {
  // The host's own answer, with one connected native row added — see the header
  // for why this row cannot come from the harness itself.
  await page.route("**/connections", async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    const response = await route.fetch();
    const rows = response.ok() ? await response.json() : [];
    await route.fulfill({
      json: [
        ...(Array.isArray(rows) ? rows : []),
        {
          provider: "slack",
          connected: true,
          via: ["native"],
          credentialSource: "static",
          account: "acme-workspace",
        },
      ],
    });
  });

  await openConnections(page);

  const tile = page.getByTestId("provider-slack");
  await expect(tile, "removing the offer must not hide a credential the company holds").toBeVisible();
  await expect(tile).toContainText("acme-workspace");

  // Releasable, and by the only route that releases anything here:
  // `DELETE …/connections/{provider}` blanks this host's own secret, which is
  // exactly what a `via: ["native"]` row is.
  await expect(tile.getByRole("button", { name: "Disconnect Slack" })).toBeVisible();

  // But still not an invitation: a connected tile is not a Connect affordance,
  // and nothing else on the page became one either.
  await expect(
    page.locator("[data-testid^='provider-'] button[aria-label^='Connect ']"),
  ).toHaveCount(0);
});

test("a host with no catalog says why the grid is empty", async ({ page }) => {
  // The case the native fallback used to paper over. Before #822 this rendered
  // eleven Connect-able tiles from console metadata; the honest answer is none,
  // and "No provider in All." would read as a broken filter rather than as a
  // host that has nothing to offer.
  await page.route("**/composio", async (route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await route.fulfill({ status: 404, json: { error: "not_found" } });
  });

  await openConnections(page);

  await expect(page.locator("[data-testid^='provider-']")).toHaveCount(0);
  const empty = page.getByTestId("providers-empty");
  await expect(empty).toBeVisible();
  await expect(empty).toContainText(/composio/i);
});
