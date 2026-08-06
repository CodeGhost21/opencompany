import { expect, test, type Page } from "@playwright/test";

import { LIVE_BRAIN, LIVE_BRAIN_REASON, MCP_SERVER } from "./capabilities";

/**
 * End-to-end proof for the MCP bridge: an operator installs a tool server, the
 * console shows what it can do, and an agent calls it (issues #50, #467).
 *
 * # What this spec used to say, and why none of it could have passed
 *
 * It had never run — it needs a server to install, and nothing supplied one —
 * so it accumulated three separate drifts, each of which alone would have
 * failed it:
 *
 * * it installed a **stdio** server (`node`, plus a script path). This host has
 *   no stdio transport: `validate_one` rejects any declaration carrying a
 *   `command`, and the add route hardcodes `command: None`. The fixture is now
 *   an HTTP server and `PW_MCP_SERVER` carries its URL.
 * * it drove `#/mcp` and `McpServersView`'s install form, which posts
 *   `{transport, command, args, url}` at a route that reads `{name, endpoint}`.
 *   The surface an operator actually uses is Connections, and that one matches
 *   the host.
 * * it asked the agent for `mcp_registry_tool_call` with
 *   `{server_id, tool_name}`. That is upstream OpenHuman's name; the belt this
 *   host builds carries OpenCompany's own `mcp_call_tool` (`src/harness/build.rs`),
 *   whose schema is `{server, tool, arguments}` and which addresses a server by
 *   name.
 *
 * # What is real here
 *
 * The host, the console, the session, the registry, the HTTP transport, and the
 * agent's tool call. Only two things are fixtures: the MCP server itself
 * (`mcp-server.mjs`, two tools, no network) and the inference backend's
 * *choice* of tool — `__MOCK_TOOL_CALL__` makes the model call one named tool,
 * because a spec cannot assert on a model that is free to decline.
 */

// The fixture is the whole subject. Without one there is nothing to install,
// and saying so is more honest than failing on an empty page.
test.skip(
  !MCP_SERVER,
  "needs PW_MCP_SERVER pointing at an HTTP MCP server. The `Console E2E " +
    "(live brain)` CI lane starts one (issue #467).",
);

/** The name the server is installed under, and the name the agent addresses. */
const SERVER = "simple";

/**
 * Opens Connections, dismissing the first-run tour if it is up. It renders as a
 * modal over the console and swallows the first click; whether it appears
 * depends on console-local state, so its absence is not a failure.
 */
async function openConnections(page: Page) {
  await page.goto("/#/settings/connections");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 10_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already seen in this context — nothing to dismiss */
    });
}

/**
 * The installed server's row. Located by its endpoint rather than its name: the
 * endpoint is unique on the page, and `simple` is a word that could plausibly
 * appear in copy elsewhere on it.
 */
function serverRow(page: Page) {
  return page.locator("li").filter({ hasText: MCP_SERVER! }).first();
}

test("an operator installs an HTTP MCP server and the console reports its tools", async ({
  page,
}) => {
  await openConnections(page);

  await page.locator("#mcp-name").fill(SERVER);
  await page.locator("#mcp-endpoint").fill(MCP_SERVER!);
  await page.getByRole("button", { name: "Add", exact: true }).click();

  const row = serverRow(page);
  await expect(row).toBeVisible({ timeout: 30_000 });
  // `runtime`, not `manifest`: this one was added through the console, and the
  // badge is what tells an operator they may remove it again.
  await expect(row.getByText("runtime", { exact: true })).toBeVisible();

  // Probe it explicitly rather than trusting the add-time probe's persisted
  // result: an on-demand Test is the affordance an operator has when a server
  // looks wrong, and it is the one this spec can prove reaches the server.
  await row.getByRole("button", { name: "Test" }).click();
  await expect(row.getByText(/ok · 2 tools/)).toBeVisible({ timeout: 30_000 });

  // Live discovery, not the declared list: these names came off the wire.
  await row.getByRole("button", { name: "Tools" }).click();
  await expect(row.getByText("echo", { exact: true })).toBeVisible({ timeout: 30_000 });
  await expect(row.getByText("describe", { exact: true })).toBeVisible();
});

test("an agent calls a tool on the installed server and shows the result", async ({
  page,
}) => {
  // Only this half needs the harness: the console flow above talks to the
  // host's own routes, which are wired on any `openhuman` build. Calling a tool
  // needs an agent that runs, and an inference backend to run it.
  test.skip(!LIVE_BRAIN, LIVE_BRAIN_REASON);

  await page.goto("/#/conversation");
  const skip = page.getByRole("button", { name: "Skip for now" });
  await skip
    .waitFor({ state: "visible", timeout: 5_000 })
    .then(() => skip.click())
    .catch(() => {
      /* already dismissed */
    });

  const marker = `agent-mcp-${Date.now()}`;
  const directive = `__MOCK_TOOL_CALL__ ${JSON.stringify({
    name: "mcp_call_tool",
    arguments: {
      server: SERVER,
      tool: "echo",
      arguments: { text: marker },
    },
  })}`;
  await page.getByPlaceholder(/^Message /).fill(directive);
  await page.getByRole("button", { name: "Send" }).click();

  // Scoped to a transcript row, not `getByText` over the page: the chat rail
  // renders a one-line preview of each thread's last message, so an unscoped
  // `.last()` resolves to the sidebar and asserts against a truncated copy.
  const reply = page.locator("div.group\\/msg").filter({ hasText: /__MOCK_LLM__/ }).last();
  await expect(reply).toBeVisible({ timeout: 60_000 });
  // Both halves of the round trip: the marker that says the mocked backend
  // answered, and the remote tool's own output, which can only have come from
  // the fixture over HTTP.
  await expect(reply).toContainText(`echo: ${marker}`, { timeout: 30_000 });
  await expect(page.getByText(/^Couldn't send/)).toHaveCount(0);
});
