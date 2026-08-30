import { expect, test } from "@playwright/test";

// The console's behaviour when the host answers `/desks` with an empty list —
// a company that declares no `[[group_chat]]`. Intercepted rather than served
// from a deskless bundle so the harness company's sign-in still applies.
test("an empty /desks answer shows no fabricated channels", async ({ page }) => {
  await page.route("**/desks", async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    await route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
  });
  await page.goto("/#/chat");
  const skip = page.getByRole("button", { name: /skip for now/i });
  if (await skip.count()) await skip.first().click({ timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(5000);
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/chat-empty.png" });
  for (const name of ["Strategy desk", "Creative studio", "Front desk"]) {
    await expect(page.getByText(name, { exact: false })).toHaveCount(0);
  }
});
