import { expect, test, type Page } from "@playwright/test";

/**
 * Proof for issue #584: inviting someone reports what actually reached them.
 *
 * The bug was not a delivery failure — there was no invite email at all, and
 * the console toasted unconditional success over it. QA went looking for a mail
 * nobody had sent. The dialog even carried a "Nothing is emailed now"
 * disclaimer at the time, and the misunderstanding happened anyway: the success
 * toast was the louder signal, and it was the wrong one.
 *
 * The first case here is verifiable end to end against the real host, which is
 * the point of putting it in this suite rather than in a unit test. `host.sh`
 * brings the host up with `env -i` and no `OPENCOMPANY_MAIL_*` transport (it
 * has to — the magic-link `dev_code` echo that global-setup signs in with is
 * gated on exactly that). So this host genuinely cannot send mail, and the
 * `no_transport` path is exercised for real rather than simulated.
 *
 * The `sent` rendering has no such luxury: no environment in CI speaks SMTP.
 * It is asserted against a stubbed response, which proves the console renders
 * the outcome correctly but proves nothing about the wire. The backend suite
 * covers the send itself at the `MailSender` seam; final proof that mail leaves
 * the building is manual, on a mail-wired host.
 */

/**
 * The first-run product tour opens a modal over a fresh console and would
 * swallow every click. `src/tour/state.ts` keys "seen" per company in
 * localStorage, so pre-seed every key this host could use before the app boots,
 * and click the dialog away if one still slips through.
 */
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

async function dismissTour(page: Page) {
  const skip = page.getByRole("button", { name: "Skip for now" });
  for (let attempt = 0; attempt < 5; attempt += 1) {
    if (!(await skip.isVisible().catch(() => false))) return;
    await skip.click({ force: true }).catch(() => {});
    await page.waitForTimeout(300);
  }
  await expect(skip).toHaveCount(0);
}

/** A fresh address per run, so a re-run never collides with its own invite. */
function unusedAddress(tag: string): string {
  return `invite-${tag}-${Date.now()}@example.test`;
}

async function openPeople(page: Page) {
  await page.goto("/#/settings/people");
  await dismissTour(page);
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible({
    timeout: 30_000,
  });
}

async function inviteFromTheDialog(page: Page, email: string) {
  await page.getByRole("button", { name: "Invite" }).first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  // The dialog must no longer promise the opposite of what the button does.
  // "Nothing is emailed now" was true when it was written and is a lie now.
  await expect(dialog).not.toContainText("Nothing is emailed now");

  await dialog.getByLabel("Email").fill(email);
  await dialog.getByRole("button", { name: "Invite" }).click();
}

test("a host that cannot send email says so instead of reporting success", async ({
  page,
}) => {
  await openPeople(page);
  const email = unusedAddress("notransport");
  await inviteFromTheDialog(page, email);

  // The operator is told the grant landed AND that nobody was emailed. Both
  // halves matter: reporting only the failure would suggest the invite did not
  // take, and reporting only the success is the bug.
  const notice = page.getByText(/nothing was emailed/i);
  await expect(notice).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText(/tell them yourself/i)).toBeVisible();

  // And the roster row carries the same fact after the toast is gone, because
  // the follow-up the operator owes outlives a four-second notification.
  const row = page
    .locator("div")
    .filter({ hasText: email })
    .locator('[data-testid="invite-meta"]')
    .first();
  await expect(row).toContainText("no invite email sent");
  await expect(row).toContainText("let them know");

  // Host-backed, not a client-side guess: it survives a reload.
  await page.reload();
  await openPeople(page);
  await expect(
    page
      .locator("div")
      .filter({ hasText: email })
      .locator('[data-testid="invite-meta"]')
      .first(),
  ).toContainText("no invite email sent");
});

test("a delivered invite is reported as emailed, and the roster says so", async ({
  page,
}) => {
  const email = unusedAddress("sent");

  // Stubbed, and labelled as such: this host has no transport, so `sent` is
  // not reachable here. What is under test is the console's rendering of the
  // outcome, not the send.
  await page.route("**/users/invites", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: "i-stub",
        email,
        role: "member",
        invitedBy: "u-stub",
        createdAtMillis: Date.now(),
        expiresAtMillis: Date.now() + 14 * 24 * 60 * 60 * 1000,
        notifiedAtMillis: Date.now(),
        delivery: "sent",
      }),
    });
  });

  await openPeople(page);
  await inviteFromTheDialog(page, email);

  await expect(page.getByText(/emailed a link to sign in/i)).toBeVisible({
    timeout: 15_000,
  });
  // No warning language on the happy path.
  await expect(page.getByText(/nothing was emailed/i)).toHaveCount(0);
});
