// What the provider detail view is allowed to claim (issue #404).
//
// The issue names the failure mode it exists to prevent: "Do not invent a
// number. If per-connection usage is not recorded today, say so on screen …
// rather than rendering a plausible-looking zero." These pin the two places the
// view could quietly do the equivalent — a missing date rendered as a blank
// (which reads as "never"), and one connection system's total read as another's.

import { describe, expect, it } from "vitest";

import type { ProviderCallsDto } from "@/api/types";
import { callsForProvider, connectedOn } from "@/lib/connection-detail";

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
