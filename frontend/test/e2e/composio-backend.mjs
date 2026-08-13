#!/usr/bin/env node
//
// A stand-in for the platform's Composio routes, so the end-to-end suite can
// exercise a company that holds **two accounts for one toolkit** (issue #820).
//
// # Why a fixture and not the real backend
//
// The thing under test is which account an agent acts as, and the only way to
// have that question exist is to hold two connections for one toolkit. Against
// the real backend that would mean running two live Gmail OAuth handshakes per
// run, from CI, against an account nobody owns. It would also make the
// assertion unobservable: the point is the **request body** the host sends —
// whether it carries `connectionId` — and only the party receiving it can say.
//
// So this serves the four routes the console and the harness actually call, and
// records every execute body for the spec to read back:
//
//   GET  /agent-integrations/composio/toolkits      → catalog (gmail, slack)
//   GET  /agent-integrations/composio/connections   → two gmail accounts, one slack
//   POST /agent-integrations/composio/authorize     → a connect URL nobody opens
//   POST /agent-integrations/composio/execute       → records the body, succeeds
//   DELETE /agent-integrations/composio/connections/{id}
//
// Plus two routes of its own, outside the backend's namespace so they cannot be
// mistaken for it:
//
//   GET  /healthz     → readiness, for `playwright.config.ts`'s webServer wait
//   GET  /__executes  → every execute body seen, oldest first
//   POST /__reset     → forget them
//
// # What it deliberately does not do
//
// It does not check the bearer. Tenant isolation is decided by which credential
// the host presents, and that is asserted where it is decidable — the host's own
// `isolation_tests` in `src/harness/composio.rs` drive a recording backend for
// exactly that. A fixture that also enforced auth would fail runs for reasons
// that have nothing to do with the spec driving it.
//
// No dependencies, so it starts as fast as the other fixtures.

import { createServer } from "node:http";

const bindArg = process.argv.indexOf("--bind");
const bind = bindArg > -1 ? process.argv[bindArg + 1] : "127.0.0.1:8097";
const [host, port] = bind.split(":");

/** The connections this company holds. Two Gmail accounts is the whole point. */
const connections = [
  {
    id: "ca_ops",
    toolkit: "gmail",
    status: "ACTIVE",
    createdAt: "2026-08-01T10:00:00Z",
    accountEmail: "ops@acme.test",
  },
  {
    id: "ca_billing",
    toolkit: "gmail",
    status: "ACTIVE",
    createdAt: "2026-08-02T10:00:00Z",
    accountEmail: "billing@acme.test",
  },
  {
    id: "ca_slack",
    toolkit: "slack",
    status: "ACTIVE",
    createdAt: "2026-08-03T10:00:00Z",
    workspace: "Acme Workspace",
  },
];

/** Every `POST …/execute` body this process has seen, oldest first. */
const executes = [];

/** The backend's success envelope. Everything it answers is wrapped in it. */
function ok(res, data) {
  res.writeHead(200, { "content-type": "application/json" });
  res.end(JSON.stringify({ success: true, data }));
}

function readBody(req) {
  return new Promise((resolve) => {
    let raw = "";
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => {
      try {
        resolve(raw ? JSON.parse(raw) : {});
      } catch {
        resolve({ __unparseable: raw });
      }
    });
  });
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  const path = url.pathname;

  if (path === "/healthz") return ok(res, { status: "ok" });

  if (path === "/__executes") {
    res.writeHead(200, { "content-type": "application/json" });
    return res.end(JSON.stringify(executes));
  }

  if (path === "/__reset" && req.method === "POST") {
    executes.length = 0;
    return ok(res, { reset: true });
  }

  if (path === "/agent-integrations/composio/toolkits") {
    return ok(res, {
      toolkits: ["gmail", "slack"],
      catalog: [
        {
          slug: "gmail",
          name: "Gmail",
          enabled: true,
          description: "Send and read email.",
          categories: ["email"],
        },
        {
          slug: "slack",
          name: "Slack",
          enabled: true,
          description: "Post messages to channels.",
          categories: ["communication"],
        },
      ],
    });
  }

  if (path === "/agent-integrations/composio/connections" && req.method === "GET") {
    return ok(res, { connections });
  }

  if (path.startsWith("/agent-integrations/composio/connections/") && req.method === "DELETE") {
    const id = decodeURIComponent(path.split("/").pop());
    const at = connections.findIndex((c) => c.id === id);
    if (at > -1) connections.splice(at, 1);
    return ok(res, { deleted: at > -1 });
  }

  if (path === "/agent-integrations/composio/authorize" && req.method === "POST") {
    return ok(res, { connectUrl: "https://connect.composio.dev/e2e", connectionId: "ca_new" });
  }

  if (path === "/agent-integrations/composio/execute" && req.method === "POST") {
    const body = await readBody(req);
    executes.push(body);
    // The shape the harness parses: a provider result plus the cost. The
    // account it *would* have acted as is echoed back so a failure reads as
    // "actedAs ca_ops, expected ca_billing" rather than as an empty diff.
    return ok(res, {
      data: { actedAs: body.connectionId ?? null, tool: body.tool },
      successful: true,
      error: null,
      costUsd: 0,
    });
  }

  res.writeHead(404, { "content-type": "application/json" });
  res.end(JSON.stringify({ success: false, error: `no fixture route for ${req.method} ${path}` }));
});

server.listen(Number(port), host, () => {
  console.error(`[composio fixture] listening on http://${bind}`);
});
