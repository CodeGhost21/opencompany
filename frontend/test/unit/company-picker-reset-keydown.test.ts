// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { CompanyStatus } from "@/api/types";
import { CompanyPicker } from "@/components/company-picker";

/**
 * Codex review on #1828 (PR comment 3861770496): the company card and its
 * nested Reset button are both keyboard-activatable, and the Reset button's
 * `onClick` only calls `stopPropagation()` on the click event. Enter/Space
 * fire `keydown` first, which bubbles up to the card's own `onKeyDown` before
 * the browser synthesizes the button's click — so with focus on Reset, Space
 * calls the card's `onPick` (switching into the company) without ever
 * reaching Reset's click, and Enter can trigger both.
 *
 * Driven through a real mount so the bubble order is the browser's own, not
 * a hand-rolled event list.
 */

const company: CompanyStatus = {
  id: "acme",
  name: "Acme Robotics",
  lifecycle: "active",
  emergency_paused: false,
  pending_approvals: 0,
} as unknown as CompanyStatus;

let container: HTMLDivElement;
let root: Root;
let onPick: ReturnType<typeof vi.fn>;
let onReset: ReturnType<typeof vi.fn>;

function resetButton(): HTMLButtonElement {
  const el = document.querySelector<HTMLButtonElement>(`[data-testid="picker-reset-${company.id}"]`);
  expect(el, "no Reset button rendered").toBeTruthy();
  return el as HTMLButtonElement;
}

async function open() {
  await act(async () => {
    root.render(
      createElement(CompanyPicker, {
        companies: [company],
        onPick,
        onReset,
        canCreate: true,
      }),
    );
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  onPick = vi.fn();
  onReset = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the Reset button on a company card", () => {
  it("does not switch into the company when Space is pressed on Reset", async () => {
    await open();

    await act(async () => {
      resetButton().dispatchEvent(
        new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true }),
      );
    });

    expect(onPick).not.toHaveBeenCalled();
  });

  it("does not switch into the company when Enter is pressed on Reset", async () => {
    await open();

    await act(async () => {
      resetButton().dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }),
      );
    });

    expect(onPick).not.toHaveBeenCalled();
  });
});
