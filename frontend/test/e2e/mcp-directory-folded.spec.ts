import { expect, test } from "@playwright/test";

/**
 * Discover is not a second screen. One field searches this company's own
 * servers and the public directory, and both halves land in the one list under
 * headings that say which is which.
 *
 * The failure this prevents: the old directory browser could not see what the
 * company already had, so it offered Install for a server sitting one tab away.
 *
 * Default-feature host. The directory route needs the `mcp` feature, so the
 * unwired path is what this lane can assert on honestly; the reachable half is
 * that no call is made at all until something is typed.
 */

type Page = import("@playwright/test").Page;

async function openMcp(page: Page) {
  await page.goto("/#/connections/mcp");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already seen in this context */
    });
  await expect(page.getByTestId("mcp-search")).toBeVisible({ timeout: 30_000 });
}

test("Discover is gone as a tab — the one list is the only place to look", async ({
  page,
}) => {
  await openMcp(page);

  await expect(page.getByRole("tab", { name: "Your servers" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Discover" })).toHaveCount(0);
});

test("the directory is not called until something is typed", async ({ page }) => {
  const directoryCalls: string[] = [];
  await page.route("**/mcp/registry/**", (route) => {
    directoryCalls.push(route.request().url());
    return route.continue();
  });

  await openMcp(page);
  await expect(page.getByTestId("mcp-server-row").first()).toBeVisible();

  // Opening the page costs nothing. Searching this company's own servers is
  // local; only a typed term reaches outward.
  expect(directoryCalls).toEqual([]);

  // And the counter above is wired: a glob that matched nothing would have made
  // the assertion pass for the wrong reason, on a page calling the directory on
  // every visit.
  await page.getByTestId("mcp-search").fill("github");
  await expect
    .poll(() => directoryCalls.length, { timeout: 15_000 })
    .toBeGreaterThan(0);
});

test("a search reports both halves, and names the company's own", async ({
  page,
}) => {
  await openMcp(page);
  await page.getByTestId("mcp-search").fill("deep");

  const companyHeading = page
    .getByTestId("mcp-group-row")
    .filter({ hasText: "In this company" });
  await expect(companyHeading).toBeVisible();

  await expect(
    page.getByTestId("mcp-server-row").filter({ hasText: "deepwiki" }),
  ).toBeVisible();
});

test("a build without the feature reads as a missing feature, not an error", async ({
  page,
}) => {
  await openMcp(page);
  await page.getByTestId("mcp-search").fill("github");

  // The directory route 404s without the `mcp` feature. That is a fact about
  // the build, and must not render as a failed search or a broken page — the
  // company's own half of the answer stays on screen either way.
  await expect(page.getByTestId("mcp-registry-unwired")).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("mcp-load-error")).toHaveCount(0);
});
