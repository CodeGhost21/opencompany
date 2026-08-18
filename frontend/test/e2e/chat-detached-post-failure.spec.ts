import { expect, test, type Locator, type Page } from "@playwright/test";

import { LIVE_BRAIN } from "./capabilities";

/**
 * The turn survives the request that started it — proved by killing the
 * request (issue #1000).
 *
 * This is the case the whole detached design exists for, stated as a test. A
 * chat POST used to hold the connection open for a turn of unbounded duration,
 * which is how five real persona tasks in a row came back as gateway 504s with
 * the work running on invisibly behind them. Since #983 the POST answers `202`
 * and the reply arrives over the stream — but the connection can still die,
 * and when it does the console's `fetch` throws exactly as it did before while
 * the host carries on and journals the reply as if nothing happened.
 *
 * The console's send bracket has three outcomes and only one of them means "the
 * reply is on screen". A throw is not that one: nothing was rendered, so the
 * live `agent_reply` frame `PendingSyncPosts` is holding for that thread is the
 * only copy of the answer this browser will ever be handed. Reporting the throw
 * as `onSendEnd` — which is what the code did — discards it, and the operator
 * watches their message fail and never learn that it was answered.
 *
 * ## Why the interception is shaped the way it is
 *
 * `route.fetch()` before the abort is load-bearing: the request really is sent
 * upstream, so the host really does accept and run the turn. What is destroyed
 * is the *response*, which is the actual production failure — a gateway cutting
 * a connection does not un-send the request.
 *
 * The pause between the two is what puts the frame in the window under test.
 * The offline echo brain answers in milliseconds, so during those seconds the
 * turn finishes and its `agent_reply` is pushed while the console still
 * believes its POST is in flight and is therefore still suppressing. Abort with
 * no pause and the suppression is lifted before the frame lands, which renders
 * it by the ordinary path and proves nothing about the outcome split.
 *
 * Nothing else can draw this reply. The `202` body never reached the page, so
 * no turn id was learned and the run poll is never armed; the channel was
 * hydrated before the send and is never reopened or reloaded. If the bubble
 * appears, the released frame is what put it there.
 */

const ENGINEERING = { id: "engineering", channel: "engineering-desk" };

/** How long the response is withheld before the connection is cut. */
const CUT_AFTER_MS = 8_000;

test.beforeEach(async ({ page }) => {
  // Same tour-skip shim the rest of the suite uses — the first-run modal
  // swallows every click otherwise.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });
});

async function openChannel(page: Page, channelId: string) {
  await page.goto(`/#/chat/${channelId}`);
  await expect(page.getByPlaceholder(/^Message /)).toBeVisible({ timeout: 30_000 });
}

function bubbles(page: Page): Locator {
  return page.locator("article[data-message-id]");
}

function reply(page: Page, marker: string): Locator {
  return bubbles(page).filter({ hasText: `You said: ${marker}` });
}

test("a chat POST killed in flight still shows the reply the host went on to write", async ({
  page,
}) => {
  test.skip(LIVE_BRAIN, "asserts the offline echo brain's `You said: <text>` reply.");
  // The deliberate pause plus a settle window at the end runs past the suite's
  // 60s default, so the budget is stated rather than inherited.
  test.setTimeout(150_000);

  let cuts = 0;
  await page.route("**/chat", async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    // Upstream first: the host accepts the turn and starts running it. Only
    // then is the answer thrown away.
    await route.fetch();
    await new Promise((resolve) => setTimeout(resolve, CUT_AFTER_MS));
    cuts += 1;
    await route.abort("connectionaborted");
  });

  await openChannel(page, ENGINEERING.id);

  const marker = `cut-${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(marker);
  await page.keyboard.press("Enter");

  // The operator is told the request failed, and that stays true — a reply
  // arriving later does not mean the send worked, and a console that quietly
  // swallowed the error would leave them unable to tell a delivered message
  // from a dropped one.
  await expect(page.getByText(/Couldn't send/).first()).toBeVisible({ timeout: 60_000 });
  expect(cuts, "the chat POST must actually have been cut").toBe(1);

  // …and the answer is on screen anyway, drawn from the frame that was held
  // while the POST's fate was unknown and released when it turned out to have
  // died. Before the outcome split this assertion failed: the throw was
  // reported as `onSendEnd`, which discarded the frame, and the reply was gone
  // for good short of a reload.
  await expect(reply(page, marker)).toBeVisible({ timeout: 30_000 });
  await expect(reply(page, marker)).toHaveCount(1);

  // Releasing must not be a licence to double-render: nothing else is going to
  // deliver this reply, so a second bubble could only come from the frame being
  // both replayed and rendered live.
  await page.waitForTimeout(5_000);
  await expect(reply(page, marker)).toHaveCount(1);
});
