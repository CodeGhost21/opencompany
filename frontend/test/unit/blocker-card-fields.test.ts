import { HelpCircle } from "lucide-react";
import { describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import { approvalIcon, blockerFields, isBlockerKind } from "@/components/approval-card";

/**
 * The blocker-specialized card render (#1862): the step's reason and what would
 * unblock it, and the connection when the stop is grouped by one.
 */

function approval(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "id">): ApprovalSummary {
  return {
    kind: "blocker.infrastructure",
    amount_usd: null,
    at_millis: 0,
    thread: "eng",
    ...over,
  };
}

describe("isBlockerKind", () => {
  it("recognises the dotted blocker prefix and nothing else", () => {
    expect(isBlockerKind("blocker.infrastructure")).toBe(true);
    expect(isBlockerKind("blocker.information")).toBe(true);
    expect(isBlockerKind("payment.send")).toBe(false);
  });
});

describe("approvalIcon", () => {
  it("gives a blocker its own glyph", () => {
    expect(approvalIcon("blocker.infrastructure")).toBe(HelpCircle);
    expect(approvalIcon("payment.send")).not.toBe(HelpCircle);
  });
});

describe("blockerFields", () => {
  it("reads reason, needed and the grouped connection off the payload", () => {
    const fields = blockerFields(
      approval({
        id: "a",
        payload: {
          reason: "could not connect to mcp server `slack`",
          needed: "the integration reconnected from Apps",
        },
        group_key: "connection:slack",
      }),
    );
    expect(fields).toEqual({
      reason: "could not connect to mcp server `slack`",
      needed: "the integration reconnected from Apps",
      connection: "slack",
    });
  });

  it("is null for a non-blocker approval", () => {
    expect(blockerFields(approval({ id: "a", kind: "payment.send" }))).toBeNull();
  });

  it("degrades to less rather than throwing when the payload is withheld", () => {
    const fields = blockerFields(approval({ id: "a", payload: undefined, contents_hidden: true }));
    expect(fields).toEqual({ reason: undefined, needed: undefined, connection: undefined });
  });
});
