import { expect, test } from "@playwright/test";

/**
 * Issue #265 — Connections → Inference must never report a successful save for
 * a save that threw the operator's key away.
 *
 * The invariant is unchanged; what upholds it is not. Managed used to be a
 * revert (`DELETE …/inference`) that could carry no credential, so a key typed
 * under a BYOK provider and left in form state by a switch back to managed was
 * dropped while the toast still said "Inference updated". That was first fixed
 * by refusing the save.
 *
 * Issue #585 made the refusal unnecessary: the company's own key on the managed
 * provider is the ordinary case, not a BYOK edge — it keeps the platform
 * endpoint and swaps only the credential — so a managed save carrying a key is
 * now a real `PUT` override that *stores* it. Nothing is discarded, so there is
 * nothing to refuse. These tests assert the invariant against the new mechanism:
 * a typed key survives the switch and lands server-side.
 *
 * This spec drives a real browser against a real host, and the `Console E2E` job
 * runs it (issue #428) — the "not part of CI" note this header used to carry
 * predates that job and was already stale. It is a merge gate, so treat a red
 * run here as a real regression rather than a stale reproduction.
 */

type Page = import("@playwright/test").Page;

/** Pick a provider from the base-ui select. */
async function pickProvider(page: Page, label: string) {
  await page.locator("#inference-provider").click();
  await page.getByRole("option", { name: label, exact: true }).click();
}

/**
 * A fresh browser context has no tour state, so the first-run welcome dialog
 * opens over the console and swallows clicks. Skip it when it shows up.
 */
async function openConnections(page: Page) {
  // Inference has a page of its own since the Connections split — it was a
  // section on the accounts page, which is the wrong neighbourhood for the
  // question it settles.
  await page.goto("/#/settings/inference");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already seen in this context — nothing to dismiss */
    });
}

test("a key typed for a BYOK provider is not discarded by switching to managed", async ({
  page,
}) => {
  await openConnections(page);

  // Managed is the default selection, and since #585 it offers the key input
  // like every other provider but Ollama — with the line that says what paying
  // for the company actually means.
  await expect(page.locator("#inference-key")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("inference-key-note")).toBeVisible();

  // Type a key under a BYOK provider, then switch back to managed. The value
  // survives the switch — that is the state that used to lose it.
  await pickProvider(page, "OpenRouter");
  const typed = `pw-e2e-${Date.now()}`;
  await page.locator("#inference-key").fill(typed);
  await pickProvider(page, "Managed (TinyHumans)");
  await expect(page.locator("#inference-key")).toHaveValue(typed);

  // Saving now stores it rather than reverting past it. The credential is
  // write-only, so `keyConfigured` is the only observable — run this against a
  // fresh `--home` for it to mean "this save stored it".
  await page.getByTestId("inference-save").click();
  await expect(
    page.getByText(/Inference updated\.|Inference saved — restart the company/),
  ).toBeVisible({ timeout: 30_000 });

  const after = await page.request.get("/api/v1/company/inference");
  expect(after.ok()).toBeTruthy();
  const body = await after.json();
  expect(body.keyConfigured).toBe(true);
  // Setting only a key must not move the company off the managed brain.
  expect(body.provider).toBe("openrouter");

  // And it can be taken back off again — set / rotate / clear, all from here.
  await page.getByTestId("inference-remove-key").click();
  await expect(page.getByText("Removed the company key.")).toBeVisible({ timeout: 30_000 });
  const cleared = await page.request.get("/api/v1/company/inference");
  expect((await cleared.json()).keyConfigured).toBe(false);
});

test("a key typed for a BYOK provider does reach the host on save", async ({ page }) => {
  // The managed case above must not be the only one that lands: the same input,
  // saved under a provider with its own endpoint, still has to reach the host.
  await openConnections(page);
  await expect(page.locator("#inference-key")).toBeVisible({ timeout: 30_000 });

  await pickProvider(page, "Custom (OpenAI-compatible)");
  await page.locator("#inference-base-url").fill("http://127.0.0.1:9/v1");
  await page.locator("#inference-model-chat-v1").fill("pw-e2e-model");
  await page.locator("#inference-key").fill(`pw-e2e-${Date.now()}`);
  await page.getByTestId("inference-save").click();

  // Either success wording is correct here, and which one shows is not this
  // spec's business: a company that booted with no inference source is on the
  // echo brain, so issue #266 makes the host report `restartRequired` for
  // exactly this not-configured → configured save and the toast says "restart"
  // instead of "next turn". What #265 asserts is that the save was *accepted*
  // and the key kept — the stored-credential check below is the real proof.
  await expect(
    page.getByText(/Inference updated\.|Inference saved — restart the company/),
  ).toBeVisible({ timeout: 30_000 });
  const status = await page.request.get("/api/v1/company/inference");
  expect(status.ok()).toBeTruthy();
  const body = await status.json();
  expect(body.keyConfigured).toBe(true);
  expect(body.provider).toBe("openai_compatible");

  // Put the company back on the committed default for whatever runs next.
  await page.getByRole("button", { name: "Reset to default" }).click();
  await expect(
    page.getByText("Reverted to the committed manifest (or managed) configuration."),
  ).toBeVisible({ timeout: 30_000 });

  // The reset is a full one, not a half-clear: the host also wipes the stored
  // credential on revert (issue #993), so nothing is left behind to reroute the
  // later specs in this lane (the live-brain workflow and MCP-agent specs) off
  // the mock brain and 401 them. Assert that here rather than clearing by hand
  // — the remove-key button exists only while a key is stored, so it being gone
  // is the observable that the reset actually cleared the key.
  await expect(page.getByTestId("inference-remove-key")).toHaveCount(0, {
    timeout: 30_000,
  });
  const cleared = await page.request.get("/api/v1/company/inference");
  expect((await cleared.json()).keyConfigured).toBe(false);
});

test("changing provider asks before replacing a typed endpoint or model", async ({ page }) => {
  await openConnections(page);
  await expect(page.locator("#inference-key")).toBeVisible({ timeout: 30_000 });

  await pickProvider(page, "Custom (OpenAI-compatible)");
  await page.locator("#inference-base-url").fill("https://models.example.test/v1");
  await page.locator("#inference-model-chat-v1").fill("operator-draft");
  await pickProvider(page, "OpenRouter");

  await expect(page.getByRole("alertdialog")).toContainText("replaces the typed Base URL and model fields");
  await page.getByRole("button", { name: "Keep draft" }).click();
  await expect(page.locator("#inference-provider")).toContainText("Custom (OpenAI-compatible)");
  await expect(page.locator("#inference-base-url")).toHaveValue("https://models.example.test/v1");
  await expect(page.locator("#inference-model-chat-v1")).toHaveValue("operator-draft");
});

test("OpenRouter models are selected from the registry and persist through reload", async ({
  page,
}) => {
  await page.route("**/inference/models", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        { id: "provider/catalog-chat", name: "Catalog Chat", contextLength: 128_000 },
        { id: "provider/catalog-reasoning", name: "Catalog Reasoning" },
      ]),
    });
  });
  await openConnections(page);
  await expect(page.locator("#inference-provider")).toBeVisible({ timeout: 30_000 });

  await pickProvider(page, "OpenRouter");
  const chat = page.getByTestId("inference-model-select-chat-v1");
  await expect(chat).toBeEnabled();
  await chat.click();
  await page.getByRole("option", { name: /Catalog Chat/ }).click();
  await page.getByTestId("inference-save").click();
  await expect(
    page.getByText(/Inference updated\.|Inference saved — restart the company/),
  ).toBeVisible({ timeout: 30_000 });

  const saved = await page.request.get("/api/v1/company/inference");
  expect(saved.ok()).toBeTruthy();
  expect((await saved.json()).models["chat-v1"]).toBe("provider/catalog-chat");

  await page.reload();
  await expect(page.getByTestId("inference-model-select-chat-v1")).toContainText("Catalog Chat", {
    timeout: 30_000,
  });

  await pickProvider(page, "Custom (OpenAI-compatible)");
  await expect(page.locator("input#inference-model-chat-v1")).toBeVisible();

  // Leave the shared E2E company on its committed default for later specs.
  await page.getByRole("button", { name: "Reset to default" }).click();
  await expect(
    page.getByText("Reverted to the committed manifest (or managed) configuration."),
  ).toBeVisible({ timeout: 30_000 });
});
