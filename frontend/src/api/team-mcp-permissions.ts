// What one teammate can actually call on this company's MCP servers.
//
// `GET {scope}/team/{agentId}/mcp/permissions` — one host-resolved read over
// every configured server rather than one read per server. The host resolves
// each row's deciding rule and whether a grant reaches the server at all;
// re-deriving either here would be a second copy of a rule the host owns.
//
// **Declared servers only.** A directory install is addressed by its install id
// and carries its policy under its own key, so its per-teammate rules are read
// through the registry route with `?agent=`.

import type { OpenCompanyClient } from "./client";
import type { ApprovalMode, ToolPolicyRow } from "./mcp-tool-policy";

/** One configured server, as it stands for one teammate. */
export interface AgentServerPermissions {
  server: string;
  /**
   * Whether this teammate's grants reach the server at all. Mode is only
   * consulted after reach, so every row on a server this is `false` for is
   * inert — and a page that hid such a server could not answer the question it
   * exists for.
   */
  reached: boolean;
  /** The grant that would make it reachable, when it is not. */
  grantNeeded?: string;
  enabled: boolean;
  /** Every tool a decision can be stated about. Empty when the document is unreadable. */
  tools: ToolPolicyRow[];
  /** When discovery last succeeded. `0` reads as never. */
  discoveredAtMillis: number;
  /**
   * Whether every tool this server is *known* to offer is refused this
   * teammate. A server no probe has reached is never this: "nothing callable"
   * and "nothing known" are different facts.
   */
  fullyRefused: boolean;
  /**
   * Whether this server's stored document could not be read. Degrades **this
   * block only** — one damaged document must not take the whole page down.
   */
  unreadable: boolean;
}

/** A tool name more than one reached server offers. */
export interface SharedToolName {
  tool: string;
  servers: string[];
}

/** One teammate's whole MCP picture. */
export interface AgentMcpPermissions {
  agent: string;
  /**
   * The grant this teammate asks for, in its three representable states:
   * absent inherits the company's standard grant, `[]` is a deliberate
   * no-tools grant, and a list narrows. Load-bearing, and invisible in a glob
   * field.
   */
  requested?: string[];
  /** The grants this teammate is actually built with. */
  effectiveGrants: string[];
  servers: AgentServerPermissions[];
  /**
   * Tool names reachable on more than one server this teammate holds. Matching
   * is on the **name**, not the capability, so this flags a hazard rather than
   * claiming two servers offer the same thing.
   */
  sharedToolNames: SharedToolName[];
  /**
   * Whether approval parking is live on this host. `false` means a
   * `needs_approval` mode behaves as allow, and every notice saying so is a
   * function of this flag rather than a constant — so it disappears on its own
   * when approvals return.
   */
  approvalsPark: boolean;
}

/** What a tool does, once the mode has been resolved for a teammate. */
export const EFFECT_WORDS: Record<ApprovalMode, string> = {
  always_allow: "runs",
  needs_approval: "asks",
  blocked: "refused",
};

/** Read one teammate's whole MCP picture. */
export function readAgentMcpPermissions(
  client: OpenCompanyClient,
  company: string | null,
  agentId: string,
): Promise<AgentMcpPermissions> {
  return client.get<AgentMcpPermissions>(
    `${client.scopeFor(company)}/team/${encodeURIComponent(agentId)}/mcp/permissions`,
  );
}
