// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { WorkflowPlacementNotice } from "@/views/overview/kg/WorkflowPlacementNotice";

/**
 * The caveat panel must open UPWARD and must not float.
 *
 * Its only caller is the compact legend, which the fullscreen field pins at
 * `bottom-5 left-5` inside an `overflow-hidden` container
 * (`KnowledgeGraphFullscreen.tsx`). That box clips in both axes, and the
 * component was caught by it twice:
 *
 *  - `top-full` dropped the panel into the ~20px strip below the legend, so it
 *    was cut off vertically;
 *  - an absolute panel anchored to one edge of the summary then ran off the
 *    opposite edge once the legend wrapped at narrow widths.
 *
 * Keeping the panel in flow and stacking it above the summary removes both:
 * the legend box just grows upward into the field, the one direction that
 * always has room.
 *
 * jsdom has no layout engine, so the clipping itself cannot be measured here.
 * What is asserted instead are the three decisions that cause it — the stacking
 * direction, the panel staying in flow, and its width being capped — plus the
 * DOM order the native disclosure depends on.
 */

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

function render(): { details: HTMLElement; panel: HTMLElement } {
  act(() => root.render(createElement(WorkflowPlacementNotice)));
  const details = host.querySelector("details");
  const panel = host.querySelector("details > p");
  expect(details).not.toBeNull();
  expect(panel).not.toBeNull();
  return { details: details as HTMLElement, panel: panel as HTMLElement };
}

const classesOf = (el: HTMLElement) => el.className.split(/\s+/);

describe("WorkflowPlacementNotice panel placement", () => {
  it("stacks the panel above the summary", () => {
    const classes = classesOf(render().details);

    expect(classes).toContain("flex");
    expect(classes).toContain("flex-col-reverse");
  });

  it("keeps the summary first in the DOM, so the disclosure stays native", () => {
    // `flex-col-reverse` reverses only the visual order. Reversing the DOM
    // instead would move the summary out of the position `<details>` requires
    // and break focus order with it.
    const { details } = render();

    expect(details.firstElementChild?.tagName).toBe("SUMMARY");
  });

  it("leaves the panel in flow rather than floating it over the field", () => {
    const classes = classesOf(render().panel);

    // The two failed attempts: a downward popover, then an edge-anchored one.
    expect(classes).not.toContain("absolute");
    expect(classes).not.toContain("top-full");
    expect(classes).not.toContain("bottom-full");
  });

  it("caps the panel width so a narrow field cannot clip it sideways", () => {
    expect(classesOf(render().panel)).toContain("max-w-full");
  });

  it("hides the panel until the disclosure is open", () => {
    const classes = classesOf(render().panel);

    expect(classes).toContain("hidden");
    expect(classes).toContain("group-open:block");
  });

  it("still renders the caveat text inside the disclosure", () => {
    expect(render().panel.textContent?.trim().length).toBeGreaterThan(0);
  });
});
