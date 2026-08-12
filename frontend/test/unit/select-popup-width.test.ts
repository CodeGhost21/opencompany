// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/**
 * The select popup's width (issue #811).
 *
 * The defect was one class: the popup was `w-(--anchor-width)`, an exact match
 * to the trigger. Triggers are `w-fit`, so a short selected value produced a
 * popup too narrow for its own options and every label was cut mid-word —
 * `Trigger — starts the w…` in the workflow node-kind picker, where the cut half
 * is the part that explains what the kind does.
 *
 * jsdom does no layout, so this cannot assert rendered pixels. What it CAN
 * assert is the rule itself, and that is the whole defect: the popup must not
 * bind `width` to the anchor. A revert to `w-(--anchor-width)` fails here rather
 * than shipping and being noticed by an operator.
 */

let container: HTMLDivElement;
let root: Root;

function renderSelect() {
  act(() => {
    root.render(
      createElement(
        Select,
        { defaultValue: "trigger", open: true },
        createElement(SelectTrigger, null, createElement(SelectValue)),
        createElement(
          SelectContent,
          null,
          createElement(SelectItem, { value: "trigger" }, "Trigger — starts the workflow"),
          createElement(SelectItem, { value: "agent" }, "Agent — a teammate performs a step"),
        ),
      ),
    );
  });
}

/** The popup renders through a portal, so it is on `document`, not `container`. */
function popup(): HTMLElement | null {
  return document.querySelector('[data-slot="select-content"]');
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

describe("the select popup's width", () => {
  it("does not pin its width to the trigger", () => {
    renderSelect();
    const cls = popup()?.className ?? "";
    // The exact-width binding, in either spelling Tailwind accepts.
    expect(cls).not.toMatch(/(^|\s)w-\(--anchor-width\)/);
    expect(cls).not.toMatch(/(^|\s)w-\[var\(--anchor-width\)\]/);
  });

  it("takes the trigger's width as a floor, so it is never narrower", () => {
    renderSelect();
    expect(popup()?.className ?? "").toMatch(/min-w-\[max\(9rem,var\(--anchor-width\)\)\]/);
  });

  it("keeps a ceiling, so one long option cannot push it off-screen", () => {
    renderSelect();
    expect(popup()?.className ?? "").toMatch(/max-w-\[min\(28rem,var\(--available-width\)\)\]/);
  });

  it("still renders its options", () => {
    // Guards the guard: if the popup stopped rendering, every assertion above
    // would pass against an empty string.
    renderSelect();
    expect(popup()?.textContent).toContain("Trigger — starts the workflow");
    expect(popup()?.textContent).toContain("Agent — a teammate performs a step");
  });
});
