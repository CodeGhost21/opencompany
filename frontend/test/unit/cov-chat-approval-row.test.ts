// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalRow } from "@/views/chat/ApprovalRow";

/**
 * The inline chat approval card's Approve/Decline (`resolveApproval`, `POST
 * {scope}/approvals/{aid}`, `operator.rs`). That route runs behind
 * `CompanyAuth` — any authenticated principal, not admin-only, the same gate
 * `pause`/`resume` carry (see `settings-general-admin-only.test.ts`'s note on
 * lifecycle) — and `ApprovalRow` takes no `canManage`/`isAdmin` prop at all:
 * every caller gets the same two buttons. This pins that the card is live for
 * whoever is handed it, and that a decision the host refuses is said on the
 * card rather than silently dropped or shown as settled.
 *
 * The ledger names Approve/Revise and a failure-toast gap on "the Approve/Revise
 * buttons themselves" — this component's Approve/Decline are that control.
 * `ThreadPanel`'s own inline Approve is the *other* card the phrasing could
 * mean (`cov-chat-thread-panel-review-auth.test.ts`, `cov-chat-review-card-
 * fail.test.ts`) and is a distinct route (`chat/review`, not `approvals`) —
 * both are covered so the ambiguity in the ledger costs nothing.
 */

const T0 = Date.UTC(2026, 8, 1, 9, 0, 0);

function approval(id: string): ApprovalSummary {
  return {
    id,
    kind: "web_fetch",
    amount_usd: null,
    at_millis: T0,
    agent: "seo",
    thread: "desk-marketing",
    batch: "turn-1",
    broadly_grantable: true,
    payload: { url: "https://example.com/items" },
  };
}

const CALL = approval("a1");

interface Decision {
  id: string;
  verdict: Verdict;
  scope: GrantScope;
}

let container: HTMLDivElement;
let root: Root;
let decisions: Decision[];

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  decisions = [];
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(
  failed: Record<string, string> = {},
  deciding: ReadonlyMap<string, Verdict> = new Map(),
) {
  await act(async () => {
    root.render(
      createElement(ApprovalRow, {
        approvals: [CALL],
        now: T0 + 60_000,
        askerNames: new Map([["seo", "SEO Specialist"]]),
        variant: "full",
        deciding,
        decided: {},
        failed,
        onDecide: (a: ApprovalSummary, verdict: Verdict, scope: GrantScope) =>
          decisions.push({ id: a.id, verdict, scope }),
      }),
    );
  });
}

function button(label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find((b) => (b.textContent ?? "").includes(label));
  if (!match) throw new Error(`no "${label}" button on the card: ${container.textContent}`);
  return match as HTMLButtonElement;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("the chat approval card, offered with no role prop at all", () => {
  it("renders live Approve and Decline for whoever the card was shown to", async () => {
    await render();

    expect(button("Approve").disabled).toBe(false);
    expect(button("Decline").disabled).toBe(false);
  });

  it("wires Approve straight to onDecide", async () => {
    await render();
    await click(button("Approve"));

    expect(decisions).toEqual([{ id: "a1", verdict: "approve", scope: { kind: "once" } }]);
  });
});

describe("a decision the host refuses", () => {
  it("names the refusal on the card and keeps Approve/Decline live for a retry", async () => {
    await render({ a1: "the company is paused" });

    expect(container.textContent).toContain("Not recorded — try again");
    expect(button("Approve").disabled).toBe(false);
    expect(button("Decline").disabled).toBe(false);
  });

  it("does not remove the card or claim it settled while a decision is still in flight", async () => {
    await render({}, new Map([["a1", "approve"]]));

    // Busy, not gone and not claiming success: the row is still asking.
    expect(container.querySelector("button")).not.toBeNull();
    expect(container.textContent).not.toContain("Approved");
  });
});
