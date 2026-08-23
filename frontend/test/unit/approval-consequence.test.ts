// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import { ApprovalHeadline } from "@/components/approval-card";

/**
 * Issue #1426: outward and irreversible requests must not look like routine
 * internal work. The host classifies the consequence, so this checks the
 * rendered shared headline rather than inferring it from the effect kind.
 */
function approval(overrides: Partial<ApprovalSummary> = {}): ApprovalSummary {
  return {
    id: "a1",
    kind: "composio_execute",
    amount_usd: null,
    at_millis: 0,
    ...overrides,
  } as ApprovalSummary;
}

let container: HTMLDivElement;
let root: Root;

async function render(a: ApprovalSummary) {
  await act(async () => {
    root.render(createElement(ApprovalHeadline, { approval: a }));
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

describe("the consequence on an approval card", () => {
  it.each([
    ["spend", "Spends money", "bg-status-failed-soft"],
    ["send", "Leaves the company", "bg-status-blocked-soft"],
    ["sign", "Makes a commitment", "bg-status-failed-soft"],
    ["publish", "Goes public", "bg-status-running-soft"],
    ["hire", "Changes who can act", "bg-status-done-soft"],
    ["identity", "Changes who can act", "bg-status-done-soft"],
  ] as const)("marks %s approvals as %s", async (group, label, iconClass) => {
    await render(approval({ group }));

    expect(container.textContent).toContain(label);
    expect(container.querySelector(`.${iconClass}`)).not.toBeNull();
  });

  it("leaves internal and old-host approvals unmarked", async () => {
    await render(approval({ group: "other" }));
    expect(container.textContent).not.toContain("Spends money");
    expect(container.textContent).not.toContain("Leaves the company");
    expect(container.querySelector(".bg-muted.text-foreground")).not.toBeNull();

    await render(approval());
    expect(container.textContent).not.toContain("Spends money");
    expect(container.textContent).not.toContain("Leaves the company");
    expect(container.querySelector(".bg-muted.text-foreground")).not.toBeNull();
  });
});
