// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalRow } from "@/views/chat/ApprovalRow";

/**
 * A batch mixing an ordinary gated call with an ungrouped blocker — reachable
 * whenever one turn both parks a gated call and hits a classified failure with
 * no connection to fold by, so both share the turn's own batch key.
 *
 * `decideAll` only ever resolves `pendingGated`, so a card that offered
 * nothing else for the blocker would leave it undecidable from chat: no
 * `BlockerDecide` (that only renders when the blocker is the card's whole
 * remaining business) and no link out to the page.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

const CALL: ApprovalSummary = {
  id: "a1",
  kind: "web_fetch",
  amount_usd: null,
  at_millis: T0,
  agent: "eng",
  thread: "eng",
  batch: "turn-1",
  broadly_grantable: true,
  payload: { url: "https://example.com" },
};

const STUCK: ApprovalSummary = {
  id: "b1",
  kind: "blocker.infrastructure",
  amount_usd: null,
  at_millis: T0,
  agent: "eng",
  thread: "eng",
  batch: "turn-1",
};

interface Decision {
  id: string;
  verdict: Verdict;
  scope: GrantScope;
}

let container: HTMLDivElement;
let root: Root;
let decisions: Decision[];

async function render(approvals: ApprovalSummary[]) {
  await act(async () => {
    root.render(
      createElement(ApprovalRow, {
        approvals,
        now: T0 + 60_000,
        askerNames: new Map([["eng", "Engineer"]]),
        variant: "full" as const,
        deciding: new Map(),
        decided: {},
        failed: {},
        onDecide: (approval: ApprovalSummary, verdict: Verdict, scope: GrantScope) =>
          decisions.push({ id: approval.id, verdict, scope }),
      }),
    );
  });
}

function links(label: string): HTMLAnchorElement[] {
  return [...container.querySelectorAll("a")].filter((a) =>
    (a.textContent ?? "").includes(label),
  );
}

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

describe("a batch mixing an ordinary call with an ungrouped blocker", () => {
  it("keeps the blocker reachable via a link, alongside the call's own buttons", async () => {
    await render([CALL, STUCK]);

    expect(links("Answer in Approvals")).toHaveLength(1);
    // The gated call still gets its own all-or-nothing controls — the fix adds
    // a way to reach the blocker, it does not take away the call's decision.
    expect(() => button("Decline")).not.toThrow();
    expect(() => button("Approve")).not.toThrow();
  });

  it("never resolves the blocker itself from the batch buttons", async () => {
    await render([CALL, STUCK]);
    await click(button("Approve"));

    expect(decisions.map((d) => d.id)).toEqual(["a1"]);
  });
});
