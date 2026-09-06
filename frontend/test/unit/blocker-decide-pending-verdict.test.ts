// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ApprovalSummary } from "@/api/types";
import { BlockerDecide } from "@/components/approval-card";
import { blockerVerdictLabel } from "@/lib/language";

/**
 * `BlockerDecide` takes only a boolean `deciding` — the parent tracks the
 * whole card, not which of the four verdicts is in flight. Before this, an
 * in-flight Cancel, Skip or Send answer put the spinner on Retry instead of
 * the control the operator actually pressed.
 */

const T0 = new Date("2026-03-02T10:00:00Z").getTime();

const BLOCKER: ApprovalSummary = {
  id: "b1",
  kind: "blocker.infrastructure",
  amount_usd: null,
  at_millis: T0,
  agent: "eng",
  broadly_grantable: false,
  payload: {
    reason: "the model id `gpt-nope` was rejected",
    needed: "a model id this provider serves",
  },
};

let container: HTMLDivElement;
let root: Root;

async function render(deciding: boolean) {
  await act(async () => {
    root.render(
      createElement(BlockerDecide, {
        approval: BLOCKER,
        askerNames: new Map([["eng", "Engineer"]]),
        now: T0 + 60_000,
        deciding,
        onDecide: () => {},
      }),
    );
  });
}

function button(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll("button")].find((b) =>
    (b.textContent ?? "").trim().startsWith(label),
  );
  if (!found) throw new Error(`no "${label}" button: ${container.textContent}`);
  return found;
}

function spinning(el: HTMLElement): boolean {
  return el.querySelector(".animate-spin") !== null;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function setValue(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
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

describe("BlockerDecide spins the control the operator actually pressed", () => {
  it("spins Skip, not Retry, while a skip is in flight", async () => {
    await render(false);
    await click(button(blockerVerdictLabel("skip")));
    await render(true);

    expect(spinning(button(blockerVerdictLabel("skip")))).toBe(true);
    expect(spinning(button(blockerVerdictLabel("retry")))).toBe(false);
  });

  it("spins Cancel, not Retry, while a cancel is in flight", async () => {
    await render(false);
    await click(button(blockerVerdictLabel("cancel")));
    await render(true);

    expect(spinning(button(blockerVerdictLabel("cancel")))).toBe(true);
    expect(spinning(button(blockerVerdictLabel("retry")))).toBe(false);
  });

  it("spins Send answer, not Retry, while an amend is in flight", async () => {
    await render(false);
    await click(button(blockerVerdictLabel("amend")));
    const field = container.querySelector("textarea") as HTMLTextAreaElement;
    await act(async () => {
      setValue(field, "use gpt-4o-mini instead");
    });
    await click(button("Send answer"));
    await render(true);

    expect(spinning(button("Send answer"))).toBe(true);
    expect(spinning(button(blockerVerdictLabel("retry")))).toBe(false);
  });

  it("clears the pending pick once deciding drops, so the next click spins fresh", async () => {
    await render(false);
    await click(button(blockerVerdictLabel("skip")));
    await render(true);
    expect(spinning(button(blockerVerdictLabel("skip")))).toBe(true);

    await render(false);
    await click(button(blockerVerdictLabel("retry")));
    await render(true);

    expect(spinning(button(blockerVerdictLabel("retry")))).toBe(true);
    expect(spinning(button(blockerVerdictLabel("skip")))).toBe(false);
  });
});
