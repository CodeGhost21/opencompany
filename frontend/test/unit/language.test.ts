import { describe, expect, it } from "vitest";

import { approvalAction, payloadLines, toolAction } from "@/lib/language";
import type { ApprovalSummary } from "@/api/types";

/**
 * The gated tools an operator is asked to sign off on say what they are (#701).
 *
 * The Rust guard (`every_consequence_tool_has_a_console_label` in
 * `src/policy/consequence.rs`) asserts that each of these tools has a *key* in
 * one of the label tables — that is the half which cannot be checked from here,
 * because the declaration table it reads is Rust. This is the other half: that
 * the key is reachable through the resolution the console actually performs.
 *
 * Both are needed. A key in `TOOL_LABELS` satisfies the guard while remaining
 * invisible if `approvalAction`'s rung order ever changed, and the whole defect
 * class (#372, #551 → #671, #701) is labels that exist somewhere and reach
 * nobody.
 */

function approval(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "kind">): ApprovalSummary {
  return { id: "a1", amount_usd: null, at_millis: 1_000, agent: "ceo", ...over };
}

/** What each per-call tool gate asks for, keyed by the kind it parks under. */
const PER_CALL: Record<string, string> = {
  curl: "Download a file from the internet",
  http_request: "Make a request to a web address",
  git_operations: "Run a git command in its workspace",
  read_workspace_state: "Check its workspace's git status",
  mcp_call_tool: "Use a tool on a connected server",
  publish_artifact: "Publish a file it produced",
  run_workflow: "Run one of its saved workflows",
};

/** The four an operator may grant standing on, so the #374 list renders them. */
const GRANTABLE: Record<string, string> = {
  file_write: "Write a file in its workspace",
  edit: "Edit a file in its workspace",
  apply_patch: "Edit several files in its workspace at once",
  csv_export: "Save data as a spreadsheet file in its workspace",
};

describe("an approval card for a gated tool", () => {
  for (const [kind, sentence] of Object.entries(PER_CALL)) {
    it(`says what \`${kind}\` is asking for`, () => {
      expect(approvalAction(approval({ kind }))).toBe(sentence);
    });
  }

  it("never leaves one of them on the generic fallback", () => {
    for (const kind of [...Object.keys(PER_CALL), ...Object.keys(GRANTABLE)]) {
      expect(approvalAction(approval({ kind }))).not.toBe("Use one of its tools");
    }
  });

  it("still resolves a business effect through the effect glossary first", () => {
    // Rung 1 is unchanged by #701 — nothing was added to EFFECT_LABELS, and a
    // tool label that shadowed one of its entries would be a silent rewording of
    // a card that has read the same way since #372.
    expect(approvalAction(approval({ kind: "payment.send" }))).toBe("Send a payment");
    expect(approvalAction(approval({ kind: "mcp_registry_tool_call" }))).toBe(
      "Use a connected tool",
    );
  });
});

describe("the standing permissions list", () => {
  for (const [kind, sentence] of Object.entries({ ...PER_CALL, ...GRANTABLE })) {
    it(`names \`${kind}\` without an approval to read it from`, () => {
      expect(toolAction(kind)).toBe(sentence);
    });
  }

  it("tells the four grantable tools apart", () => {
    // The point of labelling them: this list has no payload block, so two rows
    // reading the same sentence are two permissions an operator cannot choose
    // between.
    const rendered = Object.keys(GRANTABLE).map(toolAction);
    expect(new Set(rendered).size).toBe(rendered.length);
  });
});

describe("the payload block underneath the label", () => {
  // The labels for these three are the shortest of the eleven precisely because
  // the address is right below them. If the ordering regresses, the card still
  // renders — it just buries the one argument being consented to under a header
  // map, which is the failure no type catches.
  it("leads a request with its address, not with its headers", () => {
    const lines = payloadLines(
      approval({
        kind: "http_request",
        payload: { headers: { Authorization: "…" }, body: "{}", url: "https://x.test", method: "POST" },
      }),
    );
    expect(lines.map((l) => l.label)).toStrictEqual(["url", "method", "headers", "body"]);
  });

  it("leads a download with its address", () => {
    const lines = payloadLines(
      approval({ kind: "curl", payload: { dest_path: "out.bin", url: "https://x.test/f" } }),
    );
    expect(lines.map((l) => l.label)).toStrictEqual(["url", "dest_path"]);
  });

  it("leads a git call with the operation it is about to run", () => {
    const lines = payloadLines(
      approval({ kind: "git_operations", payload: { message: "wip", operation: "commit" } }),
    );
    expect(lines[0]).toStrictEqual({ label: "operation", value: "commit" });
  });
});

describe("a kind nobody has named", () => {
  it("says a teammate wants a tool rather than inventing one", () => {
    expect(approvalAction(approval({ kind: "some_tool_nobody_declared" }))).toBe(
      "Use one of its tools",
    );
  });

  it("says less again when there is no teammate to name", () => {
    expect(approvalAction(approval({ kind: "some.native.effect", agent: null }))).toBe(
      "Do something that needs your sign-off",
    );
  });

  it("falls back the same way from the permissions list", () => {
    expect(toolAction("some_tool_nobody_declared")).toBe("Use one of its tools");
  });
});
