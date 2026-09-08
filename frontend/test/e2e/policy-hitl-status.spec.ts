import { expect, test } from "@playwright/test";

/**
 * The policy screen must state whether policy-generated approvals (the
 * always-ask list, the spend cap, the tier) are actually creating approval
 * cards, and it must be reading that from the running host rather than
 * assuming it in the console's own TypeScript.
 *
 * The harness company (`companies/e2e_harness/company.toml`) runs with
 * policy-generated HITL disabled, like every host this suite can drive today
 * (`src/runtime/builder.rs` disables it unconditionally on the production
 * path) — so this spec pins the "disabled" half against the real, running
 * server: the honest banner, the two "(inactive)" fields, and the fact that
 * neither is a Save an operator can press. It is the case
 * `settings-authority.spec.ts` does not cover: that spec checks who is
 * offered the tier controls, not whether the always-ask list and spend cap
 * tell the truth about their own status.
 *
 * Drives a real browser against a live host, like the rest of this
 * directory, and is not a merge gate (the Playwright config declares no
 * `webServer` override here beyond the shared one).
 */

async function openGeneralSettings(page: import("@playwright/test").Page) {
  await page.goto("/#/settings/general");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* tour already dismissed in this context */
    });
}

test("the Approvals card states plainly that policy-generated approvals are disabled", async ({
  page,
}) => {
  await openGeneralSettings(page);
  const banner = page.getByTestId("policy-hitl-status");
  await expect(banner).toBeVisible({ timeout: 30_000 });
  await expect(banner).toContainText("Policy-based approval prompts are disabled");
  await expect(banner).toContainText("request_approval");
  await expect(banner).toContainText("paid-media");
  await expect(banner).toContainText("requires_approval");
  await expect(banner).not.toContainText("active");
});

test("the spend cap and always-ask fields say they are inactive and cannot be saved", async ({
  page,
}) => {
  await openGeneralSettings(page);
  await expect(page.getByText("Spend approval threshold (inactive)")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator("#spend-cap")).toBeDisabled();
  await expect(page.getByText("Always ask first (inactive)")).toBeVisible();
  await expect(page.locator("#always-approve")).toBeDisabled();
  // Typing into the disabled field is a no-op, so no "Save list" button ever
  // appears — there is nothing here for a save to lie about.
  await expect(page.getByRole("button", { name: "Save list" })).toHaveCount(0);
});
