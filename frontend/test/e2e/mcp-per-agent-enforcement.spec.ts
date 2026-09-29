import { randomUUID } from "node:crypto";

import { expect, test } from "@playwright/test";

import { LIVE_BRAIN, LIVE_BRAIN_REASON, MCP_SERVER } from "./capabilities";

/**
 * A per-teammate block reaches the agent, on its next turn, with no restart.
 *
 * This is the spec the whole staleness fix exists for. Before it, a tool set to
 * Block kept being callable: the deny list is baked into the roster attachment
 * at build time, and the staleness fingerprint did not hash the policy
 * document, so writing a permission invalidated nothing. It took a host restart
 * to apply — which the docs already claimed was unnecessary.
 *
 * Unit tests pin the fingerprint moving and the attachment denying. Only this
 * one pins the thing an operator actually cares about: the agent stops being
 * able to call it, without anybody bouncing the host.
 *
 * Needs the `openhuman` and `mcp` features plus an inference backend, so it runs
 * in the live-brain lane. The server it drives is registered here against that
 * lane's local `mcp-server.mjs`, not the manifest's `deepwiki`: the rows are the
 * probed inventory, and reading them off the public network would make this
 * spec's verdict depend on it.
 *
 * Serial, and deliberately: each test leaves a permission written that the next
 * one reads or clears, which is the same order an operator moves through.
 */

type Page = import("@playwright/test").Page;

test.skip(
  !MCP_SERVER,
  "needs PW_MCP_SERVER pointing at an HTTP MCP server. The `Console E2E " +
    "(live brain)` CI lane starts one.",
);
test.skip(!LIVE_BRAIN, LIVE_BRAIN_REASON);

test.describe.configure({ mode: "serial" });

/** The refusal `blocked_refusal` writes, matched on its stable opening. */
const REFUSAL = /is blocked by this company's tool permissions/;

/** The tool `mcp-server.mjs` answers by quoting its `text` argument back. */
const TOOL = "echo";

/**
 * The teammate the rule is written for, and the desk whose only member it is.
 *
 * A desk with one member is the only addressable way to say *which* teammate
 * takes the next turn — the company-wide line is the orchestrator's — so the
 * "for that teammate and nobody else" half is two desks rather than two hopes.
 */
const BLOCKED = { id: "engineer", name: "Engineer", desk: "engineering" };
const UNTOUCHED = { desk: "content" };

/** Registered per run: the host keeps runtime servers in its secret store. */
const SERVER = `pw-perm-${randomUUID().slice(0, 8)}`;

test.beforeAll(async ({ request }) => {
  if (!MCP_SERVER || !LIVE_BRAIN) return;
  // The add probes, so the tool inventory the rows are drawn from is stored
  // before the first test looks for a row.
  const added = await request.post("/api/v1/company/mcp/servers", {
    data: { name: SERVER, endpoint: MCP_SERVER, description: "e2e fixture" },
  });
  expect(
    added.ok(),
    `registering ${SERVER} failed: ${added.status()} ${await added.text()}`,
  ).toBeTruthy();
});

test.afterAll(async ({ request }) => {
  if (!MCP_SERVER || !LIVE_BRAIN) return;
  // Best effort, and it must not throw past a failing body: an exception in
  // teardown would replace the real failure with its own.
  await request
    .delete(`/api/v1/company/mcp/servers/${encodeURIComponent(SERVER)}`)
    .catch(() => undefined);
});

/**
 * Opens this server's permissions panel, with `showing` as the lens.
 *
 * The lens comes from the address rather than from the picker: which teammate is
 * scoped is this spec's premise, not its subject, and a click through a select
 * would make it fail for the picker's reasons.
 */
async function openPermissions(page: Page, showing?: string) {
  const lens = showing === undefined ? "" : `&showing=${showing}`;
  await page.goto(`/#/connections/mcp?server=${SERVER}${lens}`);
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already seen in this context */
    });
  await expect(page.getByTestId("mcp-tool-permissions")).toBeVisible({
    timeout: 30_000,
  });
}

/** The row for one tool, addressed by the name it carries. */
function toolRow(page: Page, tool: string) {
  return page.locator(
    `[data-testid="mcp-permission-row"][data-tool="${tool}"]`,
  );
}

/**
 * Makes the agent on `desk` call `TOOL` once, and returns the marker its result
 * would carry.
 *
 * The POST is awaited explicitly before the reload: a turn runs inside the
 * request that started it and the host drops the work when the client goes away,
 * so reloading while the send is in flight cancels the turn before it reaches
 * the model. The marker is unique per call because the mock backend serves one
 * `__MOCK_TOOL_CALL__` per distinct payload — two turns asking for the same
 * arguments would leave the second one with a plain text reply.
 */
async function callTool(page: Page, desk: string): Promise<string> {
  const marker = `perm-${randomUUID()}`;
  await page.goto(`/#/chat/${desk}`);
  const composer = page.getByPlaceholder(/^Message /);
  await expect(composer).toBeVisible({ timeout: 30_000 });

  const posted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/chat") && response.request().method() === "POST",
    { timeout: 120_000 },
  );
  await composer.fill(
    `__MOCK_TOOL_CALL__ ${JSON.stringify({
      name: "mcp_call_tool",
      arguments: {
        server: SERVER,
        tool: TOOL,
        arguments: { text: marker },
      },
    })}`,
  );
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByText(/^Couldn't send/)).toHaveCount(0);
  expect((await posted).ok(), "the chat POST did not succeed").toBeTruthy();

  // Read from a reloaded transcript, so what is asserted is the durable record
  // of the turn rather than whatever the open view chose to draw.
  await page.reload();
  await page.goto(`/#/chat/${desk}`);
  await expect(page.getByPlaceholder(/^Message /)).toBeVisible({
    timeout: 30_000,
  });
  return marker;
}

/** Every bubble in the open transcript. */
function bubbles(page: Page) {
  return page.locator("article[data-message-id]");
}

test("a block set for one teammate refuses that teammate's next turn", async ({
  page,
}) => {
  await openPermissions(page, BLOCKED.id);

  const row = toolRow(page, TOOL);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.getByTestId("mcp-mode-blocked").click();
  await expect(row.getByTestId("mcp-permission-source")).toContainText(
    new RegExp(`et for ${BLOCKED.name}`),
  );

  // No restart and no rebuild: the next turn is the whole contract.
  const marker = await callTool(page, BLOCKED.desk);

  await expect(bubbles(page).filter({ hasText: REFUSAL }).last()).toBeVisible({
    timeout: 120_000,
  });
  // And the call did not also go through — a refusal beside a result would mean
  // the deny list was consulted somewhere that does not gate the dispatch.
  await expect(bubbles(page).filter({ hasText: `echo: ${marker}` })).toHaveCount(
    0,
  );
});

test("the same tool stays callable for a teammate the rule does not name", async ({
  page,
}) => {
  // The other half of the promise, and the reason a per-teammate layer exists at
  // all: narrowing one teammate must not narrow the company.
  const marker = await callTool(page, UNTOUCHED.desk);

  await expect(
    bubbles(page).filter({ hasText: `echo: ${marker}` }).last(),
  ).toBeVisible({ timeout: 120_000 });
  await expect(bubbles(page).filter({ hasText: REFUSAL })).toHaveCount(0);
});

test("clearing the teammate's rule restores the call, again with no restart", async ({
  page,
}) => {
  await openPermissions(page, BLOCKED.id);

  const row = toolRow(page, TOOL);
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.getByTestId("mcp-permission-clear-row").click();
  await expect(row.getByTestId("mcp-permission-source")).not.toContainText(
    new RegExp(`et for ${BLOCKED.name}`),
  );

  const marker = await callTool(page, BLOCKED.desk);

  await expect(
    bubbles(page).filter({ hasText: `echo: ${marker}` }).last(),
  ).toBeVisible({ timeout: 120_000 });
  await expect(bubbles(page).filter({ hasText: REFUSAL })).toHaveCount(0);
});

test("a teammate refused every tool is told so, not left guessing", async ({
  page,
}) => {
  await openPermissions(page, BLOCKED.id);
  await expect(toolRow(page, TOOL)).toBeVisible({ timeout: 30_000 });

  const rows = page.getByTestId("mcp-permission-row");
  const count = await rows.count();
  expect(count, "the panel listed no rows to refuse").toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    await rows.nth(i).getByTestId("mcp-mode-blocked").click();
  }

  // The state that cannot arise company-wide without a deliberate all-tools
  // block, and is the expected shape of a per-teammate narrowing. An agent in it
  // sees an empty tool list and cannot tell it from a server being down.
  await expect(page.getByTestId("mcp-permissions-fully-refused")).toBeVisible();

  await page.getByTestId("mcp-permissions-clear-agent").click();
  await expect(page.getByTestId("mcp-permissions-fully-refused")).toHaveCount(0);
});
