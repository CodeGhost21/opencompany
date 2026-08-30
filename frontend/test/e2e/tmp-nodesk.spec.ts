import { expect, test } from "@playwright/test";

test("a deskless company shows no fabricated channels", async ({ page }) => {
  await page.goto("/#/chat");
  const skip = page.getByRole("button", { name: /skip for now/i });
  if (await skip.count()) await skip.first().click({ timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(4000);
  const desks = await page.evaluate(async () => {
    const r = await fetch("/api/v1/companies/e2e-harness-co/desks", { credentials: "include" });
    return { status: r.status, body: await r.text() };
  });
  console.log("DESKS " + JSON.stringify(desks).slice(0, 400));
  await page.screenshot({ path: "/private/tmp/claude-501/-Users-enamakel-work-tinyhumansai-workflow-opencompany-opencompany/1924ee62-1853-4aaa-810e-b9aaa6f47a16/scratchpad/chat.png" });
  await expect(page.getByText("Creative studio")).toHaveCount(0);
});
