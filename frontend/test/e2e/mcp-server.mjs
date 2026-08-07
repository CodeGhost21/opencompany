#!/usr/bin/env node
//
// The simple MCP server `mcp.spec.ts` installs and calls (issue #467).
//
// # Why HTTP and not stdio
//
// `mcp.spec.ts` was written to install a **stdio** server (`node`, plus a path
// to a script) and had never run, so nothing noticed that this host cannot
// accept one: `src/company/mcp.rs`'s `validate_one` rejects any declaration
// carrying a `command` — *"sets a stdio `command`, which is not supported in
// hosted v1 — declare an HTTP `endpoint` instead"* — and the add route builds
// its `McpServer` with `command: None` regardless of what is posted. An MCP
// server a company can reach is an HTTP one, so that is what this is, and
// `PW_MCP_SERVER` now carries its URL rather than a script path.
//
// # The protocol
//
// One route, `POST /mcp`, speaking newline-free JSON-RPC 2.0 with a plain JSON
// response — the shape the vendored transport actually drives, matched to the
// axum fixture in `src/harness/mcp.rs`'s own tests (no SSE, no session header,
// notifications acknowledged with a bare envelope). `initialize`, `tools/list`
// and `tools/call` are implemented; anything else answers a JSON-RPC
// "method not found" rather than hanging.
//
// # The two tools
//
// `echo` is the one the spec drives end to end: the console lists it, and an
// agent calls it through `mcp_call_tool` and must show the result. `describe`
// exists so the count in the health badge (`ok · 2 tools`) is a number the
// server chose rather than the only number it could have produced, which is
// what makes the assertion mean anything.
//
// Usage:
//   node mcp-server.mjs [--bind HOST:PORT]
// Env:
//   MCP_FIXTURE_BIND   same as --bind (default 127.0.0.1:8098)
//
// The bound address is printed to stderr as
// `[mcp fixture] listening on http://HOST:PORT/mcp`.

import { createServer } from "node:http";

/** The MCP revision this fixture claims, matching the host's own test stub. */
const PROTOCOL_VERSION = "2025-11-25";

/** What the server advertises to `tools/list`. */
const TOOLS = [
  {
    name: "echo",
    description: "Returns `echo: ` followed by the `text` argument.",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string" } },
      required: ["text"],
    },
  },
  {
    name: "describe",
    description: "Returns a one-line description of this fixture server.",
    inputSchema: { type: "object", properties: {} },
  },
];

/**
 * Runs one tool call.
 *
 * @param {string} name
 * @param {any} args
 * @returns {{content: {type: string, text: string}[], isError: boolean} | null}
 *   the result, or null when no such tool exists
 */
function callTool(name, args) {
  if (name === "echo") {
    const text = typeof args?.text === "string" ? args.text : "";
    return { content: [{ type: "text", text: `echo: ${text}` }], isError: false };
  }
  if (name === "describe") {
    return {
      content: [{ type: "text", text: "the opencompany e2e MCP fixture" }],
      isError: false,
    };
  }
  return null;
}

/**
 * Answers one JSON-RPC request.
 *
 * @param {any} request
 * @returns {any | null} the response envelope, or null for a notification
 */
function handle(request) {
  const id = request?.id;
  const method = typeof request?.method === "string" ? request.method : "";
  const params = request?.params ?? {};

  // A notification carries no id and expects no result — MCP sends
  // `notifications/initialized` right after the handshake.
  if (id === undefined || id === null) {
    process.stderr.write(`[mcp fixture] notification: ${method}\n`);
    return null;
  }

  const ok = (result) => ({ jsonrpc: "2.0", id, result });
  const fail = (message) => ({
    jsonrpc: "2.0",
    id,
    error: { code: -32601, message },
  });

  switch (method) {
    case "initialize":
      return ok({
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: "simple", version: "0.1.0" },
      });
    case "ping":
      return ok({});
    case "tools/list":
      return ok({ tools: TOOLS });
    case "tools/call": {
      const name = typeof params?.name === "string" ? params.name : "";
      const result = callTool(name, params?.arguments ?? {});
      process.stderr.write(`[mcp fixture] tools/call ${name}\n`);
      return result ? ok(result) : fail(`unknown tool \`${name}\``);
    }
    default:
      return fail(`method \`${method}\` is not implemented in this fixture`);
  }
}

/**
 * Reads a whole request body.
 *
 * @param {import("node:http").IncomingMessage} request
 * @returns {Promise<string>}
 */
function readBody(request) {
  return new Promise((resolve, reject) => {
    /** @type {Buffer[]} */
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    request.on("error", reject);
  });
}

/**
 * @param {import("node:http").ServerResponse} response
 * @param {number} status
 * @param {any} payload
 */
function sendJson(response, status, payload) {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  response.end(body);
}

const server = createServer((request, response) => {
  const path = new URL(request.url ?? "/", "http://localhost").pathname;

  if (path === "/healthz") {
    sendJson(response, 200, { ok: true });
    return;
  }
  if (request.method !== "POST") {
    sendJson(response, 405, { error: `${request.method} is not served here` });
    return;
  }

  void readBody(request)
    .then((raw) => {
      /** @type {any} */
      let body;
      try {
        body = raw ? JSON.parse(raw) : {};
      } catch (error) {
        sendJson(response, 400, {
          jsonrpc: "2.0",
          id: null,
          error: { code: -32700, message: `parse error: ${error}` },
        });
        return;
      }
      const answer = handle(body);
      // A notification still needs a body: the transport parses whatever comes
      // back, and an empty one reads as a broken server.
      sendJson(response, 200, answer ?? { jsonrpc: "2.0" });
    })
    .catch((error) => {
      sendJson(response, 500, {
        jsonrpc: "2.0",
        id: null,
        error: { code: -32603, message: String(error) },
      });
    });
});

const bindArgument = process.argv.indexOf("--bind");
const bind =
  (bindArgument >= 0 ? process.argv[bindArgument + 1] : undefined) ||
  process.env.MCP_FIXTURE_BIND ||
  "127.0.0.1:8098";
const separator = bind.lastIndexOf(":");
const host = separator > 0 ? bind.slice(0, separator) : "127.0.0.1";
const port = Number(separator > 0 ? bind.slice(separator + 1) : bind);

server.on("error", (error) => {
  process.stderr.write(`[mcp fixture] cannot bind ${bind}: ${error}\n`);
  process.exit(1);
});

server.listen(port, host, () => {
  const address = server.address();
  const chosen = typeof address === "object" && address ? address.port : port;
  process.stderr.write(`[mcp fixture] listening on http://${host}:${chosen}/mcp\n`);
});
