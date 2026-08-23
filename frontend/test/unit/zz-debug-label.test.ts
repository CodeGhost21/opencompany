// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { describe, it } from "vitest";
import type { ApprovalSummary, GrantScope, Verdict } from "@/api/types";
import { ApprovalCard } from "@/views/ApprovalsView";

const T0 = new Date("2026-03-02T10:00:00Z").getTime();
const APPROVAL: ApprovalSummary = {
  id: "a1", kind: "shell", amount_usd: null, at_millis: T0,
  agent: "ops", broadly_grantable: true,
  payload: { command: "rm -rf /tmp/build && make release", cwd: "/srv/app" },
};

describe("debug", () => {
  it("prints labels", async () => {
    (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const container = document.createElement("div");
    document.body.appendChild(container);
    const root: Root = createRoot(container);
    await act(async () => {
      root.render(createElement(ApprovalCard, {
        approval: APPROVAL, now: T0 + 60_000,
        askerNames: new Map([["ops", "Ops"]]), deciding: null,
        batchIndex: 1, batchTotal: 1,
        onDecide: (_v: Verdict, _s: GrantScope) => {},
      }));
    });
    const buttons = Array.from(container.querySelectorAll("button"));
    for (const b of buttons) console.log("LABEL:", JSON.stringify(b.getAttribute("aria-label")));
    act(() => root.unmount());
  });
});
