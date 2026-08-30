// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { KnowledgeGraphFullscreen } from "@/views/overview/kg/KnowledgeGraphFullscreen";
import type { DeptLite } from "@/views/overview/kg/KnowledgeDetail";

/**
 * Issue #1314: while the outage overlay covers the graph, its global keyboard
 * handler must be suspended. `inert` on the covered subtree cannot silence a
 * `window` keydown listener — the handler is simply not registered while the
 * shell is covered, so ←/→/Escape reach nothing in the invisible graph.
 */
const DEPT: DeptLite = {
  deptId: "d-eng",
  teamId: "eng",
  name: "Engineering",
  tagline: "Builds the thing",
  color: "#f87171",
};

let host: HTMLElement;
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

describe("the fullscreen graph chrome while an outage overlay covers it", () => {
  it("does not answer arrow keys or Escape while covered", () => {
    const onNavDept = vi.fn();
    const onBack = vi.fn();
    const onCollapseCore = vi.fn();
    act(() => {
      root.render(
        createElement(KnowledgeGraphFullscreen, {
          deptList: [DEPT],
          currentTeamId: DEPT.teamId,
          currentDept: DEPT,
          toolWiki: null,
          coreOpen: true,
          onCollapseCore,
          onNavDept,
          onBack,
          covered: true,
          children: null,
        }),
      );
    });

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));

    expect(onBack).not.toHaveBeenCalled();
    expect(onCollapseCore).not.toHaveBeenCalled();
    expect(onNavDept).not.toHaveBeenCalled();
  });

  it("answers arrow keys and Escape as usual when not covered", () => {
    const onNavDept = vi.fn();
    const onBack = vi.fn();
    const onCollapseCore = vi.fn();
    act(() => {
      root.render(
        createElement(KnowledgeGraphFullscreen, {
          deptList: [DEPT],
          currentTeamId: DEPT.teamId,
          currentDept: DEPT,
          toolWiki: null,
          coreOpen: true,
          onCollapseCore,
          onNavDept,
          onBack,
          children: null,
        }),
      );
    });

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(onCollapseCore).toHaveBeenCalledTimes(1);

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    // The single-department ring wraps back onto itself: ArrowRight on the
    // current pillar navigates to the same pillar.
    expect(onNavDept).toHaveBeenCalledWith(DEPT.teamId);
  });
});
