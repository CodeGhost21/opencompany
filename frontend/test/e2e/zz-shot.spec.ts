import { test } from "@playwright/test";

test("screenshot the runs section", async ({ page, request }) => {
  const base = "/api/v1/companies/e2e-harness-co";
  // A couple of cards for the engineer, dispatched so they record attempts.
  for (const title of ["Draft the release notes", "Audit the auth middleware"]) {
    const made = await request.post(`${base}/tasks`, {
      data: { title, assignee: "engineer", priority: "high" },
    });
    const card = await made.json().catch(() => null);
    console.log("card", made.status(), JSON.stringify(card));
    if (card?.id) {
      const moved = await request.patch(`${base}/tasks/${card.id}`, {
        data: { column: "in_progress" },
      });
      console.log("dispatch", moved.status());
    }
  }
  // A chat turn, so a second kind of source shows up.
  const chat = await request.post(`${base}/chat`, {
    data: { text: "status please", chat: "engineering" },
  });
  console.log("chat", chat.status());

  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
  await page.setViewportSize({ width: 1100, height: 1400 });
  await page.goto("/#/team/engineer");
  await page.waitForTimeout(6000);
  await page.screenshot({ path: "/tmp/claude-1000/agent-runs.png", fullPage: true });

  // …and one attempt opened.
  const row = page.locator('[data-testid^="agent-run-"]').first();
  if (await row.isVisible().catch(() => false)) {
    await row.click();
    await page.waitForTimeout(2500);
    await page.screenshot({ path: "/tmp/claude-1000/agent-run-detail.png", fullPage: true });
  }
});
