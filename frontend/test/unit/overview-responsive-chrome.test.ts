// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { DeptLite } from "@/views/overview/kg/KnowledgeDetail";
import { KnowledgeGraphFullscreen } from "@/views/overview/kg/KnowledgeGraphFullscreen";

/**
 * Issue #1385: the fullscreen graph's legend was a single 554px row and its
 * two 128px-by-48px paddles stayed that size at every viewport. At 390px the
 * legend ran beyond the clipped canvas, while the paddles covered its busiest
 * middle band. CSS is responsible for the measured layout, so this component
 * test guards the responsive class contract at the two reported breakpoints.
 */

let host: HTMLElement;
let root: Root;

const DESKS: DeptLite[] = [
  { deptId: "desk:eng", teamId: "team:desk:eng", name: "Engineering", tagline: "", color: "var(--accent)" },
  { deptId: "desk:gtm", teamId: "team:desk:gtm", name: "Go-to-Market", tagline: "", color: "var(--ok)" },
];

function render() {
  act(() => {
    root.render(
      createElement(KnowledgeGraphFullscreen, {
        deptList: DESKS,
        currentTeamId: DESKS[0].teamId,
        currentDept: DESKS[0],
        toolWiki: null,
        legendSlot: createElement("div", null, "Notes Human AI teammate Tool Workflow Stage SOP task flow placement"),
        onNavDept: () => {},
        onBack: () => {},
        children: createElement("svg"),
      }),
    );
  });
}

function paddle(label: string): HTMLElement {
  const el = host.querySelector(`[aria-label="${label}"]`);
  expect(el, `${label} paddle must render`).not.toBeNull();
  return el as HTMLElement;
}

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

describe("the fullscreen graph's narrow chrome", () => {
  it("keeps the legend within its canvas at 390px and 768px", () => {
    render();
    const legend = host.querySelector('[data-testid="kg-legend"]');
    expect(legend).not.toBeNull();
    expect(legend!.className).toContain("max-w-[calc(100%-1.5rem)]");
    expect(legend!.className).toContain("sm:max-w-[calc(100%-2.5rem)]");
  });

  it("shrinks and insets both paddles below 900px, then further at 390px", () => {
    render();
    for (const label of ["Previous desk", "Next desk"]) {
      const classes = paddle(label).className;
      expect(classes).toContain("max-[899px]:h-20");
      expect(classes).toContain("max-[899px]:w-10");
      expect(classes).toContain("max-[639px]:h-14");
      expect(classes).toContain("max-[639px]:w-8");
    }
    expect(paddle("Previous desk").className).toContain("max-[899px]:left-3");
    expect(paddle("Next desk").className).toContain("max-[899px]:right-3");
  });
});
