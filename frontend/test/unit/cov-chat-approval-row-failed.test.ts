// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalRow } from "@/views/chat/ApprovalRow";

/**
 * `AppShell.decideApproval` (the write behind this row's Approve/Decline) is
 * `CompanyAuth`, not admin-scoped — any signed-in company principal may
 * settle a request they can see, so there is no member/admin gate to pin
 * here. What the row itself owes is honesty once that write fails: `failed`
 * is keyed per approval id specifically so a batch decision can never read as
 * "all authorised" when one of them was not (`decideApproval`'s own doc), and
 * the row must stay retryable rather than stuck on its busy state.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

function approval(over: Partial<ApprovalSummary> & Pick<ApprovalSummary, "id">): ApprovalSummary {
  return {
    kind: "web_fetch",
    amount_usd: null,
    at_millis: T0,
    agent: "eng",
    thread: "eng",
    ...over,
  };
}

interface Decision {
  id: string;
  verdict: Verdict;
  scope: GrantScope;
}

let container: HTMLDivElement;
let root: Root;
let decisions: Decision[];

async function render(props: {
  approvals: ApprovalSummary[];
  decided?: Record<string, Verdict>;
  failed?: Record<string, string>;
}) {
  await act(async () => {
    root.render(
      createElement(ApprovalRow, {
        approvals: props.approvals,
        now: T0 + 60_000,
        askerNames: new Map([["eng", "Engineer"]]),
        variant: "full" as const,
        deciding: new Map(),
        decided: props.decided ?? {},
        failed: props.failed ?? {},
        onDecide: (approval: ApprovalSummary, verdict: Verdict, scope: GrantScope) =>
          decisions.push({ id: approval.id, verdict, scope }),
      }),
    );
  });
}

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

function button(label: string): HTMLButtonElement {
  const match = [...container.querySelectorAll("button")].find((b) =>
    (b.textContent ?? "").includes(label),
  );
  if (!match) throw new Error(`no "${label}" button on the card: ${container.textContent}`);
  return match as HTMLButtonElement;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

describe("a single approval whose resolveApproval call failed (FAIL)", () => {
  const A1 = approval({ id: "a1" });

  it("names the failure instead of leaving the card looking merely pending", async () => {
    await render({ approvals: [A1], failed: { a1: "network error" } });

    expect(container.textContent).toContain("Not recorded — try again");
  });

  it("keeps Approve and Decline live so the operator can retry, not stuck busy", async () => {
    await render({ approvals: [A1], failed: { a1: "network error" } });

    const approve = button("Approve");
    const decline = button("Decline");
    expect(approve.disabled).toBe(false);
    expect(decline.disabled).toBe(false);

    await click(approve);
    expect(decisions).toEqual([{ id: "a1", verdict: "approve", scope: { kind: "once" } }]);
  });

  it("shows nothing extra when nothing has failed", async () => {
    await render({ approvals: [A1] });

    expect(container.textContent).not.toContain("try again");
  });
});

/**
 * A batch where one call's decision failed to record and the others landed —
 * the row must say so per item, never as a single "it worked" or a single
 * "it failed" that erases which is which (`decideApproval`'s own doc: "a
 * failure on the third leaves two effects authorised and one not").
 */
describe("a batch where only one item's decision failed to record (AUTH — must not claim a false blanket outcome)", () => {
  const A1 = approval({ id: "a1", batch: "turn-1" });
  const A2 = approval({ id: "a2", batch: "turn-1", at_millis: T0 + 1 });
  const A3 = approval({ id: "a3", batch: "turn-1", at_millis: T0 + 2 });

  it("names exactly the one that failed, not the whole batch", async () => {
    await render({
      approvals: [A1, A2, A3],
      decided: { a1: "approve", a2: "approve" },
      failed: { a3: "network error" },
    });

    // Two of three settled; the third alone did not record — not the whole
    // batch, and each item's own row says which it was.
    expect(container.textContent).toContain("1 of 3 weren't recorded — try again");
    expect(container.textContent).not.toContain("None of the 3");
    expect(container.textContent).toContain("Not recorded — network error");
    // The two that landed still say so, per item — not swallowed by the one
    // that failed.
    expect(container.textContent?.match(/Approved/g)?.length).toBe(2);
  });

  it("still offers a retry for the batch's remaining (failed) item", async () => {
    await render({
      approvals: [A1, A2, A3],
      decided: { a1: "approve", a2: "approve" },
      failed: { a3: "network error" },
    });

    const approve = button("Approve");
    expect(approve.disabled).toBe(false);
    await click(approve);
    // `decideAll` re-sends every still-pending item — here just a3, the one
    // that never recorded — not the two already settled.
    expect(decisions).toEqual([{ id: "a3", verdict: "approve", scope: { kind: "once" } }]);
  });
});
