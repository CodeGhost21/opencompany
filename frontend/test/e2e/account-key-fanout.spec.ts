import { expect, test } from "@playwright/test";

/**
 * The Account page's key dialog, fully mocked (keys rework, issue #2306,
 * slice 4b) — `docs/key-reworks/phase-4b-account-dialog.md` §6, plus the
 * decision "X10" (2026-09-15) that a successful *live* account-key save
 * cannot be exercised here at all: this repo has no real TinyHumans key to
 * give it, so a live save would either hit the network or fail before it
 * proves anything. Every case below is instead driven through `page.route`
 * stubs shaped exactly like `company_key::fan_out`'s real response
 * (`MutationResponse`, `src/server/ops/company_key.rs`), following
 * `composio-mode-switch.spec.ts`'s pattern: a running host
 * (`playwright.config.ts` brings one up), with the one route under test
 * (`GET`/`PUT …/credential`) answered by this file instead of a real
 * TinyHumans backend. No real credential anywhere — every key below is
 * obviously fake (`th-not-a-real-key`, `th-not-a-real-key-2`,
 * `th-not-a-real-key-custom`, the exact matrix the phase-4a plan itself
 * uses).
 *
 * Coverage, beyond the phase-4b doc's own single `needsModel` case (decision
 * "X10" asks for the success path in full since nothing here can reach a
 * live one):
 *  - all three non-null fill-line variants, and the no-line case
 *  - a rotation (an existing account key replaced by a new one)
 *  - a clear that only touches copies still equal to the old key
 *  - a save that overwrites neither derived slot when both hold a custom key
 *  - an auth rejection that rolls the LLM copy back while Composio keeps its
 */

type Page = import("@playwright/test").Page;
type Route = import("@playwright/test").Route;

const isCredential = (url: URL) => /\/credential$/.test(url.pathname);

const KEY_A = "th-not-a-real-key";
const KEY_B = "th-not-a-real-key-2";
const KEY_CUSTOM = "th-not-a-real-key-custom";

/** A `GET …/credential` status, shaped like the real `CredentialStatusDto`. */
function status(over: Record<string, unknown> = {}) {
  return {
    configured: false,
    source: "none",
    notice: "This is the company's TinyHumans account key.",
    hubLink: false,
    inferenceHasOwnKey: false,
    composioHasOwnKey: false,
    defaultSet: false,
    ...over,
  };
}

/** One `SlotReportDto`. */
function slotReport(slot: string, outcome: string, detail?: string) {
  return detail === undefined ? { slot, outcome } : { slot, outcome, detail };
}

/** A `MutationResponse`, shaped like the real host's `PUT …/credential` answer. */
function mutation(over: Record<string, unknown> = {}) {
  return {
    status: status({ configured: true, source: "company" }),
    note: "Key saved.",
    slots: [
      slotReport("composio", "filled"),
      slotReport("inference", "filled"),
      slotReport("provider", "skipped", "needsModel"),
      slotReport("default", "skipped", "needsModel"),
      slotReport("health", "ok"),
    ],
    needsModel: false,
    setsDefault: false,
    ...over,
  };
}

/** Answers `GET …/credential` with a fixed body, every time it is asked. */
async function stubStatus(page: Page, body: Record<string, unknown>): Promise<void> {
  await page.route(isCredential, async (route: Route) => {
    if (route.request().method() !== "GET") return route.fallback();
    await route.fulfill({ json: body });
  });
}

/** Answers `PUT …/credential` from a fixed queue, one response per call. */
async function stubSaves(page: Page, responses: Record<string, unknown>[]): Promise<void> {
  let call = 0;
  await page.route(isCredential, async (route: Route) => {
    if (route.request().method() !== "PUT") return route.fallback();
    const response = responses[Math.min(call, responses.length - 1)];
    call += 1;
    await route.fulfill({ json: response });
  });
}

/** Open the Account page with the first-run tour out of the way. */
async function openAccount(page: Page): Promise<void> {
  await page.goto("/#/connections/api-key");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already dismissed in this context */
    });
  await expect(
    page.getByTestId("account-rows").or(page.getByTestId("account-empty")),
  ).toBeVisible({ timeout: 30_000 });
}

const toasts = (page: Page) => page.locator("[data-sonner-toast]");

test.describe("the fill line names only the slots saving would fill", () => {
  test("both slots empty: names LLM and Composio, without claiming LLM is connected", async ({
    page,
  }) => {
    await stubStatus(page, status({ inferenceHasOwnKey: false, composioHasOwnKey: false }));
    await openAccount(page);
    await page.getByTestId("account-add-key").click();

    const line = page.getByTestId("account-key-fill-line");
    await expect(line).toContainText(
      "Saving also adds this key to TinyHumans on the LLM page — choose a model there to finish — and connects it for Composio.",
    );
    await expect(line).not.toContainText("connects TinyHumans for LLM");
    await expect(page.getByTestId("account-key-llm-link")).toBeVisible();
    await expect(page.getByTestId("account-key-composio-link")).toBeVisible();
  });

  test("only the LLM slot is empty: names LLM alone, and still does not say connected", async ({
    page,
  }) => {
    await stubStatus(page, status({ inferenceHasOwnKey: false, composioHasOwnKey: true }));
    await openAccount(page);
    await page.getByTestId("account-add-key").click();

    const line = page.getByTestId("account-key-fill-line");
    await expect(line).toContainText(
      "Saving also adds this key to TinyHumans on the LLM page — choose a model there to finish.",
    );
    await expect(page.getByTestId("account-key-llm-link")).toBeVisible();
    await expect(page.getByTestId("account-key-composio-link")).toHaveCount(0);
  });

  test("only the Composio slot is empty: names Composio alone", async ({ page }) => {
    await stubStatus(page, status({ inferenceHasOwnKey: true, composioHasOwnKey: false }));
    await openAccount(page);
    await page.getByTestId("account-add-key").click();

    const line = page.getByTestId("account-key-fill-line");
    await expect(line).toContainText("Saving also connects TinyHumans for Composio.");
    await expect(page.getByTestId("account-key-llm-link")).toHaveCount(0);
    await expect(page.getByTestId("account-key-composio-link")).toBeVisible();
  });

  test("both slots already hold their own key: no line at all", async ({ page }) => {
    await stubStatus(page, status({ inferenceHasOwnKey: true, composioHasOwnKey: true }));
    await openAccount(page);
    await page.getByTestId("account-add-key").click();

    await expect(page.getByTestId("account-key-fill-line")).toHaveCount(0);
  });
});

test("saving asks for a model when the host needs one, then reposts the key with it", async ({
  page,
}) => {
  await stubStatus(page, status({ inferenceHasOwnKey: false, composioHasOwnKey: false }));
  await stubSaves(page, [
    mutation({
      status: status({ configured: true, source: "company" }),
      note: "Key saved. Choose a model to finish setting up TinyHumans for LLM.",
      slots: [
        slotReport("composio", "filled"),
        slotReport("inference", "filled"),
        slotReport("provider", "skipped", "needsModel"),
        slotReport("default", "skipped", "needsModel"),
        slotReport("health", "ok"),
      ],
      needsModel: true,
      setsDefault: true,
      models: ["acme/test-model", "acme/other-model"],
    }),
    mutation({
      status: status({ configured: true, source: "company", defaultSet: true }),
      note:
        "Key saved. TinyHumans is set up for LLM with acme/test-model. It is now the default for new work.",
      slots: [
        slotReport("composio", "kept", "alreadyCurrent"),
        slotReport("inference", "kept", "alreadyCurrent"),
        slotReport("provider", "filled"),
        slotReport("default", "filled"),
        slotReport("health", "ok"),
      ],
      needsModel: false,
      setsDefault: false,
    }),
  ]);

  await openAccount(page);
  await page.getByTestId("account-add-key").click();
  await expect(page.getByTestId("account-key-fill-line")).toContainText(
    "Saving also adds this key to TinyHumans on the LLM page — choose a model there to finish — and connects it for Composio.",
  );

  await page.getByTestId("account-key-input").fill(KEY_A);
  const firstSave = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-key-save").click();
  const first = await firstSave;
  expect(first.postDataJSON()).toEqual({ key: KEY_A });

  await expect(page.getByTestId("account-key-model-step")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Choose the model new work uses" })).toBeVisible();
  await expect(page.getByTestId("account-key-note")).toHaveText(
    "Key saved. Choose a model to finish setting up TinyHumans for LLM.",
  );

  // The real combobox, in a real browser — `inference-mocked.spec.ts`'s own
  // pattern for the catalogue select `ModelCombobox` renders.
  const trigger = page.locator("#account-key-model");
  await expect(trigger).toHaveText("Choose a model");
  await trigger.click();
  await page.getByRole("listbox", { name: "Models" }).getByRole("option", { name: "acme/test-model" }).click();
  await expect(trigger).toHaveText("acme/test-model");

  const secondSave = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-key-model-save").click();
  const second = await secondSave;
  expect(second.postDataJSON()).toEqual({ key: KEY_A, model: "acme/test-model" });

  await expect(page.getByTestId("account-key-model-step")).toHaveCount(0);
  const message = toasts(page).first();
  await expect(message).toBeVisible({ timeout: 10_000 });
  await expect(message).toContainText("TinyHumans is set up for LLM");
});

test("rotation: an existing account key is replaced by a new one", async ({ page }) => {
  // `replacing` (`canRemoveKey`) is keyed on `source === "company"`, so the
  // header's Connect button is gone and the row's own menu carries Replace.
  await stubStatus(
    page,
    status({
      configured: true,
      source: "company",
      inferenceHasOwnKey: false,
      composioHasOwnKey: false,
      defaultSet: true,
    }),
  );
  await stubSaves(page, [
    mutation({
      note: "Key saved.",
      slots: [
        slotReport("composio", "rotated"),
        slotReport("inference", "rotated"),
        slotReport("provider", "kept", "rowExists"),
        slotReport("default", "kept", "defaultAlreadySet"),
        slotReport("health", "ok"),
      ],
      needsModel: false,
      setsDefault: false,
    }),
  ]);

  await openAccount(page);
  await expect(page.getByTestId("account-add-key")).toHaveCount(0);
  await page.getByTestId("account-row-menu").click();
  await page.getByRole("menuitem", { name: "Replace key" }).click();

  await expect(page.getByRole("heading", { name: "Replace your API key" })).toBeVisible();
  await page.getByTestId("account-key-input").fill(KEY_B);
  const saved = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-key-save").click();
  const request = await saved;
  expect(request.postDataJSON()).toEqual({ key: KEY_B });

  // No model step on a rotation into an existing row/default — the dialog
  // closes at once.
  await expect(page.getByTestId("account-key-input")).toHaveCount(0);
  const message = toasts(page).first();
  await expect(message).toBeVisible({ timeout: 10_000 });
  await expect(message).toContainText("Key saved.");
});

test("clearing removes only the copies still equal to the old key — a custom key elsewhere survives", async ({
  page,
}) => {
  await stubStatus(
    page,
    status({
      configured: true,
      source: "company",
      // The Composio copy is still the fanned-out account key; the LLM copy
      // was pasted by hand on the LLM page and is not the account key.
      inferenceHasOwnKey: true,
      composioHasOwnKey: false,
      defaultSet: true,
    }),
  );
  await stubSaves(page, [
    mutation({
      status: status({ configured: false, source: "none" }),
      note:
        "Key removed. Composio's copy was removed too. LLM keeps the TinyHumans key set on its own page.",
      slots: [
        slotReport("composio", "cleared"),
        slotReport("inference", "kept", "customKey"),
        slotReport("provider", "skipped", "keyCleared"),
        slotReport("default", "skipped", "keyCleared"),
        slotReport("health", "skipped", "keyCleared"),
      ],
      needsModel: false,
      setsDefault: false,
    }),
  ]);

  await openAccount(page);
  await page.getByTestId("account-row-menu").click();
  await page.getByTestId("account-remove-key").click();
  await expect(page.getByRole("heading", { name: "Remove this company's account key?" })).toBeVisible();

  const cleared = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-remove-key-confirm").click();
  const request = await cleared;
  expect(request.postDataJSON()).toEqual({ key: "" });

  const message = toasts(page).first();
  await expect(message).toBeVisible({ timeout: 10_000 });
  await expect(message).toContainText("Composio's copy was removed too.");
  await expect(message).toContainText("LLM keeps the TinyHumans key set on its own page.");
});

test("never overwrites a key set on another page — both derived slots are kept", async ({
  page,
}) => {
  await stubStatus(page, status({ inferenceHasOwnKey: true, composioHasOwnKey: true }));
  await stubSaves(page, [
    mutation({
      note:
        "Key saved. Composio keeps the key set on its own page. LLM keeps the TinyHumans key set on its own page.",
      slots: [
        slotReport("composio", "kept", "customKey"),
        slotReport("inference", "kept", "customKey"),
        slotReport("provider", "skipped", "customKey"),
        slotReport("default", "skipped", "customKey"),
        slotReport("health", "skipped", "customKey"),
      ],
      needsModel: false,
      setsDefault: false,
    }),
  ]);

  await openAccount(page);
  await page.getByTestId("account-add-key").click();
  // No fill line at all — saving would touch neither derived slot.
  await expect(page.getByTestId("account-key-fill-line")).toHaveCount(0);

  await page.getByTestId("account-key-input").fill(KEY_CUSTOM);
  const saved = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-key-save").click();
  const request = await saved;
  // The account key itself is still written — only the derived copies are
  // left alone.
  expect(request.postDataJSON()).toEqual({ key: KEY_CUSTOM });

  const message = toasts(page).first();
  await expect(message).toBeVisible({ timeout: 10_000 });
  await expect(message).toContainText("Composio keeps the key set on its own page.");
  await expect(message).toContainText("LLM keeps the TinyHumans key set on its own page.");
});

test("an auth rejection rolls the LLM copy back while the Composio copy stays, surfaced as the save's own toast", async ({
  page,
}) => {
  await stubStatus(page, status({ inferenceHasOwnKey: false, composioHasOwnKey: false }));
  await stubSaves(page, [
    mutation({
      status: status({ configured: true, source: "company" }),
      note:
        "Key saved. Composio now uses this key. A key you created by hand may lack the connections permission Composio needs. TinyHumans rejected this key for LLM, so the LLM copy was not kept.",
      slots: [
        slotReport("composio", "filled"),
        slotReport("inference", "rolledBack"),
        slotReport("provider", "skipped", "inferenceRejected"),
        slotReport("default", "skipped", "inferenceRejected"),
        slotReport("health", "failed", "auth"),
      ],
      // Q6/the dialog's own gotcha: an auth rejection never asks for a model —
      // there is nothing new on the LLM side to name one for.
      needsModel: false,
      setsDefault: false,
    }),
  ]);

  await openAccount(page);
  await page.getByTestId("account-add-key").click();
  await page.getByTestId("account-key-input").fill(KEY_A);
  const saved = page.waitForRequest(
    (request) => isCredential(new URL(request.url())) && request.method() === "PUT",
  );
  await page.getByTestId("account-key-save").click();
  await saved;

  // Not a save error — the account key genuinely saved. The dialog closes
  // with the host's note as the toast rather than staying open with an
  // error in it.
  await expect(page.getByTestId("account-key-input")).toHaveCount(0);
  await expect(page.getByTestId("account-key-model-step")).toHaveCount(0);
  const message = toasts(page).first();
  await expect(message).toBeVisible({ timeout: 10_000 });
  await expect(message).toContainText("Composio now uses this key.");
  await expect(message).toContainText("TinyHumans rejected this key for LLM");
});

test("a response from a host predating slice 4a degrades to a single step, with no fill line", async ({
  page,
}) => {
  // No `inferenceHasOwnKey`/`composioHasOwnKey`/`defaultSet`, and a `PUT`
  // answer with no `slots`/`needsModel` at all — exactly what an older host
  // sends.
  await stubStatus(page, {
    configured: false,
    source: "none",
    notice: "n",
    hubLink: false,
  });
  await stubSaves(page, [{ status: { configured: true, source: "company" }, note: "Key saved." }]);

  await openAccount(page);
  await page.getByTestId("account-add-key").click();
  await expect(page.getByTestId("account-key-fill-line")).toHaveCount(0);

  await page.getByTestId("account-key-input").fill(KEY_A);
  await page.getByTestId("account-key-save").click();

  await expect(page.getByTestId("account-key-input")).toHaveCount(0);
  await expect(page.getByTestId("account-key-model-step")).toHaveCount(0);
});
