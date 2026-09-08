// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalCard } from "@/views/ApprovalsView";

/**
 * Resolve and extend are admin-scoped on the host. `contents_hidden` is set by
 * the same role check the host applies to those routes, so a card a member
 * cannot read is exactly a card they cannot decide — the assertions below
 * exercise that as the single source of truth for whether the decide footer
 * renders at all.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

const BASE: ApprovalSummary = {
  id: "a1",
  kind: "payment.send",
  amount_usd: 1200,
  at_millis: T0,
  expires_at_millis: T0 + 3_600_000,
  agent: "ops",
  broadly_grantable: true,
  broadly_deniable: true,
  payload: { to: "vendor@example.test" },
};

let container: HTMLDivElement;
let root: Root;

async function render(approval: ApprovalSummary) {
  await act(async () => {
    root.render(
      createElement(ApprovalCard, {
        approval,
        now: T0 + 60_000,
        askerNames: new Map([["ops", "Ops"]]),
        deciding: null,
        batchIndex: 1,
        batchTotal: 1,
        onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
        onExtend: () => {},
      }),
    );
  });
}

function query(testid: string): HTMLElement | null {
  return container.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
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

describe("ApprovalCard's decide footer, gated on contents_hidden", () => {
  it("offers Approve, Decline and Extend, and no notice, when contents are readable", async () => {
    await render(BASE);

    expect(query("approval-decide")).not.toBeNull();
    expect(buttons().some((t) => t.includes("Approve"))).toBe(true);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(true);
    expect(buttons().some((t) => t.includes("Extend"))).toBe(true);
    expect(container.textContent).not.toContain("Details hidden by your role");
  });

  it("withholds Approve, Decline and Extend, and states why once, when contents are hidden", async () => {
    // A real redaction nulls `amount_usd` alongside setting the flag — the host
    // never leaves the money visible and only the flag flipped.
    await render({ ...BASE, amount_usd: null, contents_hidden: true });

    expect(query("approval-decide")).toBeNull();
    expect(buttons().some((t) => t.includes("Approve"))).toBe(false);
    expect(buttons().some((t) => t.includes("Decline"))).toBe(false);
    expect(buttons().some((t) => t.includes("Extend"))).toBe(false);

    // The read-only content survives — a hidden card is not a blank one.
    expect(container.textContent).toContain("Details hidden by your role");
    expect(container.textContent).toContain("Amount hidden");

    // Exactly one explanation, not the payload's plus a second one repeating
    // it beneath the (now absent) buttons.
    expect(container.textContent?.match(/hidden by your role/g)?.length).toBe(1);
  });

  it("withholds the grant-scope controls alongside the buttons they configure", async () => {
    await render({ ...BASE, amount_usd: null, contents_hidden: true });

    expect(container.textContent).not.toContain("If you approve");
    expect(container.textContent).not.toContain("If you decline");
  });

  it("still offers the scope controls to a reader who may see contents", async () => {
    await render(BASE);

    expect(container.textContent).toContain("If you approve");
    expect(container.textContent).toContain("If you decline");
  });
});
