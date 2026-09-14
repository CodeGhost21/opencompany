import { expect, test } from "@playwright/test";

/**
 * Keys rework (issue #2306) — the search page's own confirm dialogs
 * (`docs/key-reworks/in-use-guards.md` §1: "**A** for the search page it
 * owns"), and the in-use guard on removing a provider that is the search
 * default (§4, X14: a confirmed removal never clears `search/default`).
 *
 * Two scenarios, per the operator's required E2E list:
 *
 *  - **Search provider add** — connecting SearXNG through the Add dialog.
 *  - **The remove-in-use warning** — removing the search default is refused
 *    on a first, unconfirmed attempt and reopens the dialog with the host's
 *    own reason, then succeeds once confirmed.
 *
 * SearXNG throughout, not a keyed provider: it needs no credential at all
 * (an instance address only), which keeps every fixture below free of
 * anything that could be mistaken for a real key.
 *
 * # Fully mocked, no live search backend
 *
 * Every request this spec cares about is stubbed at the wire with
 * `page.route`, following `composio-mode-switch.spec.ts`'s pattern: a running
 * host (`playwright.config.ts` brings it up), with the handful of routes
 * under test answered by this file instead of a real backend.
 */

type Page = import("@playwright/test").Page;
type Route = import("@playwright/test").Route;

const isSearchStatus = (url: URL) => /\/search$/.test(url.pathname);
const isSearchProviders = (url: URL) => /\/search\/providers$/.test(url.pathname);
const isSearchProviderSlug = (url: URL, slug: string) =>
  new RegExp(`/search/providers/${slug}$`).test(url.pathname);

const SEARXNG_ENDPOINT = "https://search.example.internal";
const IN_USE_ERROR = "SearXNG is the search default.";
const IN_USE_BODY = {
  error: IN_USE_ERROR,
  code: "in_use",
  usedBy: { default: true },
};

/** The wire shape this file's fixtures need — just enough of `SearchStatus`. */
interface SearchStatusMock {
  provider: string;
  effectiveProvider: string;
  providers: {
    slug: string;
    label: string;
    category: string;
    enabled: boolean;
    keyConfigured: boolean;
    takesKey: boolean;
    takesEndpoint: boolean;
    endpoint: string | null;
    complete: boolean;
    isDefault: boolean;
    usedBy?: { default?: true };
  }[];
  apiKeyConfigured: boolean;
  endpoint: string | null;
  needsApiKey: boolean;
  needsEndpoint: boolean;
  granted: boolean;
  inBuild: boolean;
  managedConfigured: boolean;
  managedDailyCallCap: number;
  supportedProviders: string[];
}

/** A status with nothing connected. */
function emptyStatus(): SearchStatusMock {
  return {
    provider: "managed",
    effectiveProvider: "managed",
    providers: [],
    apiKeyConfigured: false,
    endpoint: null,
    needsApiKey: false,
    needsEndpoint: false,
    granted: true,
    inBuild: true,
    managedConfigured: true,
    managedDailyCallCap: 100,
    supportedProviders: ["managed", "brave", "exa", "querit", "searxng"],
  };
}

/** A status with one connected, self-hosted SearXNG row, marked default. */
function searxngStatus(): SearchStatusMock {
  return {
    provider: "searxng",
    effectiveProvider: "searxng",
    providers: [
      {
        slug: "searxng",
        label: "SearXNG",
        category: "self-hosted",
        enabled: true,
        keyConfigured: false,
        takesKey: false,
        takesEndpoint: true,
        endpoint: SEARXNG_ENDPOINT,
        complete: true,
        isDefault: true,
        usedBy: { default: true },
      },
    ],
    apiKeyConfigured: false,
    endpoint: SEARXNG_ENDPOINT,
    needsApiKey: false,
    needsEndpoint: false,
    granted: true,
    inBuild: true,
    managedConfigured: true,
    managedDailyCallCap: 100,
    supportedProviders: ["managed", "brave", "exa", "querit", "searxng"],
  };
}

/** Open the search page with the first-run tour out of the way. */
async function openSearch(page: Page): Promise<void> {
  await page.goto("/#/connections/search");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already dismissed in this context */
    });
  await expect(skip).toBeHidden({ timeout: 10_000 });
  await expect(page.getByTestId("search-view")).toBeVisible({ timeout: 30_000 });
}

test.describe("Search provider add", () => {
  test("connecting SearXNG through the Add dialog lists it as the active provider", async ({
    page,
  }) => {
    let status = emptyStatus();
    await page.route(
      (url) => isSearchStatus(url),
      async (route: Route) => {
        if (route.request().method() !== "GET") return route.fallback();
        await route.fulfill({ json: status });
      },
    );
    await page.route(
      (url) => isSearchProviders(url),
      async (route: Route) => {
        if (route.request().method() !== "POST") return route.fallback();
        const body = route.request().postDataJSON() as {
          slug?: string;
          endpoint?: string;
        };
        expect(body.slug).toBe("searxng");
        expect(body.endpoint).toBe(SEARXNG_ENDPOINT);
        status = searxngStatus();
        await route.fulfill({
          json: {
            ok: true,
            probeClass: null,
            message: null,
            saved: true,
            status,
          },
        });
      },
    );

    await openSearch(page);
    await expect(page.getByTestId("search-provider-empty")).toBeVisible();

    await page.getByTestId("search-add").click();
    await expect(page.getByTestId("search-add-provider")).toBeVisible();

    // SearXNG is the self-hosted category's only option in this build.
    await page
      .getByRole("combobox", { name: "Self-hosted" })
      .click();
    await page.getByRole("option", { name: "SearXNG" }).click();

    const connectDialog = page.getByTestId("search-connect-provider");
    await expect(connectDialog).toBeVisible();
    await page.getByTestId("search-connect-endpoint").fill(SEARXNG_ENDPOINT);

    const connected = page.waitForRequest(
      (request) =>
        isSearchProviders(new URL(request.url())) && request.method() === "POST",
    );
    await page.getByTestId("search-connect-submit").click();
    await connected;

    await expect(connectDialog).toBeHidden({ timeout: 10_000 });
    const row = page.getByTestId("search-provider-searxng");
    await expect(row).toBeVisible();
    await expect(row.getByTestId("search-provider-searxng-default")).toBeVisible();
  });
});

test.describe("removing the search default needs confirmation", () => {
  test.beforeEach(async ({ page }) => {
    await page.route(
      (url) => isSearchStatus(url),
      async (route: Route) => {
        if (route.request().method() !== "GET") return route.fallback();
        await route.fulfill({ json: searxngStatus() });
      },
    );
  });

  test("opens a confirm dialog, re-opens on a stale-UI 409, and succeeds once confirmed", async ({
    page,
  }) => {
    await page.route(
      (url) => isSearchProviderSlug(url, "searxng"),
      async (route: Route) => {
        if (route.request().method() !== "DELETE") return route.fallback();
        const confirmed = new URL(route.request().url()).searchParams.get(
          "confirmInUse",
        );
        if (confirmed !== "true") {
          await route.fulfill({ status: 409, json: IN_USE_BODY });
          return;
        }
        const after = emptyStatus();
        await route.fulfill({ json: after });
      },
    );

    await openSearch(page);

    await page.getByTestId("search-provider-searxng-menu").click();
    await page.getByRole("menuitem", { name: "Remove" }).click();

    const dialog = page.getByTestId("search-confirm");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("heading", { name: "Remove SearXNG?" })).toBeVisible();
    // The generic question — this UI has not yet been told anything depends
    // on the row.
    await expect(dialog).not.toContainText(IN_USE_ERROR);

    const confirm = page.getByTestId("search-confirm-action");

    // First click: no `confirmInUse` yet, so the stub answers 409. The
    // dialog must stay open and now show the host's own reason rather than a
    // generic error toast.
    await confirm.click();
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(IN_USE_ERROR);

    // Second click, now informed: this one sends `?confirmInUse=true`, the
    // stub answers 200, and the dialog closes.
    const confirmed = page.waitForRequest(
      (request) =>
        isSearchProviderSlug(new URL(request.url()), "searxng") &&
        request.method() === "DELETE" &&
        new URL(request.url()).searchParams.get("confirmInUse") === "true",
    );
    await confirm.click();
    await confirmed;
    await expect(dialog).toBeHidden({ timeout: 10_000 });
  });

  test("Cancel leaves the dialog with nothing sent", async ({ page }) => {
    let calls = 0;
    await page.route(
      (url) => isSearchProviderSlug(url, "searxng"),
      async (route: Route) => {
        calls++;
        await route.fulfill({ status: 409, json: IN_USE_BODY });
      },
    );

    await openSearch(page);

    await page.getByTestId("search-provider-searxng-menu").click();
    await page.getByRole("menuitem", { name: "Remove" }).click();
    await expect(page.getByTestId("search-confirm")).toBeVisible();

    await page.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByTestId("search-confirm")).toBeHidden();
    expect(calls, "Cancel must send no request at all").toBe(0);
  });
});
