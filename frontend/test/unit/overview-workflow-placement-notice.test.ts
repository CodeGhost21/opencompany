// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { DERIVED_NOTICE } from "@/views/overview/kg/adapter";
import { WorkflowPlacementNotice } from "@/views/overview/kg/WorkflowPlacementNotice";

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(createElement(WorkflowPlacementNotice)));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("the workflow placement caveat", () => {
  it("is a visible, keyboard- and touch-operable disclosure of the canonical notice", () => {
    const disclosure = host.querySelector("details");
    const summary = disclosure?.querySelector("summary");
    const notice = disclosure?.querySelector("p");

    expect(disclosure).not.toBeNull();
    expect(summary?.textContent).toContain("workflow placement is inferred");
    expect(summary?.className).toContain("cursor-pointer");
    expect(summary?.className).toContain("focus-visible:ring-1");
    expect(notice?.textContent).toBe(DERIVED_NOTICE);

    summary?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(disclosure?.open).toBe(true);
  });
});
