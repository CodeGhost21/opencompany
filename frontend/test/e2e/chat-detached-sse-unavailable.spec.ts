import { expect, test } from "@playwright/test";

import { liveReceipt, openChannel, reply, workingRow } from "./chat-helpers";

import { LIVE_BRAIN } from "./capabilities";

/**
 * The proof issue #1000's hosted-delivery investigation demanded before the
 * detached-turn feature could be called done: on a hosted tenant, `/events`
 * never delivers at all.
 *
 * `EventSource` there reaches `readyState 2` without ever firing `onopen`, and
 * a raw `fetch` to the same route never returns response headers — still
 * pending past two minutes (`opencompany-microservice#23`). The cause is the
 * hosting manager's `forward()` awaiting the complete upstream body before it
 * will build a response, and an SSE stream never completes, so nothing about
 * this is a timeout to tune — the stream is not a slow delivery path on a
 * hosted tenant, it is not a delivery path at all.
 *
 * Which means the poll loop in `AppShell` (`GET {scope}/runs/{turnId}`, armed
 * the moment `onSendDetached` sees the `202`) is not a backstop for the stream.
 * It is the *only* way a detached turn's reply reaches a hosted operator. This
 * spec proves that loop alone — with `/events` behaving exactly as it does in
 * production, not merely absent — carries a chat turn from send to a rendered
 * reply and a settled working row.
 *
 * Blocks the route rather than asserting `LIVE_BRAIN`/`ECHO_BRAIN_ONLY`
 * skip-free: the offline echo brain still answers a real accepted turn with a
 * real durable row, which is everything the poll loop reads. It does not need
 * the stream to be *reachable* to prove the stream is not required.
 */

const ENGINEERING = { id: "engineering", channel: "engineering-desk" };

/**
 * Every `[events] …` line `use-events.ts` writes, collected for the assertion
 * that the stream really was dead.
 *
 * Without it this spec proves nothing the moment the route pattern stops
 * matching — a glob that quietly misses would leave the real stream delivering
 * the reply and the test still green, reporting on the opposite of what it
 * claims. `"[events] connected"` is logged from `onopen`, so its absence is
 * direct evidence no stream ever opened.
 */
let eventLog: string[] = [];

test.beforeEach(async ({ page }) => {
  eventLog = [];
  page.on("console", (message) => {
    const text = message.text();
    if (text.startsWith("[events]")) eventLog.push(text);
  });

  // Same tour-skip shim `chat-live-events.spec.ts` uses — the first-run modal
  // swallows every click otherwise.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });

  // The hosted defect, reproduced exactly: not a 404, not a dropped
  // connection — a request that never gets a response. `route.fulfill` and
  // `route.abort` both complete the request, which is not what the manager
  // does; never calling either leaves it pending, same as the two-minute hang
  // the investigation observed. Registered before navigation so the very
  // first `EventSource` this page opens is caught.
  await page.route("**/events", () => {
    /* deliberately never fulfilled, aborted, or continued */
  });
});

test("with /events unreachable, a detached turn's reply still lands and the working row still settles", async ({
  page,
}) => {
  test.skip(LIVE_BRAIN, "asserts the offline echo brain's `You said: <text>` reply.");
  // The poll is on a 4s tick and this waits out several of them, plus a settle
  // window at the end. The suite's 60s default would cut the run off before the
  // assertions it is here to make, so the budget is stated rather than inherited.
  test.setTimeout(150_000);

  await openChannel(page, ENGINEERING.id);

  const marker = `sse-down-${Date.now()}`;
  await page.getByPlaceholder(/^Message /).fill(marker);
  await page.keyboard.press("Enter");

  // The 202 still comes back over the ordinary POST — that path never touched
  // `/events` — so the in-flight indicator arms right away regardless of the
  // stream. On a same-session send that indicator is the live receipt (issue
  // #1934), and issue #2021 is that it must RIDE the turn past its 202 into the
  // detached window rather than being torn down for the bare working row: the
  // receipt keeps the elapsed clock, the picked-up-by name and the 30s stall
  // notice the operator would otherwise lose the instant the turn detached.
  await expect(liveReceipt(page)).toBeVisible({ timeout: 10_000 });
  // And the downgrade the bug was: the bare working/queued row must NOT be what
  // the operator is left with once the turn detaches. Pre-#2021 the receipt was
  // cleared at the 202 and this row took its place; now the receipt supersedes
  // it for the whole same-session detached turn.
  await expect(workingRow(page)).toHaveCount(0);

  // The reply has exactly one possible source now: `GET {scope}/runs/{id}`
  // going terminal and the resulting `chat/history` re-read (`AppShell`'s poll
  // effect, `TURN_POLL_MS` = 4s). Nothing subscribed to `/events` ever fires —
  // the route above holds every request to it open — so if this appears, the
  // poll loop, and only the poll loop, delivered it.
  await expect(reply(page, marker)).toBeVisible({ timeout: 60_000 });
  // Exactly one — the poll's `chat/history` re-read dedupes by the host's own
  // message id, so a slow second poll tick catching the same terminal turn
  // must not double the bubble.
  await expect(reply(page, marker)).toHaveCount(1);

  // And the indicator comes down. A reply that landed but left the receipt
  // ticking is the same failure users see as a turn that never finished — and
  // the receipt is cleared by the poll's own terminal settle (issue #2021),
  // the same transition that drains the open-turn row, so neither lingers.
  await expect(liveReceipt(page)).toHaveCount(0, { timeout: 30_000 });
  await expect(workingRow(page)).toHaveCount(0);

  // Holds a moment past settle: a stray late poll tick re-adding the row, or a
  // duplicate re-hydration, would both be silent otherwise.
  await page.waitForTimeout(5_000);
  await expect(liveReceipt(page)).toHaveCount(0);
  await expect(workingRow(page)).toHaveCount(0);
  await expect(reply(page, marker)).toHaveCount(1);

  // And the premise itself, asserted rather than assumed. `use-events.ts` logs
  // `[events] connected` from `onopen`; if that ever appears here the route
  // above stopped matching and everything above was proved by the stream, which
  // is the one delivery path this spec exists to do without.
  expect(eventLog, "the console listener must have logged something").not.toEqual([]);
  expect(
    eventLog.filter((line) => line.includes("connected")),
    `the stream must never have opened, saw: ${eventLog.join(" | ")}`,
  ).toEqual([]);
});
