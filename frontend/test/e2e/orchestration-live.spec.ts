import { expect, test, type APIRequestContext } from "@playwright/test";

import { LIVE_LLM, LIVE_LLM_REASON } from "./capabilities";
import {
  DONE,
  SCOPE,
  column,
  dispatch,
  openBoard,
  openChannel,
  say,
  silenceTour,
} from "./orchestration";

/**
 * **The same loop as `orchestration-simulation.spec.ts`, with a real model
 * making the decisions.**
 *
 * The scripted lane proves the machinery runs: a goal reaches the orchestrator,
 * `spawn_task` opens cards, a dispatch runs a teammate's turn, `review_task`
 * closes them. What it cannot prove is that any of that is *reachable* — that a
 * model, handed this company's roster and the real descriptions of these tools,
 * decides to break a goal up, give the pieces to the right people, and accept
 * the results afterwards. A prompt that stopped describing the board, a tool
 * description that stopped saying what it is for, a roster the orchestrator can
 * no longer see: every one of those leaves the scripted lane green, because the
 * scripted lane never reads them.
 *
 * This lane reads them. Nothing here scripts a choice — `live-brain-proxy.mjs`
 * forwards the turn to a real router and returns whatever comes back.
 *
 * # Why it is not in CI, and what that costs
 *
 * It spends tokens and its verdict is a model's judgement, which is the one
 * thing a required check must not depend on: a model having a bad day would
 * turn every unrelated pull request red. So it is run by a person — before
 * changing an orchestrator prompt, a delegation tool's description, or the
 * drain — with `npm run e2e:live-llm`.
 *
 * The cost is that it can rot silently, which is why it is written to fail
 * *legibly*: every wait names what it was waiting for, and the two failures
 * this lane actually has — "the model was never asked" and "the model was asked
 * and chose nothing" — are separated by the first assertion below.
 *
 * # What is asserted, given that the answers are not ours
 *
 * Outcomes, never wording, and never *which* tool was used. An orchestrator
 * that opens cards with `spawn_task` and one that hands work over with
 * `delegate_to_teammate` (which opens its own card) are both doing the job; a
 * spec that demanded one would be asserting a preference. So what is checked is
 * the shape of the board afterwards: work appeared, somebody on the roster owns
 * it, dispatching it runs a turn, and asking for it to be closed out closes it.
 *
 * Cards are identified by **what was not there before**, not by title: the
 * model writes the titles, and a spec that grepped for its own words would be
 * testing the model's obedience rather than the company's behaviour.
 */

/** The board's own record for every card the company holds. */
async function allCards(
  request: APIRequestContext,
): Promise<{ id: string; title: string; column: string; stage?: string; assignee?: string }[]> {
  const response = await request.get(`${SCOPE}/tasks`);
  expect(response.ok(), `listing cards failed: ${response.status()}`).toBeTruthy();
  const body = await response.json();
  return (body.tasks ?? body) as never[];
}

/** The ids on the board right now — the baseline a goal is measured against. */
async function cardIds(request: APIRequestContext): Promise<Set<string>> {
  return new Set((await allCards(request)).map((task) => task.id));
}

/** The cards that appeared since `before`. */
async function newCards(request: APIRequestContext, before: Set<string>) {
  return (await allCards(request)).filter((task) => !before.has(task.id));
}

/** The roster ids an orchestrator may hand work to, read from the host. */
async function roster(request: APIRequestContext): Promise<string[]> {
  const response = await request.get(`${SCOPE}/team`);
  expect(response.ok(), `reading the team failed: ${response.status()}`).toBeTruthy();
  const body = await response.json();
  const members = (body.members ?? body.team ?? body) as { id: string }[];
  return members.map((member) => member.id);
}

test.beforeEach(async ({ page }) => {
  await silenceTour(page);
});

test("a real model takes a goal, gives it to its team, and closes it out", async ({
  page,
  request,
}) => {
  test.skip(!LIVE_LLM, LIVE_LLM_REASON);
  // Every turn here is a real model round trip, and there are at least five of
  // them: the goal, two dispatched teammate turns, and the close-out. Timed for
  // a slow rung rather than a fast one — a lane that fails on latency teaches
  // nobody anything.
  test.setTimeout(900_000);

  const before = await cardIds(request);
  const team = await roster(request);
  expect(team.length, "a company with no roster has nobody to delegate to").toBeGreaterThan(1);

  // ── 1. The goal, in the operator's own words ────────────────────────────
  // Specific about the *outcome* (cards on the board, owned by teammates) and
  // silent about the mechanism, because which tool to reach for is exactly the
  // decision under test.
  await openChannel(page);
  await say(
    page,
    "I want a short market digest published this week. Break it into two pieces of " +
      "work, open a card on the board for each, and give each one to whichever " +
      "teammate should own it. Do not do the work yourself in this message.",
  );

  // The first failure this lane can have: the model was never reached, or was
  // reached and chose nothing. A board that grew is the proof it decided.
  await expect
    .poll(() => newCards(request, before).then((cards) => cards.length), {
      message:
        "the orchestrator opened no cards for the goal — either the model was not " +
        "reached (check the proxy's stderr for a non-200) or it answered in prose " +
        "without calling a delegation tool",
      timeout: 300_000,
      intervals: [3_000],
    })
    .toBeGreaterThanOrEqual(1);

  // Let the turn finish before reading the board: a fan-out arrives one card at
  // a time, and a run read at the first one would dispatch half a goal.
  await page.waitForTimeout(5_000);
  const opened = await newCards(request, before);
  // The cap is the host's own (`MAX_DELEGATIONS_PER_TURN`), so more than three
  // means something other than this turn wrote to the board.
  expect(opened.length, "more cards than one turn can open").toBeLessThanOrEqual(3);

  // Somebody real owns the work. An unassigned card is a to-do list entry; the
  // claim this lane exists for is that the orchestrator *delegated*.
  const owned = opened.filter((card) => card.assignee && team.includes(card.assignee));
  expect(
    owned.length,
    `no card was given to anyone on the roster (${opened
      .map((card) => `${card.title} → "${card.assignee ?? ""}"`)
      .join("; ")})`,
  ).toBeGreaterThanOrEqual(1);

  // The reply is a model's, not the fixture's. Cheap, and it is the one line
  // that tells a confused reader which lane they are actually running.
  await expect(page.getByText("__MOCK_LLM__")).toHaveCount(0);

  // ── 2. The operator starts the work ────────────────────────────────────
  // Whatever is still unstarted gets dispatched by hand, on the board. A card
  // the orchestrator already ran (a `delegate_to_teammate` hand-off opens one
  // mid-turn) is left alone rather than re-run.
  for (const card of opened) {
    const current = (await allCards(request)).find((held) => held.id === card.id);
    if ((current?.stage ?? current?.column) === "pending") {
      await dispatch(page, card.title);
    }
  }

  // ── 3. …and the team does it ───────────────────────────────────────────
  // Settled, not finished-in-a-particular-way: a real turn can land in review,
  // or park on an approval, and which of those happens is the model's business.
  // What must not happen is a card left running forever.
  for (const card of opened) {
    await expect
      .poll(
        async () => {
          const held = (await allCards(request)).find((row) => row.id === card.id);
          return held?.stage ?? held?.column ?? "in_progress";
        },
        {
          message: `card "${card.title}" never settled`,
          timeout: 420_000,
          intervals: [5_000],
        },
      )
      .not.toBe("in_progress");
  }

  // ── 4. The operator asks for it to be closed out ───────────────────────
  // The ids are named because an operator naming them is the realistic ask —
  // "the two you opened" would be a test of the model's memory of its own turn,
  // which is a different claim from the one this spec makes.
  const settled = (await allCards(request)).filter((held) =>
    opened.some((card) => card.id === held.id),
  );
  const reviewable = settled.filter((card) => (card.stage ?? card.column) === "in_review");
  test.skip(
    reviewable.length === 0,
    `nothing reached review — the run parked instead (${settled
      .map((card) => `${card.title}: ${card.stage ?? card.column}`)
      .join("; ")}). That is a legitimate landing, but the close-out below has ` +
      "nothing to accept.",
  );

  await openChannel(page);
  await say(
    page,
    `The work is back and it looks good to me. Please review and approve ` +
      `${reviewable.length === 1 ? "this card" : "these cards"}, so ` +
      `${reviewable.length === 1 ? "it is" : "they are"} marked done: ` +
      reviewable.map((card) => `${card.id} ("${card.title}")`).join(", "),
  );

  for (const card of reviewable) {
    await expect
      .poll(
        async () => {
          const held = (await allCards(request)).find((row) => row.id === card.id);
          return held?.column ?? "";
        },
        {
          message:
            `card "${card.title}" was never accepted — the orchestrator did not ` +
            "call review_task, or called it with a different id",
          timeout: 300_000,
          intervals: [5_000],
        },
      )
      .toBe("done");
  }

  // ── 5. The board an operator comes back to ─────────────────────────────
  await page.reload();
  await openBoard(page);
  for (const card of reviewable) {
    await expect(column(page, DONE)).toContainText(card.title, { timeout: 60_000 });
  }
});
