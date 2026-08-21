// Whether this deployment can actually run the MCP servers its console manages
// (issue #567).
//
// The `/mcp/servers` management routes ship in every build; the agent-side
// registry is pushed onto a teammate's tool belt behind the `mcp` Cargo feature.
// So a build without it accepts a server, stores its token and reports it
// healthy — and no agent ever receives a single tool from it. The console has to
// state that, because every other reading on the screen looks correct: on a
// build with the harness but without the bridge, live tool discovery and health
// probes answer for real.
//
// The three-state answer matters more than it looks. `mcpInBuild` is optional on
// the wire: a host predating the field sends nothing, and rendering that silence
// as "no teammate can use these" would be a fresh lie in the other direction. Only
// an explicit `false` is absence.

import type { CapabilityStatusDto, McpHealth } from "@/api/types";

/**
 * What this build can do with MCP servers.
 *
 * - `present` — the bridge is compiled in; managed servers reach agents.
 * - `absent` — the host said the bridge is missing; servers are configuration
 *   this deployment cannot honour.
 * - `unknown` — the host did not say (an older build, or the capability read
 *   failed). Claim nothing.
 */
export type McpBridgeState = "present" | "absent" | "unknown";

/**
 * Reads the MCP bridge's build state off the capability status.
 *
 * A missing status (the read failed, or the host has no `…/capabilities`
 * surface) and a status without the field are both `unknown` — the console
 * degrades to saying nothing rather than to an accusation it cannot support.
 */
export function mcpBridgeState(status: CapabilityStatusDto | null | undefined): McpBridgeState {
  if (!status || typeof status.mcpInBuild !== "boolean") return "unknown";
  return status.mcpInBuild ? "present" : "absent";
}

/**
 * What to tell the operator after a server is added.
 *
 * The success path is where the old claim did its damage: a banner stating that
 * no agent can use tool servers here is worth nothing if the confirmation
 * toast, fired at the moment the operator acts, still promises that agents pick
 * the server up. On a build with no bridge the add genuinely succeeded — the
 * server is stored, and it survives the rebuild that adds the feature — so this
 * says that, rather than dressing a success up as a failure.
 *
 * `unknown` gets the ordinary message: the host has not said the bridge is
 * missing, and the pickup promise is the host's own (`NEXT_TURN_NOTE`, since
 * issue #566 — the console said "next rebuild" for a while after the runtime
 * stopped needing one).
 */
export function mcpAddedMessage(name: string, bridge: McpBridgeState): string {
  return bridge === "absent"
    ? `Added ${name}. It is stored, but no teammate can call it until this deployment is rebuilt with the MCP bridge.`
    : `Added ${name}. Teammates pick it up on their next turn.`;
}

/**
 * The visual register a server-health badge is entitled to.
 *
 * - `delivering` — the success colour. The probe answered AND the bridge carries
 *   its tools to teammates.
 * - `configured` — the neutral register: stored/reachable, but NOT delivering.
 *   The success colour is withheld here on purpose.
 * - `warn` / `error` — a genuine probe problem, bridge or no bridge.
 */
export type McpBadgeTone = "delivering" | "configured" | "warn" | "error";

export interface McpBadgeDescriptor {
  tone: McpBadgeTone;
  label: string;
}

/**
 * What the per-row MCP health badge should read, folding the bridge state in
 * (issue #1405).
 *
 * The badge used to derive entirely from `health.status`, so a reachable server
 * read a green `ok · N tools` even under the banner saying no teammate can call
 * any of them — reachability rendered as delivery. `HostingView` solved the same
 * shape by folding `inBuild` into its `connected` expression; this does the
 * equivalent for MCP. On `bridge === "absent"` every affirmative reading drops
 * out of the success colour into the neutral `configured` register — the badge
 * then says the same thing as the banner. `needs_config` / `error` are real
 * problems either way and keep their own registers. `unknown` (an older host, or
 * a failed capability read) is treated as delivering: claiming non-delivery
 * there would be the #567 lie in the other direction.
 *
 * Returns `null` when there is nothing to show — never probed and no stored
 * credential, or a probe status the badge does not render.
 */
export function mcpHealthBadge(
  health: McpHealth | undefined,
  authConfigured: boolean,
  bridge: McpBridgeState,
): McpBadgeDescriptor | null {
  const delivers = bridge !== "absent";
  if (!health) {
    if (!authConfigured) return null;
    return delivers
      ? { tone: "delivering", label: "auth set" }
      : { tone: "configured", label: "auth set · not delivered" };
  }
  const tools = `${health.toolCount} tool${health.toolCount === 1 ? "" : "s"}`;
  switch (health.status) {
    case "ok":
      return delivers
        ? { tone: "delivering", label: `ok · ${tools}` }
        : { tone: "configured", label: `reachable · ${tools}, not delivered` };
    case "needs_config":
      return { tone: "warn", label: "needs config" };
    case "error":
      return { tone: "error", label: "error" };
    default:
      return null;
  }
}
