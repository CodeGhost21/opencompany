// What the connection detail view is allowed to claim (issues #404, #821).
//
// #404 names the failure mode this exists to prevent: "Do not invent a number.
// If per-connection usage is not recorded today, say so on screen … rather than
// rendering a plausible-looking zero." These pin the places the view could
// quietly do the equivalent — a missing date rendered as a blank (which reads as
// "never"), one connection system's total read as another's, and (#821) a
// remote MCP server's two independent facts collapsed into one "connected".

import { describe, expect, it } from "vitest";

import type { McpHealth, McpServer, ProviderCallsDto } from "@/api/types";
import {
  callsForProvider,
  connectedOn,
  mcpProviderSlug,
  mcpStanding,
  probedOn,
} from "@/lib/connection-detail";

describe("connectedOn", () => {
  it("states the date Composio recorded", () => {
    // Formatted in the viewer's locale, so assert the parts rather than a
    // rendering — a CI runner and a dev box do not agree on the order.
    const line = connectedOn("2026-08-09T15:30:00Z");
    expect(line).toContain("connected");
    expect(line).toContain("2026");
  });

  it("says the date is not recorded rather than leaving a blank", () => {
    // The native store keeps `{token, account}` and journals nothing on
    // connect; MCP has no such concept. A blank in this slot reads as "never
    // connected", which is a different — and false — sentence.
    expect(connectedOn(undefined)).toBe("connection date not recorded");
    expect(connectedOn("")).toBe("connection date not recorded");
  });

  it("does not render a host's unparseable string as a date", () => {
    // Forwarded verbatim from Composio, so this is a wire value, not ours.
    // `Invalid Date` on screen is worse than admitting there is no date.
    expect(connectedOn("whenever")).toBe("connection date not recorded");
  });
});

describe("callsForProvider", () => {
  function row(provider: string, calls: number): ProviderCallsDto {
    return { provider, calls };
  }

  it("counts the toolkit's own successful calls", () => {
    expect(callsForProvider([row("gmail", 12), row("slack", 3)], "gmail")).toBe(12);
  });

  it("reads zero for a provider with no calls in the window", () => {
    // A real zero, not a missing one: `composio_execute` meters every
    // successful call, so a provider with no row has had none.
    expect(callsForProvider([row("slack", 3)], "gmail")).toBe(0);
  });

  it("never reads an MCP server's calls as the Composio toolkit's", () => {
    // `mcp:` exists precisely so these two cannot merge (issue #698): a company
    // with a Composio `gmail` and an MCP server its operator also called
    // `gmail` has two connections, and one row's total is not the other's.
    expect(callsForProvider([row("mcp:gmail", 40), row("gmail", 2)], "gmail")).toBe(2);
    expect(callsForProvider([row("mcp:gmail", 40)], "gmail")).toBe(0);
  });

  it("matches the host's normalization rather than its spelling", () => {
    // `by_provider` groups on a trimmed, lowercased slug (`src/metering/oauth.rs`).
    expect(callsForProvider([row("GitHub", 5)], "github")).toBe(5);
  });
});

describe("mcpProviderSlug", () => {
  it("mirrors the key the host records an MCP call under", () => {
    // `mcp_provider` in `src/metering/oauth.rs`: trimmed, lowercased, prefixed.
    expect(mcpProviderSlug("linear")).toBe("mcp:linear");
    expect(mcpProviderSlug("  LINEAR ")).toBe("mcp:linear");
  });

  it("reads the server's own total and not the toolkit's", () => {
    // The whole point of the prefix (#698). Composio `linear` and an MCP server
    // called `linear` are two connections; each panel reads its own row.
    const rows: ProviderCallsDto[] = [
      { provider: "mcp:linear", calls: 31 },
      { provider: "linear", calls: 4 },
    ];
    expect(callsForProvider(rows, mcpProviderSlug("linear")!)).toBe(31);
    expect(callsForProvider(rows, "linear")).toBe(4);
  });

  it("refuses to attribute a server it cannot name", () => {
    // The host would have recorded a bare `unknown` — its shared
    // cannot-attribute bucket, holding every other unnamed call too. Reading
    // that as this server's total is the invented number the panel exists to
    // avoid, so there is no slug to look up and the caller says so instead.
    expect(mcpProviderSlug("")).toBeNull();
    expect(mcpProviderSlug("   ")).toBeNull();
  });
});

describe("mcpStanding", () => {
  function server(over: Partial<McpServer> = {}): McpServer {
    return {
      name: "linear",
      endpoint: "https://mcp.linear.app/mcp",
      source: "runtime",
      enabled: true,
      allowedTools: [],
      disallowedTools: [],
      readOnlyTools: [],
      timeoutSecs: 30,
      authConfigured: true,
      ...over,
    };
  }

  function health(over: Partial<McpHealth> = {}): McpHealth {
    return { status: "ok", message: "", toolCount: 3, checkedAtMillis: 1_760_000_000_000, ...over };
  }

  it("reads a turned-off server as contributing nothing, however well it probes", () => {
    // The two facts that must not collapse into one badge: `enabled` decides
    // whether any agent gets the tools, the probe decides whether the endpoint
    // answered. A healthy disabled server is still handing out nothing.
    const standing = mcpStanding(server({ enabled: false }), health());
    expect(standing.live).toBe(false);
    expect(standing.summary).toContain("turned off");
    expect(standing.probe).toContain("reachable");
  });

  it("says whether there is a credential, without ever having one to say", () => {
    expect(mcpStanding(server({ authConfigured: true }), undefined).summary).toContain(
      "with a stored credential",
    );
    expect(mcpStanding(server({ authConfigured: false }), undefined).summary).toContain(
      "with no credential",
    );
  });

  it("keeps a probe that never ran apart from one that could not tell", () => {
    // No `health` is a server nobody has pressed Test on. `unknown` is a probe
    // that ran and came back inconclusive. Rendering either as the other
    // invents a result — in one direction a probe, in the other an answer.
    expect(mcpStanding(server(), undefined).probe).toBe("this server has not been probed from here");
    expect(mcpStanding(server(), health({ status: "unknown" })).probe).toContain("could not tell");
  });

  it("counts the tools the last probe actually found", () => {
    expect(mcpStanding(server(), health({ toolCount: 1 })).probe).toContain("1 tool on");
    expect(mcpStanding(server(), health({ toolCount: 9 })).probe).toContain("9 tools on");
  });

  it("distinguishes a refused credential from an endpoint that did not answer", () => {
    expect(mcpStanding(server(), health({ status: "needs_config" })).probe).toContain(
      "want of a credential",
    );
    expect(mcpStanding(server(), health({ status: "error" })).probe).toContain("did not reach it");
  });
});

describe("probedOn", () => {
  it("states the probe time the host does record", () => {
    // The counterpart of `connectedOn`: MCP records no connect, but it does
    // record when it last asked. Locale-formatted, so assert the parts.
    const line = probedOn(Date.UTC(2026, 7, 13, 9, 30));
    expect(line).toContain("last probed");
    expect(line).toContain("2026");
  });

  it("has no probe date rather than one at the epoch", () => {
    // The host omits `health` entirely when it has never probed, so a zero here
    // is a malformed wire value — and "last probed 1 Jan 1970" is the same
    // class of plausible-looking lie as a rendered zero.
    expect(probedOn(undefined)).toBeNull();
    expect(probedOn(0)).toBeNull();
    expect(probedOn(Number.NaN)).toBeNull();
  });
});
