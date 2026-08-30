// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { approvalThreadLink, type ApprovalThreadLink } from "@/components/approval-card";
import type { Desk } from "@/lib/desks";
import type { TeamMember } from "@/lib/team";
import { ApprovalCard } from "@/views/ApprovalsView";

const T0 = new Date("2026-08-20T20:00:00Z").getTime();
const ENGINEERING: Desk[] = [
  { id: "engineering", channel: "engineering", name: "Engineering", blurb: "" },
];
const NO_MEMBERS: TeamMember[] = [];

function nativeApproval(over: Partial<ApprovalSummary> = {}): ApprovalSummary {
  return {
    id: "a1",
    kind: "runtime.unlabelled_effect",
    amount_usd: null,
    at_millis: T0,
    agent: null,
    ...over,
  };
}

let container: HTMLDivElement;
let root: Root;

async function render(approval: ApprovalSummary, thread?: ApprovalThreadLink | null) {
  await act(async () => {
    root.render(
      createElement(ApprovalCard, {
        approval,
        now: T0 + 60_000,
        askerNames: new Map(),
        thread: thread === undefined ? approvalThreadLink(approval, ENGINEERING, NO_MEMBERS) : thread,
        deciding: null,
        batchIndex: 1,
        batchTotal: 1,
        onDecide: (_verdict: Verdict, _scope: GrantScope) => {},
      }),
    );
  });
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

describe("an unlabelled approval without details (#1419)", () => {
  it("links back to its conversation and says that the host supplied no details", async () => {
    const approval = nativeApproval({ thread: "engineering" });
    await render(approval);

    expect(container.textContent).toContain("Do something that needs your sign-off");
    expect(container.textContent).toContain("No further details were supplied.");
    const thread = [...container.querySelectorAll("a")].find((link) =>
      link.textContent?.includes("#engineering"),
    );
    expect(thread?.getAttribute("href")).toBe("#/chat/engineering");
  });

  it("does not invent a conversation for an absent or unresolvable thread", async () => {
    expect(approvalThreadLink(nativeApproval(), ENGINEERING, NO_MEMBERS)).toBeNull();
    expect(
      approvalThreadLink(nativeApproval({ thread: "someone-else" }), ENGINEERING, NO_MEMBERS),
    ).toBeNull();

    await render(nativeApproval({ thread: "someone-else" }));
    expect(container.textContent).not.toContain("Asked in");
  });

  it("writes a DM channel id unescaped, so the hash router can resolve it", async () => {
    // A DM's channel id is `dm:<agent-id>`. The hash router splits
    // `#/chat/…` on "/" without decoding, so `encodeURIComponent` turning
    // the ":" into `%3A` would produce a segment no channel matches.
    await render(nativeApproval(), { channelId: "dm:ada-1f3k", label: "Ada" });

    const thread = [...container.querySelectorAll("a")].find(
      (link) => link.getAttribute("href") === "#/chat/dm:ada-1f3k",
    );
    expect(thread?.getAttribute("href")).toBe("#/chat/dm:ada-1f3k");
  });
});
