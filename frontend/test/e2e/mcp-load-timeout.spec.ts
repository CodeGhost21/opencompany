import { expect, test } from "@playwright/test";

/**
 * Regression proof for #2014 — the MCP servers page hung on its skeleton forever.
 *
 * `GET .../mcp/servers` reaches the host through `OpenCompanyClient`'s one JSON
 * read path, which had no timeout. A host that accepted the connection and then
 * never answered produced no `fetch` event, so `McpServersSection`'s own `catch`
 * — the one that flips the page to its `mcp-load-error` state — never ran, and
 * the page sat on `<Skeleton>` with no error and no escape.
 *
 * The section is unchanged by the fix. The shared request deadline
 * (`DEFAULT_REQUEST_TIMEOUT_MS`, 30s, on every GET) turns the stall into an
 * ordinary rejection its existing error branch already renders. This drives
 * that whole path in a real browser: intercept the list read and never answer
 * it, then prove the page reaches its error state at the deadline instead of
 * hanging.
 *
 * ## What is faked here, and what is not
 *
 * Only the one read is faked — held open and never fulfilled, which is the stall
 * itself. Everything else is real: the host, the console bundle, the session,
 * and the client's real 30s deadline (not shortened for the test — the point is
 * that the deadline fires). `test.setTimeout(45_000)` gives that 30s room.
 *
 * Like the rest of `test/e2e`, this drives a running host and is not wired into
 * CI (the Playwright config declares no `webServer`); `npm run typecheck:e2e`
 * compiles it, nothing runs it automatically. It runs on the default Console
 * E2E lane — the list read is stalled before any capability matters, so no
 * `LIVE_BRAIN` feature is needed.
 */

/**
 * The company's MCP server list, in both addressing shapes the client serves —
 * single-company (`/api/v1/company/mcp/servers`) and platform
 * (`/api/v1/companies/{id}/mcp/servers`). Anchored on `$` so a server's tool
 * read (`.../mcp/servers/{name}/tools`) is left alone.
 */
const isMcpServerList = (url: URL) => /\/mcp\/servers$/.test(url.pathname);

/**
 * The first-run product tour opens a Radix dialog over a fresh console and would
 * cover the page under test. Same suppression the other specs use — answer
 * "already skipped" for whatever company id the host resolves to.
 */
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

test("the MCP page surfaces a load error at the deadline instead of hanging", async ({
  page,
}) => {
  // The client's GET deadline is 30s; give the whole flow room past it.
  test.setTimeout(45_000);

  // Hold the list read open and never answer it — the stall #2014 is about. A
  // route handler that never settles leaves the request pending, exactly as a
  // host that accepted the connection and then went silent would.
  await page.route(isMcpServerList, () => new Promise<never>(() => {}));

  await page.goto("/#/connections/mcp");

  // The page starts on its loading skeleton with no error, as it always did.
  await expect(page.locator(".h-24.rounded-xl")).toBeVisible();
  await expect(page.getByTestId("mcp-load-error")).toBeHidden();

  // Once the 30s deadline fires, the read rejects and the section's existing
  // error branch renders — the skeleton is gone and the error is shown.
  const error = page.getByTestId("mcp-load-error");
  await expect(error).toBeVisible({ timeout: 40_000 });
  await expect(error).toContainText("Couldn't load");
  await expect(page.locator(".h-24.rounded-xl")).toHaveCount(0);
});
