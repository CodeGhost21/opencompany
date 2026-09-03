import { expect, test, type Page } from "@playwright/test";

/**
 * **Issue #2028 — reachability, which only a real click can prove.**
 *
 * The engine has taken four blocker verdicts for some time, and a green unit
 * suite said so while an operator on the Approvals page could still only send
 * two of them: approve became a retry, decline became a cancel, and skip and
 * amend had no control at all. That gap is invisible to any test that calls the
 * host directly — it exists between a pointer and a request body — so it is
 * asserted here, on the request the browser actually sends when the operator
 * clicks the thing they are looking at.
 *
 * ## What is faked, and what is not
 *
 * The approvals *list* is faked, for the reason `approval-timeout-honesty`
 * gives: parking a real blocker needs an agent turn that fails in a particular
 * way, which needs an inference credential and is not deterministic. The shape
 * served is the host's own `ApprovalSummary` with a `blocker.*` kind.
 *
 * The resolve is intercepted rather than fulfilled by the host, because what is
 * under test is the **body the console composes** — the host's own tests cover
 * what it does with that body, and pairing the two is what stops the halves
 * drifting on a token.
 *
 * Real: the host, the console bundle, the session, the routing, the polling
 * feed, and every DOM interaction.
 *
 * Like the rest of `test/e2e`, this drives a running host and is not wired into
 * CI; `npm run typecheck:e2e` compiles it, nothing runs it automatically.
 */

const BLOCKER_ID = "e2e-2028-blocker";
const GATED_ID = "e2e-2028-gated";

const isApprovalList = (url: URL) => /\/approvals$/.test(url.pathname);
const isCompanyRead = (url: URL) =>
  /\/api\/v1\/(company|companies|companies\/[^/]+)$/.test(url.pathname);
const isApprovalResolve = (url: URL) => /\/approvals\/[^/]+$/.test(url.pathname);

/** A parked workflow-node blocker, in the host's own `ApprovalSummary` shape. */
function parkedBlocker() {
  return {
    id: BLOCKER_ID,
    kind: "blocker.infrastructure",
    amount_usd: null,
    at_millis: Date.now() - 30_000,
    task: { link: "unlinked" as const },
    agent: "writer",
    payload: {
      kind: "infrastructure",
      source: "provider",
      reason: "the model id `gpt-nope` was rejected",
      needed: "a model id this provider serves",
    },
  };
}

/** An ordinary gated tool call, for the control case. */
function parkedGatedCall() {
  return {
    id: GATED_ID,
    kind: "payment.send",
    amount_usd: 42.5,
    at_millis: Date.now() - 30_000,
    task: { link: "unlinked" as const },
    agent: "finance",
    payload: { to: "acme-supplies", memo: "invoice 8812" },
  };
}

test.beforeEach(async ({ page }) => {
  // The first-run tour would open a modal over every click below.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

/**
 * Serve a fixed queue, with the company status stubbed in step with it — both
 * project the journal's parked set, and a fixture that fakes one and leaves the
 * other real is not a state the host can be in.
 */
async function stubQueue(page: Page, parked: unknown[]) {
  await page.route(isApprovalList, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(parked),
    });
  });
  await page.route(isCompanyRead, async (route) => {
    const response = await route.fetch();
    if (!response.ok()) return route.fulfill({ response });
    const body = await response.json();
    await route.fulfill({
      status: response.status(),
      contentType: "application/json",
      body: JSON.stringify(
        Array.isArray(body)
          ? body.map((c) => ({ ...c, pending_approvals: parked.length }))
          : { ...body, pending_approvals: parked.length },
      ),
    });
  });
}

/** Capture the resolve body the console composes, and answer it plausibly. */
async function captureResolve(page: Page) {
  const bodies: Record<string, unknown>[] = [];
  await page.route(isApprovalResolve, async (route) => {
    const raw = route.request().postData();
    bodies.push(raw ? JSON.parse(raw) : {});
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ responses: [], stillAwaiting: 0, outcome: "settled" }),
    });
  });
  return bodies;
}

const decideFooter = (page: Page) => page.getByTestId("approval-decide");

async function openApprovals(page: Page, parked: unknown[]) {
  await stubQueue(page, parked);
  await page.goto("/#/approvals");
  await expect(decideFooter(page).first()).toBeVisible({ timeout: 15_000 });
}

test("a blocker card offers four verdicts where an ordinary card offers two", async ({
  page,
}) => {
  await openApprovals(page, [parkedBlocker()]);
  const footer = decideFooter(page);

  // The whole issue in one assertion: the operator can see, and reach, all four.
  await expect(footer.getByRole("button", { name: /^Retry/ })).toBeVisible();
  await expect(footer.getByRole("button", { name: /^Answer this question/ })).toBeVisible();
  await expect(footer.getByRole("button", { name: /^Skip this step/ })).toBeVisible();
  await expect(footer.getByRole("button", { name: /^Cancel run/ })).toBeVisible();
  // …and the two that flattened them are gone.
  await expect(footer.getByRole("button", { name: /^Approve:/ })).toHaveCount(0);
  await expect(footer.getByRole("button", { name: /^Decline:/ })).toHaveCount(0);

  // The consequence of each is on the card, not hidden behind a hover — skip
  // and cancel are opposite outcomes one click apart.
  await expect(page.getByText("Moves past this step.", { exact: false })).toBeVisible();
  await expect(page.getByText("Stops the run here.", { exact: false })).toBeVisible();
});

test("an ordinary approval still decides with Decline and Approve", async ({ page }) => {
  await openApprovals(page, [parkedGatedCall()]);
  const footer = decideFooter(page);
  await expect(footer.getByRole("button", { name: /^Approve:/ })).toBeVisible();
  await expect(footer.getByRole("button", { name: /^Decline:/ })).toBeVisible();
  await expect(footer.getByRole("button", { name: /^Skip this step/ })).toHaveCount(0);
});

test("Skip sends a skip, not the approve it rides on", async ({ page }) => {
  const bodies = await captureResolve(page);
  await openApprovals(page, [parkedBlocker()]);

  await decideFooter(page).getByRole("button", { name: /^Skip this step/ }).click();
  await expect.poll(() => bodies.length).toBe(1);
  expect(bodies[0]).toMatchObject({ verdict: "approve", blocker_verdict: "skip" });
  // Nothing rides along that the host would refuse.
  expect(bodies[0]).not.toHaveProperty("blocker_answer");
  expect(bodies[0]).not.toHaveProperty("scope");
});

test("Cancel run sends a cancel on the deny it rides on", async ({ page }) => {
  const bodies = await captureResolve(page);
  await openApprovals(page, [parkedBlocker()]);

  await decideFooter(page).getByRole("button", { name: /^Cancel run/ }).click();
  await expect.poll(() => bodies.length).toBe(1);
  expect(bodies[0]).toMatchObject({ verdict: "deny", blocker_verdict: "cancel" });
});

test("Retry sends a retry", async ({ page }) => {
  const bodies = await captureResolve(page);
  await openApprovals(page, [parkedBlocker()]);

  await decideFooter(page).getByRole("button", { name: /^Retry/ }).click();
  await expect.poll(() => bodies.length).toBe(1);
  expect(bodies[0]).toMatchObject({ verdict: "approve", blocker_verdict: "retry" });
});

test("an answer reaches the host verbatim, and a blank one cannot be sent", async ({
  page,
}) => {
  const bodies = await captureResolve(page);
  await openApprovals(page, [parkedBlocker()]);
  const footer = decideFooter(page);

  await footer.getByRole("button", { name: /^Answer this question/ }).click();
  const send = footer.getByRole("button", { name: /^Send this answer/ });
  // The host refuses a blank amend rather than downgrading it to a retry — a
  // retry would re-run the step that stopped for want of these words — so the
  // control must refuse it first.
  await expect(send).toBeDisabled();
  await page.getByRole("textbox", { name: /^Answer:/ }).fill("   ");
  await expect(send).toBeDisabled();

  await page.getByRole("textbox", { name: /^Answer:/ }).fill("use gpt-4o-mini instead");
  await expect(send).toBeEnabled();
  await send.click();
  await expect.poll(() => bodies.length).toBe(1);
  expect(bodies[0]).toMatchObject({
    verdict: "approve",
    blocker_verdict: "amend",
    blocker_answer: "use gpt-4o-mini instead",
  });
});

test("a bulk decision never sweeps a question in with the gated calls", async ({
  page,
}) => {
  const bodies = await captureResolve(page);
  await openApprovals(page, [parkedGatedCall(), parkedBlocker()]);

  // One gated call and one blocker: the bulk control speaks for the gated call
  // alone, so it is not offered over a queue of one.
  await expect(page.getByRole("button", { name: "Approve all" })).toHaveCount(0);
  expect(bodies).toHaveLength(0);
});
