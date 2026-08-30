import { describe, expect, it } from "vitest";

import type { McpConfigDoc } from "@/api/mcp-config";
import { formatMcpConfig, mcpConfigChanged, parseMcpConfig } from "@/lib/mcp-json";

/**
 * The `mcp.json` editor's local reading of what an operator typed.
 *
 * A form can only produce shapes the host accepts; a text buffer can say
 * anything, so the surface has to distinguish "this isn't JSON" from "this isn't
 * the shape" from "the host refused it" — and it must not over-reach into the
 * third, which is the host's answer to give. These hold both halves of that:
 * the checks that are worth making locally, and the ones deliberately not made.
 */

const doc: McpConfigDoc = {
  mcpServers: {
    notion: {
      type: "http",
      url: "https://notion.example/mcp",
      enabled: true,
      source: "manifest",
      authConfigured: true,
    },
  },
};

describe("parseMcpConfig", () => {
  it("reads a document the host would serve", () => {
    const parsed = parseMcpConfig(formatMcpConfig(doc));
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    expect(parsed.doc.mcpServers.notion.url).toBe("https://notion.example/mcp");
  });

  it("strips the fields the host only echoes", () => {
    // Sending back a provenance the operator may have edited invites the belief
    // that editing it does something.
    const parsed = parseMcpConfig(formatMcpConfig(doc));
    if (!parsed.ok) throw new Error(parsed.message);
    expect(parsed.doc.mcpServers.notion).not.toHaveProperty("source");
    expect(parsed.doc.mcpServers.notion).not.toHaveProperty("authConfigured");
  });

  it("names a syntax error rather than reporting a shape problem", () => {
    const parsed = parseMcpConfig('{ "mcpServers": { ');
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toMatch(/not valid json/i);
  });

  it("refuses an empty buffer, and says what an MCP-less company looks like", () => {
    // The dangerous reading: a blank editor is not "remove everything" by
    // accident. Saying the empty document explicitly is what makes the
    // deliberate version available.
    const parsed = parseMcpConfig("   ");
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toContain('{ "mcpServers": {} }');
  });

  it("accepts an empty server set — a company can genuinely have none", () => {
    expect(parseMcpConfig('{"mcpServers":{}}').ok).toBe(true);
  });

  it("names the missing key when the document has no mcpServers", () => {
    const parsed = parseMcpConfig('{"servers":{}}');
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toContain("mcpServers");
  });

  it("names the entry that has no url", () => {
    const parsed = parseMcpConfig('{"mcpServers":{"notion":{"enabled":true}}}');
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toContain("notion");
    expect(parsed.message).toContain("url");
  });

  it("tells a stdio entry why it cannot run here, not that it mistyped a url", () => {
    // What a server copied out of a desktop config carries. "Needs a url" reads
    // as a typo; the real problem is that this deployment launches no
    // subprocess, and an operator who does not learn that pastes it again.
    const parsed = parseMcpConfig('{"mcpServers":{"fs":{"command":"npx"}}}');
    expect(parsed.ok).toBe(false);
    if (parsed.ok) return;
    expect(parsed.message).toMatch(/subprocess/i);
  });

  it("accepts `endpoint` as a synonym, so a row body pastes in", () => {
    const parsed = parseMcpConfig('{"mcpServers":{"notion":{"endpoint":"https://x/mcp"}}}');
    expect(parsed.ok).toBe(true);
  });

  it("leaves the host's own rules to the host", () => {
    // An `http://` endpoint, an unknown key, a timeout of zero: all things the
    // host validates and may refuse. The editor must not pre-empt that — a
    // console copy of the host's rules is one more thing to keep in step, and
    // the failure mode is refusing a document the host would have accepted.
    const parsed = parseMcpConfig(
      '{"mcpServers":{"x":{"url":"http://internal/mcp","timeoutSecs":0,"whatever":1}}}',
    );
    expect(parsed.ok).toBe(true);
  });
});

describe("mcpConfigChanged", () => {
  it("treats reformatting and key reordering as no change", () => {
    // A Save button lit by a stray newline trains an operator to ignore it.
    const text = JSON.stringify({
      mcpServers: {
        notion: {
          authConfigured: true,
          source: "manifest",
          enabled: true,
          url: "https://notion.example/mcp",
          type: "http",
        },
      },
    });
    expect(mcpConfigChanged(text, doc)).toBe(false);
  });

  it("sees a real edit", () => {
    const text = formatMcpConfig(doc).replace('"enabled": true', '"enabled": false');
    expect(mcpConfigChanged(text, doc)).toBe(true);
  });

  it("sees an added credential header, which the loaded document never carries", () => {
    const text = formatMcpConfig(doc).replace(
      '"type": "http"',
      '"type": "http",\n      "headers": { "Authorization": "Bearer x" }',
    );
    expect(mcpConfigChanged(text, doc)).toBe(true);
  });

  it("treats an unparsable buffer as changed, so Revert stays available", () => {
    expect(mcpConfigChanged("{", doc)).toBe(true);
  });
});
