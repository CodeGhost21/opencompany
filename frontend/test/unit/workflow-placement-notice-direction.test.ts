// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { WorkflowPlacementNotice } from "@/views/overview/kg/WorkflowPlacementNotice";

/**
 * The caveat panel must open UPWARD.
 *
 * Its only caller is the compact legend, which the fullscreen field pins at
 * `bottom-5` inside an `overflow-hidden` container
 * (`KnowledgeGraphFullscreen.tsx`). A downward panel (`top-full`) therefore
 * has ~20px of field beneath it and is clipped away, which defeats the whole
 * point of giving the caveat a non-hover affordance — pointer, keyboard, and
 * touch users open the disclosure and still cannot read it.
 *
 * jsdom has no layout engine, so the clipping itself cannot be measured here.
 * What is asserted instead is the one decision that causes it: the side of the
 * summary the panel is anchored to.
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

function panel(): HTMLElement {
  act(() => root.render(createElement(WorkflowPlacementNotice)));
  const p = host.querySelector("details > p");
  expect(p).not.toBeNull();
  return p as HTMLElement;
}

describe("WorkflowPlacementNotice panel direction", () => {
  it("anchors the panel above the summary, not below it", () => {
    const classes = panel().className.split(/\s+/);

    expect(classes).toContain("bottom-full");
    // The bug: `top-full` dropped the panel into the clipped strip under a
    // bottom-anchored legend.
    expect(classes).not.toContain("top-full");
  });

  it("offsets the gap upward too, so the margin follows the anchor", () => {
    const classes = panel().className.split(/\s+/);

    expect(classes).toContain("mb-1");
    expect(classes).not.toContain("mt-1");
  });

  it("still renders the caveat text inside the disclosure", () => {
    expect(panel().textContent?.trim().length).toBeGreaterThan(0);
  });
});
