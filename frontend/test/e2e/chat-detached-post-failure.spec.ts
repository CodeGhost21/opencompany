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
 * The pause between the two is what puts the frame in the window under test:
 * the turn finishes and its `agent_reply` is pushed while the console still
 * believes its POST is in flight and is therefore still suppressing. Abort with
 * no pause and the suppression is lifted before the frame lands, which renders
 * it by the ordinary path and proves nothing about the outcome split.
 *
 * That pause used to be a fixed eight seconds, chosen because "the offline echo
 * brain answers in milliseconds" — an assumption nothing enforced. When it did
 * not hold, the spec could not tell "the frame was held and released" apart
 * from "the frame did not exist yet", and reported the second as a 60-second
 * wait for the first (issue #1885). It is now an observation:
 * `awaitJournaledReply` blocks until the host has actually journaled this
 * turn's reply, and only then is the connection cut. A slow host now delays
 * this spec instead of failing it, and a host that never replies fails it with
 * that stated plainly rather than as a timeout somewhere downstream.
 *
 * Nothing else can draw this reply, and the spec makes that true rather than
 * assuming it. The `202` body never reached the page, so no turn id was learned
 * from the POST — but the shell also arms its turn poll from `listRuns` at
 * mount, and the harness company this suite shares carries open desk work. A
 * run settling inside that pause takes the poll's terminal
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

/**
 * How long the released `agent_reply` frame is given to traverse the stream and
 * be captured, once the host is KNOWN to have journaled the reply.
 *
 * Issue #1885: this replaces a fixed 8s `CUT_AFTER_MS` that stood in for "the
 * turn has surely finished by now". The distinction is not the number — it is
 * that this waits on one already-sent push rather than on a turn of unknown
 * duration, so a slow host delays the spec instead of failing it.
 */
const FRAME_SETTLE_MS = 1_000;

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
  // The hydration request's own URL and credential, captured before the bar
  // goes up, so the cut below can ask the HOST whether the reply exists yet —
  // see `awaitJournaledReply`. Captured rather than reconstructed: this is the
  // exact thread scope and auth the console itself uses, with no second source
  // of truth for either to drift from.
  let historyProbe: { url: string; auth: string | undefined } | null = null;
  await page.route("**/chat/history*", async (route) => {
    if (!holdHistory) {
      const request = route.request();
      // THIS thread's read, not merely the most recent one. Hydration fans out
      // over every desk the console knows, so capturing unconditionally leaves
      // the probe pointing at whichever finished last — which then never sees
      // this turn at all. The thread rides the `desk` query param
      // (`client.getChatHistory`).
      if (new URL(request.url()).searchParams.get("desk") === ENGINEERING.id) {
        historyProbe = {
          url: request.url(),
          auth: (await request.allHeaders()).authorization,
        };
      }
      await route.continue();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: "[]",
    });
  });

  /**
   * Block until the host has journaled this turn's reply, or throw.
   *
   * This is the premise the cut depends on, made an OBSERVATION rather than an
   * assumption. The test claims a *held* frame is released, which is only true
   * if the reply was emitted while the POST was still unresolved and therefore
   * captured by `PendingSyncPosts`. The previous fixed `CUT_AFTER_MS` wait
   * asserted that by hoping — "the offline echo brain answers in
   * milliseconds" — and nothing enforced it, so the spec could not tell "the
   * frame was held and released" apart from "the frame did not exist yet"
   * (issue #1885).
   *
   * `page.request` is deliberate: an `APIRequestContext` is NOT subject to
   * `page.route`, so this reads the real durable transcript while the page
   * itself stays barred from it. The bar on the page is what keeps the
   * assertions honest; it must not blind the test's own premise check too.
   */
  const awaitJournaledReply = async (): Promise<string | null> => {
    const deadline = Date.now() + 45_000;
    let lastStatus = "no history request was captured during hydration";
    while (Date.now() < deadline) {
      if (historyProbe) {
        const probe: { url: string; auth: string | undefined } = historyProbe;
        const response = await page.request
          .get(probe.url, probe.auth ? { headers: { authorization: probe.auth } } : {})
          .catch(() => null);
        if (response == null) lastStatus = "the history probe request failed";
        else if (!response.ok()) lastStatus = `history probe returned ${response.status()}`;
        else {
          const body = await response.text().catch(() => "");
          // The REPLY, not the marker alone — the operator's own message
          // carries the marker too and is journaled the moment the host
          // accepts the turn, so matching that would cut before any reply
          // existed and defeat the premise this wait exists to establish.
          // Same text `reply()` locates in the DOM.
          if (body.includes(`You said: ${marker}`)) return null;
          lastStatus = "the host has not journaled this turn's reply yet";
        }
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    return lastStatus;
  };

  // What was on screen at the moment of the cut, read inside the handler and
  // asserted after it. An `expect` that throws inside a route handler aborts
  // the handler, so `route.abort` never runs, the POST never fails, and the
  // run dies waiting for an error line that was never going to appear —
  // reporting a premise violation as a timeout somewhere else entirely.
  let repliesAtCut: number | null = null;
  // Why the premise wait gave up, or `null` if it did not — asserted after the
  // cut, never thrown inside the handler. See `awaitJournaledReply`.
  let premiseFailure: string | null = null;
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
    // Wait for the reply to EXIST, not for a duration guessed to outlast it
    // (issue #1885). Then a short settle so the `agent_reply` frame the host
    // just pushed can traverse the stream and be captured while this POST is
    // still unresolved — which is the state the assertions below are about.
    // Bounded and small: it covers one push already known to have been sent,
    // not an unbounded turn of unknown length, which is exactly the difference
    // between this and the fixed wait it replaces.
    // Recorded, never thrown — the same discipline `repliesAtCut` below is
    // written with, and for the same reason stated there: an exception inside
    // a route handler aborts the HANDLER, so `route.abort` never runs, the
    // POST never fails, and the run dies waiting for a `Couldn't send` line
    // that was never going to appear. That reports a premise violation as a
    // timeout somewhere else entirely, which is precisely the misdirection
    // this spec is being fixed to stop making.
    premiseFailure = await awaitJournaledReply();
    await new Promise((resolve) => setTimeout(resolve, FRAME_SETTLE_MS));
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
  // Stated before the reply assertion below, so a host that never produced the
  // reply is reported as exactly that rather than as a 60-second wait for a
  // bubble that was never coming (issue #1885).
  expect(
    premiseFailure,
    `the host never journaled a reply for ${marker} before the cut — a real delivery failure, ` +
      "not a timing artefact: the turn was accepted and the connection was still open",
  ).toBeNull();

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
