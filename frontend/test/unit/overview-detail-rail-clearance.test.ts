// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { DeptLite } from "@/views/overview/kg/KnowledgeDetail";
import { KnowledgeGraphFullscreen } from "@/views/overview/kg/KnowledgeGraphFullscreen";

/**
 * Issue #1307: the detail rail buried the chrome on the right edge.
 *
 * The rail is an absolute `z-30` overlay 300px wide, on purpose — resizing the
 * canvas under it reflowed the graph on every open and close. The two controls
 * that live on the right edge were left at `right-2` / `right-3` and `z-20` /
 * `z-10` underneath it, so opening any node's card hid the "Next desk"
 * paddle, the snapshot clock, the Refresh button and the total-outage alert,
 * and made every one of them unclickable behind an opaque panel.
 *
 * What makes this worth a test rather than a fix is that the shell already
 * *claimed* the behaviour — "The right paddle steps aside when the detail panel
 * is open" sat in a comment above a paddle nailed to `right-2` with nothing
 * implementing it. A sentence is not a guard.
 *
 * jsdom does no layout, so this asserts the class contract rather than
 * measured geometry: with a card open, both controls carry the offset that
 * clears the rail's width; with no card open, neither does.
 */

let host: HTMLElement;
let root: Root;

const RAIL_CLEARANCE = "right-[316px]";

const DESKS: DeptLite[] = [
  { deptId: "desk:eng", teamId: "team:desk:eng", name: "Engineering", tagline: "", color: "var(--accent)" },
  { deptId: "desk:gtm", teamId: "team:desk:gtm", name: "Go-to-Market", tagline: "", color: "var(--ok)" },
];

/** The shell, with or without a detail card in the rail. */
function render(detail: boolean) {
  act(() => {
    root.render(
      createElement(KnowledgeGraphFullscreen, {
        deptList: DESKS,
        currentTeamId: DESKS[0].teamId,
        currentDept: DESKS[0],
        toolWiki: null,
        extraDetail: detail ? createElement("div", { "data-testid": "card" }, "a card") : undefined,
        statusSlot: createElement("div", { "data-testid": "snapshot" }, "Snapshot 09:41"),
        onNavDept: () => {},
        onBack: () => {},
        // `children` is a required prop here, not a JSX convenience — passing
        // it positionally to `createElement` satisfies React and not the type.
        children: createElement("svg"),
      }),
    );
  });
}

/** The element that positions the snapshot line — the slot's parent. */
function statusColumn(): HTMLElement {
  const slot = host.querySelector('[data-testid="snapshot"]');
  expect(slot, "the shell must render the status slot it is given").not.toBeNull();
  return slot!.parentElement as HTMLElement;
}

function nextDesk(): HTMLElement {
  const el = host.querySelector('[aria-label="Next desk"]');
  expect(el).not.toBeNull();
  return el as HTMLElement;
}

function paddle(direction: "Previous" | "Next"): HTMLButtonElement {
  const el = host.querySelector(`[aria-label="${direction} desk"]`);
  expect(el).not.toBeNull();
  return el as HTMLButtonElement;
}

beforeEach(() => {
  // React only treats `act` as authoritative when the environment says so;
  // without this every render logs "not configured to support act(...)".
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("the right-edge chrome and the detail rail", () => {
  it("uses desk wording consistently in paddle names and tooltips", () => {
    render(false);
    expect(paddle("Previous").title).toBe("Previous desk (←)");
    expect(paddle("Next").title).toBe("Next desk (→)");
  });

  it("sits at the canvas edge while no card is open", () => {
    render(false);
    expect(statusColumn().className).toContain("right-5");
    expect(statusColumn().className).not.toContain(RAIL_CLEARANCE);
    expect(nextDesk().className).toContain("right-2");
    expect(nextDesk().className).not.toContain(RAIL_CLEARANCE);
  });

  it("steps clear of the rail when a card opens", () => {
    render(true);
    expect(host.querySelector('[data-testid="card"]')).not.toBeNull();
    expect(statusColumn().className).toContain(RAIL_CLEARANCE);
    expect(nextDesk().className).toContain(RAIL_CLEARANCE);
  });

  it("stays above the rail rather than merely beside it", () => {
    // Belt and braces: the offset is what makes them visible, but the rail is
    // `z-30` and both of these used to be below it. If a future change moves
    // the rail rather than the controls, the stacking order must not be the
    // thing that re-buries them.
    render(true);
    expect(statusColumn().className).toContain("z-40");
    expect(nextDesk().className).toContain("z-40");
  });

  it("returns to the edge when the card closes", () => {
    render(true);
    render(false);
    expect(statusColumn().className).not.toContain(RAIL_CLEARANCE);
    expect(nextDesk().className).not.toContain(RAIL_CLEARANCE);
  });
});
