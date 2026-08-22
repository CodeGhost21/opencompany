import { describe, expect, it } from "vitest";

import { mcpAddedMessage, mcpBridgeState, mcpHealthBadge } from "@/lib/mcp-bridge";
import type { CapabilityStatusDto, McpHealth } from "@/api/types";

/**
 * The MCP tab's build-state signal (issue #567).
 *
 * The management routes are ungated, so an operator on a build without the `mcp`
 * feature adds a server, stores a token, watches it probe healthy — and no agent
 * ever receives a tool from it. What makes this worth a test rather than an
 * inline ternary is the third state: `mcpInBuild` is optional on the wire, and
 * treating a host that never sends it as "the bridge is missing" would replace
 * one false claim with another, on every older host.
 */

function status(over: Partial<CapabilityStatusDto> = {}): CapabilityStatusDto {
  return { configured: false, ...over };
}

describe("mcpBridgeState", () => {
  it("reads an explicit true as the bridge being present", () => {
    expect(mcpBridgeState(status({ mcpInBuild: true }))).toBe("present");
  });

  it("reads an explicit false as the bridge being absent", () => {
    expect(mcpBridgeState(status({ mcpInBuild: false }))).toBe("absent");
  });

  it("says nothing when the host omits the field", () => {
    // An older host that predates the flag. Rendering this as "absent" would
    // tell every such operator their servers are dead when they may be fine.
    expect(mcpBridgeState(status())).toBe("unknown");
  });

  it("says nothing when the capability read failed", () => {
    expect(mcpBridgeState(null)).toBe("unknown");
    expect(mcpBridgeState(undefined)).toBe("unknown");
  });

  it("does not treat a non-boolean as an answer", () => {
    // A host that sends `null` (or any other shape) for the field has not said
    // the bridge is missing — the same silence as omitting it.
    const malformed = { configured: false, mcpInBuild: null } as unknown as CapabilityStatusDto;
    expect(mcpBridgeState(malformed)).toBe("unknown");
  });
});

/**
 * The add-success message, which is where the screen's other claim used to
 * live: "Teammates pick it up on the next rebuild" — fired at the moment the
 * operator acts, and false twice over on a build with no bridge (nothing picks
 * it up, and since #566 a rebuild is not how pickup happens either).
 */
describe("mcpAddedMessage", () => {
  it("confirms the add without promising pickup when the bridge is missing", () => {
    const message = mcpAddedMessage("notion", "absent");
    expect(message).toContain("notion");
    // Still a success: the server IS stored, and survives the rebuild that adds
    // the feature. Saying otherwise would report a working add as a failure.
    expect(message).toMatch(/stored/i);
    expect(message).not.toMatch(/teammates pick it up/i);
  });

  it("promises next-turn pickup when the bridge is present", () => {
    const message = mcpAddedMessage("notion", "present");
    expect(message).toMatch(/next turn/i);
    // #566: pickup no longer needs a restart, and the console said it did long
    // after the host stopped saying so.
    expect(message).not.toMatch(/rebuild|restart/i);
  });

  it("treats an unanswering host as ordinary, not as broken", () => {
    // Unknown is not absence: claiming no agent can use the server, on a host
    // that never said so, is the same class of lie in the other direction.
    expect(mcpAddedMessage("notion", "unknown")).toBe(mcpAddedMessage("notion", "present"));
  });
});

/**
 * The per-row health badge's register (issue #1405).
 *
 * The bug this pins: a reachable server read a green `ok · N tools` even under
 * the banner saying no teammate can call one — reachability rendered as
 * delivery. Folding the bridge state in demotes every affirmative reading to the
 * neutral register when — and only when — the bridge is explicitly absent.
 */
function health(over: Partial<McpHealth> = {}): McpHealth {
  return { status: "ok", message: "", toolCount: 3, checkedAtMillis: 1, ...over };
}

describe("mcpHealthBadge", () => {
  it("reads a healthy server as delivering when the bridge is present", () => {
    const badge = mcpHealthBadge(health({ toolCount: 12 }), true, "present");
    expect(badge).toEqual({ tone: "delivering", label: "ok · 12 tools" });
  });

  it("drops a healthy server out of the success colour when the bridge is absent", () => {
    // The exact #1405 contradiction: green `ok` under the "no teammate receives
    // their tools" banner. Reachable, but not delivering.
    const badge = mcpHealthBadge(health({ toolCount: 12 }), true, "absent");
    expect(badge?.tone).toBe("configured");
    expect(badge?.tone).not.toBe("delivering");
    expect(badge?.label).toMatch(/not delivered/);
    expect(badge?.label).toContain("12 tools");
  });

  it("keeps the success colour on an unknown host — non-delivery must not be invented", () => {
    // Symmetry with the banner: `unknown` has not said the bridge is missing, so
    // demoting the badge there would be the #567 lie in the other direction.
    expect(mcpHealthBadge(health(), true, "unknown")?.tone).toBe("delivering");
  });

  it("singularises a one-tool count in both registers", () => {
    expect(mcpHealthBadge(health({ toolCount: 1 }), true, "present")?.label).toBe("ok · 1 tool");
    expect(mcpHealthBadge(health({ toolCount: 1 }), true, "absent")?.label).toMatch(/1 tool,/);
  });

  it("shows the auth hint when never probed, and demotes it under an absent bridge", () => {
    expect(mcpHealthBadge(undefined, true, "present")).toEqual({
      tone: "delivering",
      label: "auth set",
    });
    expect(mcpHealthBadge(undefined, true, "absent")?.tone).toBe("configured");
    // Nothing at all when there is neither a probe nor a stored credential.
    expect(mcpHealthBadge(undefined, false, "present")).toBeNull();
  });

  it("leaves genuine probe problems in their own register, bridge or no bridge", () => {
    // A missing credential or an unreachable endpoint is a real problem the
    // bridge state does not change — these keep amber / red.
    expect(mcpHealthBadge(health({ status: "needs_config" }), true, "present")?.tone).toBe("warn");
    expect(mcpHealthBadge(health({ status: "needs_config" }), true, "absent")?.tone).toBe("warn");
    expect(mcpHealthBadge(health({ status: "error" }), true, "absent")?.tone).toBe("error");
  });
});
