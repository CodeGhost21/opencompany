// `mcp.json` — the company's declared MCP servers as one editable document
// (`.../mcp/config`).
//
// The same store the per-server routes in `api/mcp.ts` write, read and written
// as a whole file. That is the point: the console's MCP page offers a row per
// connection *and* this document, and an operator switching between them must
// not be editing two copies of the configuration that can disagree.
//
// Credentials stay write-only across both directions. A `headers` object is
// accepted on save and stored in the host's secret store; the read never echoes
// one back, reporting only `authConfigured`. An entry that arrives without
// `headers` therefore leaves the stored credential alone rather than clearing
// it — a round-trip cannot silently deauthenticate a server.

import type { OpenCompanyClient } from "./client";
import type { McpServer, McpSource } from "./types";

/** One server as the document renders it. Never carries a credential. */
export interface McpConfigEntry {
  /** Always `http` — the only transport this deployment dials. */
  type?: string;
  url: string;
  description?: string;
  enabled?: boolean;
  allowedTools?: string[];
  disallowedTools?: string[];
  readOnlyTools?: string[];
  timeoutSecs?: number;
  /** Echoed for the reader; ignored on save (provenance is the host's). */
  source?: McpSource;
  /** Whether a credential is stored. Echoed for the reader; ignored on save. */
  authConfigured?: boolean;
  /**
   * The outbound credential, write-only. One header — `Authorization: Bearer …`
   * is stored in the same slot the row's token button writes. Never present on
   * a read.
   */
  headers?: Record<string, string>;
}

/** The document: `{ "mcpServers": { … } }`. */
export interface McpConfigDoc {
  mcpServers: Record<string, McpConfigEntry>;
}

/** A save's answer: the resulting rows, plus the host's pickup note. */
export interface McpConfigWriteResponse {
  servers: McpServer[];
  note: string;
}

/** Read the company's `mcp.json`. */
export function getMcpConfig(
  client: OpenCompanyClient,
  company: string | null,
): Promise<McpConfigDoc> {
  return client.get<McpConfigDoc>(`${client.scopeFor(company)}/mcp/config`);
}

/**
 * Replace the company's declared MCP servers with `doc`.
 *
 * A replace, not a merge: a runtime server absent from the document is removed.
 * A manifest or install-default server absent from it is refused (409) — that
 * declaration lives in `company.toml` or the install config, so the way to
 * silence one is `"enabled": false`, which persists as an override.
 */
export function putMcpConfig(
  client: OpenCompanyClient,
  company: string | null,
  doc: McpConfigDoc,
): Promise<McpConfigWriteResponse> {
  return client.put<McpConfigWriteResponse>(`${client.scopeFor(company)}/mcp/config`, doc);
}
