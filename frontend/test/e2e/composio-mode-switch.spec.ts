import { expect, test } from "@playwright/test";

/**
 * Keys rework (issue #2306) — every destructive action and every on/off
 * toggle on the Composio page gets a confirm dialog, and the host refuses an
 * in-use clear/switch unless the request confirms it
 * (`docs/key-reworks/in-use-guards.md`).
 *
 * # Two gaps this closes
 *
 * `ComposioSection` already asked before the FIRST move onto BYOK (an inline
 * warning inside the credential dialog, `confirmSwitch`). It asked nothing at
 * all before clearing the managed token, or before giving the managed route
 * back — both used to write immediately on click. Those two are what this
 * file drives.
 *
 * # Fully mocked, no live Composio
 *
 * Every request this spec cares about is stubbed at the wire with
 * `page.route`, following `connections-native-not-offered.spec.ts`'s pattern:
 * a running host (`playwright.config.ts` brings one up), with the handful of
 * routes under test answered by this file instead of a real backend. No
 * `COMPOSIO` fixture, no network to Composio, and no real credential — the
 * pasted values below are obviously fake
 * (`th-not-a-real-key`/`ak-not-a-real-key`) and the host never dials out for
 * them because the PUT handlers themselves are intercepted.
 */

type Page = import("@playwright/test").Page;
type Route = import("@playwright/test").Route;

/** The status route, and only it — not `.../token`, `.../api-key`, `.../connections`. */
const isComposioStatus = (url: URL) => /\/composio$/.test(url.pathname);
const isComposioToken = (url: URL) => /\/composio\/token$/.test(url.pathname);
const isComposioApiKey = (url: URL) => /\/composio\/api-key$/.test(url.pathname);

const IN_USE_ERROR = "Composio's key is used by Composio.";
const IN_USE_BODY = {
  error: IN_USE_ERROR,
  code: "in_use",
  usedBy: { surfaces: ["composio"] as const },
};

/** A status where the managed route is active and holds a stored token. */
function managedStatus() {
  return {
    inBuild: true,
    granted: true,
    credentialSource: "static",
    managedCredentialSource: "static",
    mode: "managed",
    backendUrl: "https://api.tinyhumans.ai",
    toolkits: ["gmail"],
    openMode: false,
    effectiveToolkits: ["gmail"],
    effectiveCatalog: [],
    catalogSource: "manifest",
    catalogNotice: null,
  };
}

/**
 * A status where BYOK is active — the managed chain still resolves
 * (`managedCredentialSource: "attested"`), so the managed row offers "Use
 * this" rather than hiding it as a switch into an outage.
 */
function byokStatus() {
  return {
    inBuild: true,
    granted: true,
    credentialSource: "static",
    managedCredentialSource: "attested",
    mode: "byok",
    backendUrl: "https://backend.composio.dev",
    toolkits: ["gmail"],
    openMode: false,
    effectiveToolkits: ["gmail"],
    effectiveCatalog: [],
    catalogSource: "manifest",
    catalogNotice: null,
  };
}

/** Stub `GET .../composio` to answer with a fixed status, every time it is asked. */
async function stubStatus(page: Page, status: Record<string, unknown>): Promise<void> {
  await page.route(
    (url) => isComposioStatus(url),
    async (route: Route) => {
      if (route.request().method() !== "GET") return route.fallback();
      await route.fulfill({ json: status });
    },
  );
}

/**
 * Stub a guarded PUT route (`.../token` or `.../api-key`) to answer 409
 * `in_use` on a request that does not confirm, and 200 (echoing `usedBy`) on
 * one that does — exactly the host's own guard behaviour
 * (`src/server/ops/composio.rs`), without a real backend behind it.
 */
async function stubGuardedRoute(
  page: Page,
  matches: (url: URL) => boolean,
  okStatus: Record<string, unknown>,
  okNote: string,
): Promise<void> {
  await page.route(
    (url) => matches(url),
    async (route: Route) => {
      if (route.request().method() !== "PUT") return route.fallback();
      const body = route.request().postDataJSON() as { confirmInUse?: boolean };
      if (body.confirmInUse !== true) {
        await route.fulfill({ status: 409, json: IN_USE_BODY });
        return;
      }
      await route.fulfill({
        json: {
          status: okStatus,
          note: okNote,
          usedBy: { surfaces: ["composio"] },
        },
      });
    },
  );
}

/** Open the Composio page with the first-run tour out of the way. */
async function openComposio(page: Page): Promise<void> {
  await page.goto("/#/connections/composio");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already dismissed in this context */
    });
  await expect(skip).toBeHidden({ timeout: 10_000 });
  await expect(page.getByRole("heading", { name: "Connected" })).toBeVisible({
    timeout: 30_000,
  });
}

test.describe("clearing the managed-route token", () => {
  test.beforeEach(async ({ page }) => {
    await stubStatus(page, managedStatus());
    await stubGuardedRoute(page, isComposioToken, managedStatus(), "Composio token cleared.");
  });

  test("opens a confirm dialog naming the action, and re-opens on a stale-UI 409", async ({
    page,
  }) => {
    await openComposio(page);

    await page.getByTestId("composio-row-managed-remove").click();

    const dialog = page.getByTestId("composio-clear-token-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("heading", { name: "Disconnect Composio?" })).toBeVisible();
    // The generic question — this UI has not yet been told anything depends
    // on the token.
    await expect(dialog).not.toContainText(IN_USE_ERROR);

    const confirm = page.getByTestId("composio-clear-token-confirm");
    await expect(confirm).toHaveText(/Disconnect Composio/);

    // First click: no `confirmInUse` yet, so the stub answers 409. The
    // dialog must stay open and now show the host's own reason rather than a
    // generic error toast.
    await confirm.click();
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(IN_USE_ERROR);

    // Second click, now informed: this one sends `confirmInUse: true`, the
    // stub answers 200, and the dialog closes.
    const confirmed = page.waitForRequest(
      (request) =>
        isComposioToken(new URL(request.url())) &&
        request.method() === "PUT" &&
        (request.postDataJSON() as { confirmInUse?: boolean }).confirmInUse === true,
    );
    await confirm.click();
    await confirmed;
    await expect(dialog).toBeHidden({ timeout: 10_000 });
  });

  test("Cancel leaves the dialog with nothing sent", async ({ page }) => {
    await openComposio(page);

    let calls = 0;
    await page.route(isComposioToken, async (route) => {
      calls++;
      await route.fulfill({ status: 409, json: IN_USE_BODY });
    });

    await page.getByTestId("composio-row-managed-remove").click();
    await expect(page.getByTestId("composio-clear-token-dialog")).toBeVisible();

    await page.getByRole("button", { name: "Keep the token" }).click();
    await expect(page.getByTestId("composio-clear-token-dialog")).toBeHidden();
    expect(calls, "Cancel must send no request at all").toBe(0);
  });
});

test.describe("switching Composio back to the managed route", () => {
  test.beforeEach(async ({ page }) => {
    await stubStatus(page, byokStatus());
    await stubGuardedRoute(
      page,
      isComposioApiKey,
      managedStatus(),
      "Composio API key cleared.",
    );
  });

  test("opens a confirm dialog naming the action, and sends confirmInUse on the informed retry", async ({
    page,
  }) => {
    await openComposio(page);

    // The managed row's radio is this route's own "Use this" — see
    // `composioRows`: it is offered because `managedCredentialSource` here
    // resolves ("attested"), so choosing it is not a switch into an outage.
    await page.getByTestId("composio-row-managed-select").click();

    const dialog = page.getByTestId("composio-use-managed-dialog");
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByRole("heading", {
        name: "Switch Composio to the TinyHumans-managed route?",
      }),
    ).toBeVisible();
    await expect(dialog).not.toContainText(IN_USE_ERROR);

    const confirm = page.getByTestId("composio-use-managed-confirm");

    // First click: unconfirmed, refused, dialog reopens with the host's own
    // reason.
    await confirm.click();
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(IN_USE_ERROR);

    // Second click sends `confirmInUse: true` and an empty `apiKey` — the
    // clear-and-switch-back this route always performs together.
    const confirmed = page.waitForRequest((request) => {
      if (!isComposioApiKey(new URL(request.url())) || request.method() !== "PUT") {
        return false;
      }
      const body = request.postDataJSON() as { apiKey?: string; confirmInUse?: boolean };
      return body.confirmInUse === true && body.apiKey === "";
    });
    await confirm.click();
    await confirmed;
    await expect(dialog).toBeHidden({ timeout: 10_000 });
  });
});
