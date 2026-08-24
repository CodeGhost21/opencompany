import { expect, test } from "@playwright/test";

import { openChannel, reply } from "./chat-helpers";

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
 * Nothing else can draw this reply, and the spec makes that true rather than
 * assuming it. The `202` body never reached the page, so no turn id was learned
 * from the POST — but the shell also arms its turn poll from `listRuns` at
 * mount, and the harness company this suite shares carries open desk work. A
 * run settling inside the eight-second pause takes the poll's terminal
 * `chat/history` re-read with it, and that read folds in whatever the durable
 * transcript holds by then: this turn's own reply, drawn seconds before the
 * connection is cut.
 *
 * That is what the CI failure artifact shows. Beside the early reply, the
 * operator's own message is rendered *twice* — which nothing but a durable fold
 * can produce, because the optimistic bubble's id is only reconciled by the
 * POST response this test destroys — and the working row is still up, so a turn
 * was armed. A flake in the environment, not a defect in the product, and it
 * reported itself as a 60-second wait for `Couldn't send`, the loudest symptom
 * being the one thing that was not wrong.
 *
 * So from the moment the send starts, `chat/history` answers `[]`. The channel
 * is hydrated before that and is never reopened or reloaded; the durable read
 * is barred from the window under test. If the bubble appears, the released
 * frame is the only thing that can have put it there.
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

test("a chat POST killed in flight still shows the reply the host went on to write", async ({
  page,
}) => {
  test.skip(LIVE_BRAIN, "asserts the offline echo brain's `You said: <text>` reply.");
  // The deliberate pause plus a settle window at the end runs past the suite's
  // 60s default, so the budget is stated rather than inherited.
  test.setTimeout(150_000);

  let cuts = 0;
  // Named before the route is registered so the premise reading taken inside it
  // can target this turn's own reply.
  const marker = `cut-${Date.now()}`;

  // The durable transcript, barred from the window under test — see the header.
  // Held only from the send onwards, so the channel still hydrates normally
  // first: what is excluded is a re-read landing *during* the cut, not the
  // hydration the premise depends on.
  let holdHistory = false;
  await page.route("**/chat/history*", async (route) => {
    if (!holdHistory) {
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "[]",
    });
  });

  // What was on screen at the moment of the cut, read inside the handler and
  // asserted after it. An `expect` that throws inside a route handler aborts
  // the handler, so `route.abort` never runs, the POST never fails, and the
  // run dies waiting for an error line that was never going to appear —
  // reporting a premise violation as a timeout somewhere else entirely.
  let repliesAtCut: number | null = null;
  let cutReady!: () => void;
  const cutReadyPromise = new Promise<void>((resolve) => {
    cutReady = resolve;
  });
  await page.route("**/chat", async (route) => {
    if (route.request().method() !== "POST") {
      await route.continue();
      return;
    }
    // Upstream first: the host accepts the turn and starts running it. Only
    // then is the answer thrown away. Holding the browser-facing response
    // keeps the POST in flight until the latch confirms the premise below.
    const response = await route.fetch();
    await new Promise((resolve) => setTimeout(resolve, CUT_AFTER_MS));
    cuts += 1;
    // The premise, recorded rather than assumed: at the moment the connection
    // is cut, nothing has drawn this reply yet — it can only appear later from
    // the released frame, which is exactly what the assertions below pin.
    // `reply` targets this turn's own echo, not the total bubble count, so a
    // line that would render the answer early fails the test instead of this
    // reading hardening nothing.
    repliesAtCut = await reply(page, marker).count();
    cutReady();
    await route.abort("connectionaborted");
    void response;
  });

  await openChannel(page, ENGINEERING.id);

  await page.getByPlaceholder(/^Message /).fill(marker);
  holdHistory = true;
  await page.keyboard.press("Enter");

  // The operator is told the request failed, and that stays true — a reply
  // arriving later does not mean the send worked, and a console that quietly
  // swallowed the error would leave them unable to tell a delivered message
  // from a dropped one.
  await expect(page.getByText(/Couldn't send/).first()).toBeVisible({ timeout: 60_000 });
  await cutReadyPromise;
  expect(cuts, "the chat POST must actually have been cut").toBe(1);
  expect(repliesAtCut, "nothing had drawn this reply when the connection was cut").toBe(0);

  // …and the answer is on screen anyway, drawn from the frame that was held
  // while the POST's fate was unknown and released when it turned out to have
  // died. Before the outcome split this assertion failed: the throw was
  // reported as `onSendEnd`, which discarded the frame, and the reply was gone
  // for good short of a reload. Sixty seconds rather than the suite default:
  // a saturated CI runner can delay the SSE frame well past the echo brain's
  // millisecond answer, and the reply either appears in that window (proving
  // the release) or the test fails all the same — the wait is slack, not grace.
  await expect(reply(page, marker)).toBeVisible({ timeout: 60_000 });
  await expect(reply(page, marker)).toHaveCount(1);

  // Releasing must not be a licence to double-render: nothing else is going to
  // deliver this reply, so a second bubble could only come from the frame being
  // both replayed and rendered live.
  await page.waitForTimeout(5_000);
  await expect(reply(page, marker)).toHaveCount(1);
});
