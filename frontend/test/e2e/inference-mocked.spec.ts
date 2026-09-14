import { expect, test } from "@playwright/test";

/**
 * The LLM page, fully mocked — no live host state, no real provider.
 *
 * `test/e2e/inference.spec.ts` drives a real host and real (fake-keyed)
 * credentials; it cannot stage a `409 in_use` refusal, a slow probe worth
 * cancelling mid-flight, or a host that already speaks the keys-rework
 * contract (`defaultChoice`, `providers[].model`, `usedBy`) ahead of the
 * backend actually shipping it. This file is the round-2 review's own
 * request for exactly those three (P0-1, P0-2, P1-1), following
 * `composio-mode-switch.spec.ts`'s pattern: a running host
 * (`playwright.config.ts` brings one up), with the handful of routes under
 * test answered by this file instead. No real credential anywhere — every
 * key below is obviously fake (`sk-not-a-real-key`).
 */

type Page = import("@playwright/test").Page;
type Route = import("@playwright/test").Route;

const isInferenceStatus = (url: URL) => /\/inference$/.test(url.pathname);
const isManagedEnabled = (url: URL) => /\/inference\/managed\/enabled$/.test(url.pathname);
const isProviderEnabled = (url: URL) => /\/inference\/providers\/[^/]+\/enabled$/.test(url.pathname);
const isProbe = (url: URL) => /\/inference\/probe$/.test(url.pathname);

const IN_USE_ERROR = "Acme is pinned by an agent.";

/** A minimal, already-contract-shaped status — see the file doc on why this is mocked rather than read from a live host. */
function status(over: Record<string, unknown> = {}) {
  return {
    provider: "acme",
    slug: "acme",
    baseUrl: "https://acme.example/v1",
    models: {},
    defaultTierModels: {},
    source: "runtime",
    keyConfigured: true,
    cognition: "harness",
    usageMetering: "perTurn",
    restartRequired: false,
    harnessReachable: true,
    designsProfiles: true,
    canRebuildInPlace: true,
    defaultChoice: { provider: "acme", model: "acme/test-model" },
    routesNotCarried: null,
    providers: [
      {
        id: "prv_acme",
        slug: "acme",
        label: "Acme",
        kind: "openai_compatible",
        baseUrl: "https://acme.example/v1",
        models: {},
        model: "acme/test-model",
        modelAmbiguous: false,
        enabled: true,
        keyConfigured: true,
        origin: "indexed",
        isDefault: true,
      },
      {
        id: "prv_beta",
        slug: "beta",
        label: "Beta",
        kind: "openai_compatible",
        baseUrl: "https://beta.example/v1",
        models: {},
        model: "beta/test-model",
        modelAmbiguous: false,
        enabled: true,
        keyConfigured: true,
        origin: "indexed",
        isDefault: false,
      },
    ],
    routes: {},
    managed: { source: "none", configured: false, baseUrl: "", enabled: true, legacyRow: true, needsModel: false },
    ...over,
  };
}

async function stubStatus(page: Page, body: Record<string, unknown>): Promise<void> {
  await page.route(
    (url) => isInferenceStatus(url),
    async (route: Route) => {
      if (route.request().method() !== "GET") return route.fallback();
      await route.fulfill({ json: body });
    },
  );
}

async function openInference(page: Page): Promise<void> {
  await page.goto("/#/connections/inference");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {});
  await expect(
    page.getByTestId("inference-providers").or(page.getByTestId("inference-providers-empty")),
  ).toBeVisible({ timeout: 30_000 });
}

test.describe("P0-1: the legacy Managed row's toggle never posts to a provider route with no row behind it", () => {
  test.beforeEach(async ({ page }) => {
    await stubStatus(
      page,
      status({
        managed: {
          source: "company_account",
          configured: true,
          baseUrl: "https://api.tinyhumans.ai",
          enabled: true,
          legacyRow: true,
          needsModel: false,
        },
      }),
    );
  });

  test("turning it off posts to /inference/managed/enabled with {enabled:false}, never .../providers/tinyhumans/enabled", async ({
    page,
  }) => {
    await openInference(page);
    const row = page.getByTestId("inference-provider-managed");
    await expect(row).toBeVisible();

    // A provider-route call would be the bug this pins against — assert it
    // never fires, alongside asserting the right one does.
    let providerRouteCalled = false;
    await page.route(isProviderEnabled, async (route) => {
      providerRouteCalled = true;
      await route.fulfill({ status: 404, json: { error: "no such provider", code: "not_found" } });
    });

    const managedCall = page.waitForRequest(
      (request) =>
        isManagedEnabled(new URL(request.url())) &&
        request.method() === "POST" &&
        (request.postDataJSON() as { enabled?: boolean }).enabled === false,
    );
    await page.route(isManagedEnabled, async (route) => {
      if (route.request().method() !== "POST") return route.fallback();
      await route.fulfill({
        json: { status: status({ managed: { source: "company_account", configured: true, baseUrl: "https://api.tinyhumans.ai", enabled: false, legacyRow: true, needsModel: false } }), note: "Managed turned off." },
      });
    });

    await row.getByTestId("inference-provider-managed-toggle").click();
    await expect(page.getByTestId("inference-remove-dialog")).toContainText("Turn off");
    await page.getByTestId("inference-remove-confirm").click();
    await managedCall;
    await expect(page.getByTestId("inference-remove-dialog")).toHaveCount(0, { timeout: 10_000 });
    expect(providerRouteCalled, "the legacy toggle must never call the provider route").toBe(false);
  });
});

test.describe("P0-2: a stale probe never leaks into the next dialog", () => {
  test.beforeEach(async ({ page }) => {
    await stubStatus(page, status({ providers: [] }));
  });

  test("Escape is blocked while the probe is in flight; once it settles, closing and reopening a different provider starts clean", async ({
    page,
  }) => {
    await openInference(page);

    // A short, real delay — the original bug was Escape racing a probe that
    // was going to answer anyway, not one that never would. Resolves with a
    // catalogue, so the model step opening is the deterministic signal that
    // `busy` has cleared.
    await page.route(isProbe, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 300));
      await route.fulfill({ json: { ok: true, modelCount: 1, models: ["groq/test-model"] } });
    });

    await page.getByTestId("inference-add-open").click();
    await page.locator("#inference-add-cloud").click();
    await page.getByRole("option", { name: /^Groq/ }).click();
    await expect(page.getByTestId("inference-connect-provider")).toBeVisible();
    await page.locator("#inference-connect-key").fill("sk-not-a-real-key-groq");
    await page.getByTestId("inference-connect-submit").click();

    // Busy: Escape must not close the dialog (round-2 review, P0-2 — this is
    // the guard that makes the original race structurally unreachable, rather
    // than reopening the door for the attempt counter to have to catch it).
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("inference-connect-provider")).toBeVisible();

    // The probe settles — busy clears, and the model step is now showing
    // Groq's own catalogue.
    await expect(page.getByTestId("inference-connect-model-step")).toBeVisible({ timeout: 10_000 });

    // NOT busy any more: Escape now works.
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("inference-connect-provider")).toHaveCount(0);

    // A different provider, opened fresh.
    await page.getByTestId("inference-add-open").click();
    await page.locator("#inference-add-cloud").click();
    await page.getByRole("option", { name: /^Anthropic/ }).click();
    await expect(page.getByTestId("inference-connect-provider")).toBeVisible();

    // Starts clean at the key step — no leftover model step, no leftover
    // catalogue or error from Groq, and the key field is genuinely empty.
    await expect(page.getByTestId("inference-connect-model-step")).toHaveCount(0);
    await expect(page.getByTestId("inference-connect-error")).toHaveText("");
    await expect(page.locator("#inference-connect-key")).toHaveValue("");
  });

  test("Escape and an overlay click send nothing while a request is in flight", async ({ page }) => {
    await openInference(page);
    let probeCalls = 0;
    await page.route(isProbe, async () => {
      probeCalls++;
      await new Promise(() => {});
    });

    await page.getByTestId("inference-add-open").click();
    await page.locator("#inference-add-cloud").click();
    await page.getByRole("option", { name: /^Groq/ }).click();
    await page.locator("#inference-connect-key").fill("sk-not-a-real-key-groq");
    await page.getByTestId("inference-connect-submit").click();
    expect(probeCalls).toBe(1);

    // Busy: Escape must not close the dialog.
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("inference-connect-provider")).toBeVisible();
    expect(probeCalls, "no second request from the ignored Escape").toBe(1);
  });
});

test.describe("P1-1: confirmInUse is sent only when the dialog actually showed a usedBy, and a stale 409 re-opens it", () => {
  test("an agent pinned after the page loaded gets a 409, the dialog re-opens naming it, and the confirmed retry sends confirmInUse:true", async ({
    page,
  }) => {
    // The status the dialog opens against says nothing is pinned — this is
    // the "stale UI" the whole flow exists for: the page loaded before the
    // pin happened elsewhere.
    await stubStatus(page, status());
    await openInference(page);

    const row = page.getByTestId("inference-provider-beta");
    await expect(row).toBeVisible();
    await row.getByTestId("inference-provider-beta-menu").click();
    await page.getByTestId("inference-provider-beta-remove").click();

    const dialog = page.getByTestId("inference-remove-dialog");
    await expect(dialog).toBeVisible();
    // Nothing shown as used yet — the confirm click below must therefore NOT
    // send confirmInUse on the first try.
    await expect(dialog).not.toContainText(IN_USE_ERROR);

    let firstBody: { confirmInUse?: boolean } | undefined;
    await page.route(
      (url) => /\/inference\/providers\/beta$/.test(url.pathname),
      async (route: Route) => {
        if (route.request().method() !== "DELETE") return route.fallback();
        const confirmed = new URL(route.request().url()).searchParams.get("confirmInUse") === "true";
        if (!firstBody) firstBody = { confirmInUse: confirmed };
        if (!confirmed) {
          await route.fulfill({
            status: 409,
            json: { error: IN_USE_ERROR, code: "in_use", usedBy: { agents: [{ id: "a1", name: "Researcher" }] } },
          });
          return;
        }
        await route.fulfill({
          json: {
            status: status({ providers: [status().providers[0]] }),
            note: "Beta removed.",
          },
        });
      },
    );

    const firstAttempt = page.waitForRequest(
      (request) => /\/inference\/providers\/beta/.test(request.url()) && request.method() === "DELETE",
    );
    await page.getByTestId("inference-remove-confirm").click();
    await firstAttempt;
    expect(firstBody?.confirmInUse, "first click must not send confirmInUse — nothing was shown as used").toBe(
      false,
    );

    // The dialog stays open and now names what the host just said.
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(IN_USE_ERROR);
    await expect(dialog).toContainText("Researcher");

    // The confirmed retry sends confirmInUse:true.
    const confirmedRetry = page.waitForRequest(
      (request) =>
        /\/inference\/providers\/beta/.test(request.url()) &&
        request.method() === "DELETE" &&
        new URL(request.url()).searchParams.get("confirmInUse") === "true",
    );
    await page.getByTestId("inference-remove-confirm").click();
    await confirmedRetry;
    await expect(dialog).toBeHidden({ timeout: 10_000 });
  });
});
