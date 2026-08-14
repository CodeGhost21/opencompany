import { expect, test, type APIRequestContext, type Page } from "@playwright/test";

import { LIVE_BRAIN, LIVE_BRAIN_REASON } from "./capabilities";

/**
 * Issue #863: the canvas paints per-node state WHILE a run walks the graph.
 *
 * Two halves, and the second is the one that was broken. A console watching
 * from the start folds the run out of the `workflow_run_started` /
 * `workflow_node_started` / `workflow_node_finished` frames it saw (#371,
 * #382). A console that joined LATE has none of them — the frame window only
 * holds what arrived after it connected — and used to paint nothing at all for
 * the entire run: not a partial trail, nothing, until the run settled and the
 * operator went looking in the history. That is every cron fire, every run
 * started from another surface, every reload mid-run, and every `EventSource`
 * reconnect.
 *
 * Both cases need a run slow enough to be observed in flight, which is what the
 * mock brain's `__MOCK_SLOW_MS__` directive is for: each agent node carries it
 * in its summary, so every node takes a beat instead of a millisecond.
 */

const COMPANY_SCOPE = "/api/v1/company";

/** Each agent node holds this long, so a run of N of them takes N × this. */
const NODE_MS = 1_200;

test.describe("workflow canvas during a live run", () => {
  // Agent nodes have to actually execute for the host to bracket them with
  // per-node frames, which is the whole subject here.
  test.skip(!LIVE_BRAIN, LIVE_BRAIN_REASON);

  /** Dismisses the first-run tour if it is up; its overlay swallows clicks. */
  async function dismissTour(page: Page) {
    const skip = page.getByRole("button", { name: "Skip for now" });
    try {
      await skip.waitFor({ state: "visible", timeout: 10_000 });
    } catch {
      return;
    }
    await skip.click();
  }

  /** A chain of agent nodes: `start → n0 → … → done`. */
  function chain(id: string, name: string, steps: number) {
    return {
      id,
      name,
      description: "Created by the #863 live-canvas spec.",
      nodes: [
        { id: "start", kind: "trigger", name: "Start" },
        ...Array.from({ length: steps }, (_, i) => ({
          id: `n${i}`,
          kind: "agent",
          name: `Step ${i}`,
          agent: "writer",
          // The directive rides the NODE, not the run request: the request text
          // is not what an agent node prompts with, so a directive placed there
          // never reaches the brain and every node answers in a millisecond.
          summary: `__MOCK_SLOW_MS__ ${NODE_MS} draft a line`,
        })),
        { id: "done", kind: "output", name: "Done" },
      ],
      edges: [
        { from: "start", to: "n0" },
        ...Array.from({ length: steps - 1 }, (_, i) => ({ from: `n${i}`, to: `n${i + 1}` })),
        { from: `n${steps - 1}`, to: "done" },
      ],
    };
  }

  async function createWorkflow(request: APIRequestContext, id: string, steps: number) {
    const res = await request.post(`${COMPANY_SCOPE}/workflows`, {
      data: chain(id, `Live canvas ${id}`, steps),
    });
    expect(res.ok(), `create ${id}: ${res.status()} ${await res.text()}`).toBeTruthy();
  }

  /** The per-node run marks the canvas is painting right now. */
  async function painted(page: Page): Promise<Record<string, string>> {
    return page.evaluate(() =>
      Object.fromEntries(
        Array.from(document.querySelectorAll("[data-run-state]")).map((el) => [
          el.querySelector(".truncate")?.textContent?.trim() ?? "?",
          el.getAttribute("data-run-state") ?? "",
        ]),
      ),
    );
  }

  /** Polls the canvas until `predicate` holds, or gives up with what it saw. */
  async function until(
    page: Page,
    predicate: (marks: Record<string, string>) => boolean,
    timeoutMs = 30_000,
  ): Promise<Record<string, string>> {
    const deadline = Date.now() + timeoutMs;
    let last: Record<string, string> = {};
    while (Date.now() < deadline) {
      last = await painted(page);
      if (predicate(last)) return last;
      await page.waitForTimeout(200);
    }
    return last;
  }

  test("a console watching from the start sees each node light up (#863)", async ({
    page,
    request,
  }) => {
    test.setTimeout(120_000);
    const id = `e2e-863-watching-${Date.now()}`;
    await createWorkflow(request, id, 3);

    try {
      await page.goto(`/#/workflows/${id}`);
      await dismissTour(page);
      await expect(page.getByRole("combobox").first()).toContainText(`Live canvas ${id}`, {
        timeout: 30_000,
      });

      // Start the run from the host side so the click cannot be what paints:
      // the frames are, which is the claim.
      void request.post(`${COMPANY_SCOPE}/workflows/${id}/run`, {
        data: { request: "draft something" },
        timeout: 120_000,
      });

      // Mid-run, not after: a node marked while others are still unmarked is
      // exactly the trail the issue says is missing.
      const marks = await until(page, (m) => Object.values(m).some((s) => s === "running"));
      expect(
        Object.values(marks).some((s) => s === "running"),
        `expected a node to be marked running mid-run; canvas showed ${JSON.stringify(marks)}`,
      ).toBeTruthy();

      const settled = await until(page, (m) => m["Done"] === "ok", 60_000);
      expect(settled["Step 0"], JSON.stringify(settled)).toBe("ok");
    } finally {
      await request.delete(`${COMPANY_SCOPE}/workflows/${id}`).catch(() => undefined);
    }
  });

  test("a console that opens mid-run still paints the trail so far (#863)", async ({
    page,
    request,
  }) => {
    test.setTimeout(120_000);
    const id = `e2e-863-late-${Date.now()}`;
    await createWorkflow(request, id, 4);

    try {
      // The run starts with NO console attached: this is the cron fire, the run
      // launched from chat, the reload, the reconnect.
      const run = request.post(`${COMPANY_SCOPE}/workflows/${id}/run`, {
        data: { request: "draft something" },
        timeout: 120_000,
      });

      // Join once the run is a couple of nodes in.
      await page.waitForTimeout(NODE_MS * 2);
      await page.goto(`/#/workflows/${id}`);
      await dismissTour(page);

      const marks = await until(
        page,
        (m) => Object.values(m).some((s) => s === "ok" || s === "running"),
        30_000,
      );
      expect(
        Object.values(marks).some((s) => s === "ok" || s === "running"),
        `a console joining mid-run painted nothing; canvas showed ${JSON.stringify(marks)}`,
      ).toBeTruthy();

      // And it keeps painting from the frames that arrive after it joined,
      // through to the end of the run.
      const settled = await until(page, (m) => m["Done"] === "ok", 60_000);
      expect(settled["Done"], JSON.stringify(settled)).toBe("ok");
      await run;
    } finally {
      await request.delete(`${COMPANY_SCOPE}/workflows/${id}`).catch(() => undefined);
    }
  });
});
