import { expect, test, type Page } from "@playwright/test";

/**
 * End-to-end proof for issue #1141 — the Company page leads with the teammates.
 *
 * # The failure this reproduces
 *
 * Everything here already existed and none of it was reachable. `#/team`
 * rendered the teammate card grid and `#/team/<agentId>` opened a full detail
 * sub-page, but `team` was routable *without a nav entry* — so the only way to
 * either was to type a URL nobody knew. The one nav entry that leads here,
 * Company, opened the org chart: the desks, not the people.
 *
 * On the parent commit the first test fails because the Company nav lands on a
 * tree with no teammate card on it, and the rest because there is no toggle, no
 * status or workload on a card, and no breadcrumb on the detail page.
 *
 * # Why this mocks the operator API
 *
 * Like `org-tree.spec.ts` beside it. The interesting input is a *board*: cards
 * spread across the host's columns, one of them assigned to a desk rather than
 * to a person, which is what proves the count is the teammate's own work and
 * not their desk's. No default harness company produces that on demand.
 *
 * No `LIVE_BRAIN` gate: nothing here needs inference.
 */

const COMPANY = "acme";

const ROSTER = [
  {
    id: "maya",
    name: "Maya",
    role: "Research Lead",
    description: "Tracks competitor moves and drafts the weekly brief.",
  },
  { id: "ravi", name: "Ravi", role: "Analyst", description: "Digs through the numbers." },
  { id: "priya", name: "Priya", role: "Writer", description: "Turns findings into words." },
];

/**
 * The board, as the host's fixed column table declares it (`src/ledger/board.rs`).
 * `closed` is what makes a card open or finished, and it comes from here rather
 * than from any console-side list.
 */
const STATUSES = [
  { name: "todo", label: "To-do", closed: false },
  { name: "planning", label: "Planning", closed: false },
  { name: "in_progress", label: "In progress", closed: false },
  { name: "paused", label: "Paused", closed: false },
  { name: "in_review", label: "In review", closed: false },
  { name: "done", label: "Done", closed: true },
];

/**
 * One card per interesting case:
 *
 * - Maya has an attempt open **and** something queued → working, 2 open.
 * - Ravi has only finished work → idle, 0 open. The `done` card must not count.
 * - Priya has nothing of her own; the in-flight card next to her name is her
 *   *desk's*, and the host deliberately never resolves a desk assignment to a
 *   person. So she reads idle with nothing, not working with one.
 */
const TASKS = [
  { id: "t1", title: "Scan competitor pricing", column: "in_progress", priority: "high", assignee: "maya", updatedAt: 0 },
  { id: "t2", title: "Draft the weekly brief", column: "todo", priority: "medium", assignee: "maya", updatedAt: 0 },
  { id: "t3", title: "Q3 cohort numbers", column: "done", priority: "medium", assignee: "ravi", updatedAt: 0 },
  { id: "t4", title: "Rewrite the landing copy", column: "in_progress", priority: "medium", assignee: "research", updatedAt: 0 },
];

async function mockApi(page: Page) {
  // The first-run product tour renders a modal over everything and swallows
  // clicks beneath it. Answer "already skipped" for any company id.
  await page.addInitScript(() => {
    const real = Storage.prototype.getItem;
    Storage.prototype.getItem = function getItem(key: string) {
      return key.startsWith("oc-tour:") ? '{"skipped":true}' : real.call(this, key);
    };
  });

  await page.route("**/api/v1/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    const json = (body: unknown, status = 200) =>
      route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
    const status = { id: COMPANY, name: "Acme", lifecycle: "running", pending_approvals: 0 };

    if (path === "/api/v1/companies") return json([status]);
    if (path === `/api/v1/companies/${COMPANY}`) return json(status);
    if (path.endsWith("/desks"))
      return json([{ id: "research", name: "Research", members: ["maya", "priya"] }]);
    if (path.endsWith("/tasks")) return json(TASKS);
    if (path.endsWith("/ledgers"))
      return json({
        ledgers: [
          {
            slug: "tasks",
            title: "Tasks",
            purpose: "The board",
            source: "builtin",
            derived: "derived/TASKS.md",
            writtenBy: "the board",
            builtin: true,
            fields: [],
            statuses: STATUSES,
            sections: [],
            open: 3,
            closed: 1,
          },
        ],
      });
    if (path.endsWith("/team")) return json(ROSTER);
    const agent = path.match(/\/team\/([^/]+)$/);
    if (agent) {
      const found = ROSTER.find((m) => m.id === agent[1]);
      if (!found) return json({ error: "no such teammate" }, 404);
      return json({
        ...found,
        source: "overlay",
        // An overlay teammate is editable, which is what makes the header's
        // Edit a live control rather than a disabled explanation.
        editable: ["name", "role", "description"],
        isOrchestrator: false,
        tools: { requested: [], companyAllow: ["web_search"], effective: ["web_search"] },
        desks: [{ id: "research", name: "Research", lead: true }],
        inboxEnabled: false,
      });
    }
    if (path.endsWith("/me")) return json({ id: "op", email: "op@example.com", role: "admin" });
    if (path.endsWith("/events"))
      return route.fulfill({ status: 200, contentType: "text/event-stream", body: "" });
    return json([]);
  });
}

const card = (page: Page, name: string) =>
  page.getByTestId("team-card").filter({ hasText: name }).first();

test("#1141 the Company nav lands on the teammates, not on the desks", async ({ page }) => {
  await mockApi(page);

  // Start elsewhere, so arriving is a real navigation rather than the page the
  // document happened to load on.
  await page.goto("/#/overview");
  const nav = page
    .getByRole("link", { name: "Company", exact: true })
    .or(page.getByRole("button", { name: "Company", exact: true }))
    .first();
  await expect(nav).toBeVisible({ timeout: 30_000 });
  await nav.click();

  // The cards, which no operator could reach before this: `#/team` rendered
  // them from a route with no nav entry.
  await expect(card(page, "Maya")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.url()).toContain("#/company");
});

test("#1141 a card carries the description, the status and the open count", async ({ page }) => {
  await mockApi(page);
  await page.goto("/#/company");

  const maya = card(page, "Maya");
  await expect(maya).toBeVisible({ timeout: 30_000 });

  // What the teammate is for. It was on the roster read all along.
  await expect(maya.getByTestId("team-card-description")).toContainText(
    "Tracks competitor moves",
  );

  // What they are on, and how much. Both derived from the board — an attempt
  // open makes Maya working, and the queued card counts beside it.
  await expect(maya.getByTestId("team-card-status")).toHaveText("Working");
  await expect(maya.getByTestId("team-card-tasks")).toHaveText("2 open tasks");

  // Ravi's only card is finished, and a closed column is not open work.
  const ravi = card(page, "Ravi");
  await expect(ravi.getByTestId("team-card-status")).toHaveText("Idle");
  await expect(ravi.getByTestId("team-card-tasks")).toHaveText("0 open tasks");

  // Priya sits on the desk the in-flight card is assigned to, and that card is
  // the *desk's*. The host refuses to resolve a desk assignment to a person
  // (`AssigneeResolution::links_working_agent`), and neither does this — so she
  // is idle with nothing rather than credited with somebody else's work.
  const priya = card(page, "Priya");
  await expect(priya.getByTestId("team-card-status")).toHaveText("Idle");
  await expect(priya.getByTestId("team-card-tasks")).toHaveText("0 open tasks");
});

test("#1141 the org chart is one toggle away, and the choice is remembered", async ({ page }) => {
  await mockApi(page);
  await page.goto("/#/company");
  await expect(card(page, "Maya")).toBeVisible({ timeout: 30_000 });

  await page.getByTestId("company-mode-chart").click();
  const chart = page.getByRole("tree", { name: "Company org chart" });
  await expect(chart).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("company-mode-chart")).toHaveAttribute("aria-pressed", "true");

  // The chart is where desk creation and membership live (issue #311), so it
  // has to survive a reload rather than being a mode the page forgets.
  await page.reload();
  await expect(chart).toBeVisible({ timeout: 30_000 });

  // And back, which is the half this page leads with.
  await page.getByTestId("company-mode-cards").click();
  await expect(card(page, "Maya")).toBeVisible();
});

test("#1141 asking for Cards from a desk address leaves the desk", async ({ page }) => {
  await mockApi(page);

  // `#/company/<deskId>` forces the chart — a desk address is an org-chart
  // address (issue #485).
  await page.goto("/#/company/research");
  await expect(page.getByRole("tree", { name: "Company org chart" })).toBeVisible({
    timeout: 30_000,
  });

  // So Cards has to clear the desk. Otherwise it is a control an operator can
  // see and press while the route silently outranks it, and the only thing that
  // changes is a preference they cannot observe.
  await page.getByTestId("company-mode-cards").click();
  await expect(card(page, "Maya")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.url()).not.toContain("research");
});

test("#1141 a host with no board says nothing, rather than idle", async ({ page }) => {
  await mockApi(page);
  // Routed *after* `mockApi`, so this handler wins: a ledger list with no board
  // in it. `fetchBoardColumns` resolves empty for that rather than rejecting,
  // which is the failure most likely to be read as "everybody is free".
  await page.route("**/api/v1/**/ledgers", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ledgers: [] }),
    }),
  );

  await page.goto("/#/company");
  const maya = card(page, "Maya");
  await expect(maya).toBeVisible({ timeout: 30_000 });
  // The teammate still renders in full; only the claim this console cannot
  // support is missing.
  await expect(maya.getByTestId("team-card-description")).toBeVisible();
  await expect(maya.getByTestId("team-card-status")).toHaveCount(0);
  await expect(maya.getByTestId("team-card-tasks")).toHaveCount(0);
});

test("#1141 bare #/team is the Company page now", async ({ page }) => {
  await mockApi(page);

  // The address that used to render this grid from nowhere. One grid, one
  // address: it redirects rather than answering in parallel.
  await page.goto("/#/team");
  await expect(card(page, "Maya")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.url()).toContain("#/company");
  await expect.poll(() => page.url()).not.toContain("#/team");
});

test("#1141 a card opens a teammate, breadcrumbed and editable", async ({ page }) => {
  await mockApi(page);
  await page.goto("/#/company");
  await card(page, "Maya").getByTestId("team-card-open").click();

  // Still a linkable page rather than a modal (issue #264).
  await expect.poll(() => page.url()).toContain("#/team/maya");
  await expect(page.getByTestId("agent-name")).toHaveText("Maya", { timeout: 30_000 });

  // The breadcrumb says where the operator *is* — this page is linked into
  // from the org chart and the chat pane, so "Back to team" named a page half
  // its arrivals had never seen.
  const crumb = page.getByTestId("agent-breadcrumb");
  await expect(crumb).toContainText("Company");
  await expect(crumb).toContainText("Maya");

  // The same two facts the card showed, on the page they belong to.
  await expect(page.getByTestId("agent-status")).toHaveText("Working");
  await expect(page.getByTestId("agent-tasks")).toHaveText("2 open tasks");

  // Edit is on the header row, not buried in a card halfway down, and this
  // teammate is an overlay so it is live.
  const edit = page.getByTestId("agent-edit");
  await expect(edit).toBeEnabled();
  await edit.click();
  await expect(page.getByTestId("agent-save")).toBeVisible();

  // And the crumb goes back to the page it names.
  await page.getByTestId("agent-breadcrumb-company").click();
  await expect(card(page, "Maya")).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.url()).toContain("#/company");
});
