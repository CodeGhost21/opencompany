// Reading and checking the `mcp.json` an operator types into the editor
// (`views/mcp/McpJsonEditor.tsx`).
//
// Split out of the component because these are the rules a text editor needs
// and a form does not: a form can only produce shapes the host accepts, while a
// document can say anything, and the difference between "this isn't JSON",
// "this isn't the shape" and "the host refused it" is the difference between an
// operator fixing a typo in three seconds and staring at a red box. The host
// validates it all again — it must, it is the authority — but a round trip to
// learn a brace is missing is a round trip the console can save.

import type { McpConfigDoc, McpConfigEntry } from "@/api/mcp-config";

/** The fields the host echoes for a reader and ignores on write. */
const ECHOED_FIELDS = ["source", "authConfigured"] as const;

/** A parse that produced a document, or the reason it didn't. */
export type McpJsonParse =
  | { ok: true; doc: McpConfigDoc }
  | { ok: false; message: string };

/** The document as text, in the shape the editor shows it. */
export function formatMcpConfig(doc: McpConfigDoc): string {
  return `${JSON.stringify(doc, null, 2)}\n`;
}

/**
 * Reads the editor's text as an `mcp.json`.
 *
 * Checks only what a client can check without guessing at the host's rules:
 * that it is JSON, that it has the `mcpServers` object, and that each entry is
 * an object naming a URL. Everything else — whether the URL is dialable, whether
 * a manifest server was dropped, whether the credential header is one the host
 * can store — is the host's answer to give, and the editor shows it verbatim
 * rather than second-guessing it here.
 *
 * The two echoed fields (`source`, `authConfigured`) are stripped from the
 * result. The host ignores them, but sending back a provenance the operator may
 * have edited invites the belief that editing it does something.
 */
export function parseMcpConfig(text: string): McpJsonParse {
  if (!text.trim()) {
    return {
      ok: false,
      message: "The file is empty. An MCP-less company is `{ \"mcpServers\": {} }`.",
    };
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (err) {
    return {
      ok: false,
      message: err instanceof Error ? `Not valid JSON — ${err.message}` : "Not valid JSON.",
    };
  }
  if (!isRecord(value)) {
    return { ok: false, message: "mcp.json holds an object with an `mcpServers` key." };
  }
  const servers = value.mcpServers;
  if (servers === undefined) {
    return { ok: false, message: "No `mcpServers` key — every server lives under it." };
  }
  if (!isRecord(servers)) {
    return { ok: false, message: "`mcpServers` maps a server name to its settings." };
  }
  const out: Record<string, McpConfigEntry> = {};
  for (const [name, entry] of Object.entries(servers)) {
    if (!name.trim()) {
      return { ok: false, message: "A server needs a name — one entry's key is empty." };
    }
    if (!isRecord(entry)) {
      return { ok: false, message: `\`${name}\` holds an object, e.g. { "url": "https://…" }.` };
    }
    const url = entry.url ?? entry.endpoint;
    if (typeof url !== "string" || !url.trim()) {
      // `command` is the interesting miss: it is what a server copied out of a
      // desktop config carries, and "needs a url" alone would read as a typo
      // rather than as a server this deployment cannot run at all.
      return {
        ok: false,
        message:
          typeof entry.command === "string"
            ? `\`${name}\` runs as a local subprocess (\`command\`), which this deployment can't launch. It needs a \`url\`.`
            : `\`${name}\` needs a \`url\`.`,
      };
    }
    const clean = { ...entry } as Record<string, unknown>;
    for (const field of ECHOED_FIELDS) delete clean[field];
    out[name] = clean as unknown as McpConfigEntry;
  }
  return { ok: true, doc: { mcpServers: out } };
}

/**
 * Whether the text says anything the loaded document does not.
 *
 * Compares the *parsed* documents, so reformatting, key reordering and
 * whitespace are not edits — a Save button lit by a stray newline trains an
 * operator to ignore it.
 */
export function mcpConfigChanged(text: string, loaded: McpConfigDoc | null): boolean {
  if (loaded === null) return text.trim().length > 0;
  const parsed = parseMcpConfig(text);
  if (!parsed.ok) return true;
  const stripped = parseMcpConfig(formatMcpConfig(loaded));
  if (!stripped.ok) return true;
  return stableJson(parsed.doc) !== stableJson(stripped.doc);
}

/** JSON with object keys sorted, so key order is not a difference. */
function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (isRecord(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
