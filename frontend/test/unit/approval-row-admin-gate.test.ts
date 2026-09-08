// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalRow } from "@/views/chat/ApprovalRow";

/**
 * The chat and board rendering of a parked approval shares `contents_hidden`
 * with the Approvals page's own gate (`approval-admin-gate.test.ts`): the same
 * host role check backs both the redaction and the admin-scoped resolve route,
 * so a card whose contents this viewer cannot read is a card they cannot
 * decide from any surface that renders it.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

const CALL: ApprovalSummary = {
  id: "a1",
  kind: "web_fetch",
  amount_usd: null,
  at_millis: T0,
  agent: "eng",
  broadly_grantable: true,
  payload: { url: "https://example.com" },
};

let container: HTMLDivElement;
let root: Root;

async function render(
  approval: ApprovalSummary,
  variant: "full" | "compact" | "card" = "full",
) {
  await act(async () => {
    root.render(
      createElement(ApprovalRow, {
        approvals: [approval],
        now: T0 + 60_000,
        askerNames: new Map([["eng", "Engineer"]]),
        variant,
        deciding: new Map(),
        decided: {},
        failed: {},
        onDecide: (_approval: ApprovalSummary, _verdict: Verdict, _scope: GrantScope) => {},
      }),
    );
  });
}

function buttons(): string[] {
  return [...container.querySelectorAll("button")].map((b) => b.textContent ?? "");
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("ApprovalRow's decide actions, gated on contents_hidden", () => {
  it("offers Approve and Decline on the full transcript card when contents are readable", async () => {
    await render(CALL, "full");

    expect(buttons().some((t) => t.includes("Approve"))).toBe(true);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(true);
  });

  it("withholds Approve and Decline on the full card when contents are hidden", async () => {
    await render({ ...CALL, contents_hidden: true }, "full");

    expect(buttons().some((t) => t.includes("Approve"))).toBe(false);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(false);
    // The card still says what it is and why it cannot be decided here.
    expect(container.textContent).toContain("hidden by your role");
  });

  it("withholds the grant-scope control on the full card when contents are hidden", async () => {
    await render({ ...CALL, contents_hidden: true }, "full");

    expect(container.textContent).not.toContain("If you approve");
  });

  it("withholds the compact chat row's actions when contents are hidden", async () => {
    await render({ ...CALL, contents_hidden: true }, "compact");

    expect(buttons().some((t) => t.includes("Approve"))).toBe(false);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(false);
  });

  it("withholds the board card's actions when contents are hidden", async () => {
    await render({ ...CALL, contents_hidden: true }, "card");

    expect(buttons().some((t) => t.includes("Approve"))).toBe(false);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(false);
  });
});
