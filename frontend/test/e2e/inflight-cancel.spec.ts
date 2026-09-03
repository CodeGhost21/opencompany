import { expect, test } from "@playwright/test";

import { openChannel } from "./chat-helpers";

/**
 * Cancelling an in-flight run that has no card, from Room.
 *
 * A `kind: "delegation"` run carries `taskId: null`. The host registers it so
 * an operator can stop it, and `POST …/tasks/{key}/steer` accepts `cancel` for
 * it, but every console control over a run used to be reached through a card.
 *
 * The in-flight read is intercepted rather than driven through a real
 * delegation: a delegated turn against the scripted brain settles in seconds,
 * so a spec racing that window would assert on whether it lost the race. What
 * is under test here is the console's side — that a taskless row is offered a
 * control at all, and that the control addresses the run by its `key`.
 */

const ENGINEERING = "engineering";

/** Same tour suppression as `agent-profile-panel.spec.ts` — seeded before boot. */
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const seen = JSON.stringify({ skipped: true, seenAt: Date.now() });
    for (const key of ["oc-tour:single", "oc-tour:e2e-harness-co", "oc-tour:null"]) {
      window.localStorage.setItem(key, seen);
    }
  });
});

/** The cancel control for a run, by the accessible name the row gives it. */
function cancelFor(page: import("@playwright/test").Page, title: string) {
  return page.locator(`[data-testid="inflight-run-bar"] button[aria-label="Cancel ${title}"]`);
}

/** A delegation as `GET …/tasks/inflight` returns one: running, and cardless. */
const DELEGATION = {
  taskId: null,
  key: "01a0682eba78-000000000039",
  kind: "delegation",
  title: "content",
  agentId: "writer",
  startedAt: 1_788_454_287_992,
  pendingAction: null,
};

/**
 * The other producer's shape. Its key is `approval:<id>`, so it also pins that
 * a colon survives the round trip into the request path.
 */
const REISSUE = {
  ...DELEGATION,
  key: "approval:01a0682fc5ae-000000000040",
  title: "re-issue request_approval",
  agentId: "ceo",
};

/** Serve `runs` from the in-flight read, and hand back the steers it receives. */
async function withInflight(
  page: import("@playwright/test").Page,
  runs: readonly unknown[],
) {
  const steers: { url: string; body: unknown }[] = [];
  let current = [...runs];

  await page.route("**/tasks/inflight", (route) =>
    route.fulfill({ json: current }),
  );
  await page.route("**/steer", async (route) => {
    steers.push({
      url: route.request().url(),
      body: route.request().postDataJSON(),
    });
    // The run leaves the registry, which is what clears the row on the refetch
    // the console issues after a steer lands.
    current = [];
    await route.fulfill({ status: 202, body: "" });
  });

  page.on("dialog", (dialog) => void dialog.accept());
  return steers;
}

test("a delegation with no card is offered a cancel", async ({ page }) => {
  await withInflight(page, [DELEGATION]);
  await openChannel(page, ENGINEERING);

  await expect(page.getByTestId("inflight-run-bar")).toBeVisible({ timeout: 30_000 });
  await expect(cancelFor(page, "content")).toBeVisible();
});

test("the cancel addresses the run by its key, not by a card", async ({ page }) => {
  const steers = await withInflight(page, [DELEGATION]);
  await openChannel(page, ENGINEERING);

  await cancelFor(page, "content").click();
  await expect.poll(() => steers.length, { timeout: 30_000 }).toBe(1);

  // The whole point of #2058: `taskId` is null and cannot address anything.
  expect(decodeURIComponent(steers[0].url)).toContain(`/tasks/${DELEGATION.key}/steer`);
  expect(steers[0].body).toEqual({ action: "cancel", confirm: true });
});

test("a key carrying a colon reaches the host intact", async ({ page }) => {
  const steers = await withInflight(page, [REISSUE]);
  await openChannel(page, ENGINEERING);

  await cancelFor(page, "re-issue request_approval").click();
  await expect.poll(() => steers.length, { timeout: 30_000 }).toBe(1);

  expect(decodeURIComponent(steers[0].url)).toContain(`/tasks/${REISSUE.key}/steer`);
  expect(steers[0].url).toContain("approval%3A");
});

test("the row leaves once the run is no longer in flight", async ({ page }) => {
  await withInflight(page, [DELEGATION]);
  await openChannel(page, ENGINEERING);

  await cancelFor(page, "content").click();
  await expect(page.getByTestId("inflight-run-bar")).toHaveCount(0, { timeout: 30_000 });
});

test("a double click buys only one steer", async ({ page }) => {
  const steers: unknown[] = [];
  let current: readonly unknown[] = [DELEGATION];
  let reads = 0;

  // The re-read that follows the steer is held open, so the window between the
  // POST landing and the fresh row arriving is wide enough for a second press
  // to land in. That window is the whole bug: the host accepts the second
  // cancel and journals a second `TaskSteered` for one operator intent.
  await page.route("**/tasks/inflight", async (route) => {
    reads += 1;
    if (reads > 1) await new Promise((r) => setTimeout(r, 3000));
    await route.fulfill({ json: current });
  });
  await page.route("**/steer", async (route) => {
    steers.push(route.request().postDataJSON());
    current = [];
    await route.fulfill({ status: 202, body: "" });
  });
  page.on("dialog", (dialog) => void dialog.accept());

  await openChannel(page, ENGINEERING);
  const button = cancelFor(page, "content");
  await expect(button).toBeVisible({ timeout: 30_000 });

  await button.dblclick();
  for (let i = 0; i < 3; i++) {
    await button.click({ force: true, timeout: 1_000 }).catch(() => {});
  }

  await page.waitForTimeout(6_000);
  expect(steers).toEqual([{ action: "cancel", confirm: true }]);
});

test("nothing in flight draws no bar", async ({ page }) => {
  await withInflight(page, []);
  await openChannel(page, ENGINEERING);

  await expect(page.getByPlaceholder(/^Message /)).toBeVisible();
  await expect(page.getByTestId("inflight-run-bar")).toHaveCount(0);
});
