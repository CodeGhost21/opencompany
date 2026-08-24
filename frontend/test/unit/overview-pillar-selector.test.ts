// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { DeptLite } from "@/views/overview/kg/KnowledgeDetail";
import { KnowledgeGraphFullscreen } from "@/views/overview/kg/KnowledgeGraphFullscreen";

/**
 * Issue #1309: the desk selector would not say which desk was which.
 *
 * It drew one 10px dot per desk at 50% opacity under the words "Pick a
 * desk", and each desk's name existed only in that dot's `title` — a hover
 * away on a pointer and unreachable on touch. So the control whose whole
 * purpose is choosing a desk named none of them at rest, while the graph
 * labelled all of them in their own colours a few inches away. You had to
 * click a blind dot to learn what it was.
 *
 * The caption is also under test, because "Pick a desk / 0 DESKS" was an
 * imperative the page made impossible to follow on a company that has not
 * seated a desk yet.
 */

let host: HTMLElement;
let root: Root;

const DESKS: DeptLite[] = [
  { deptId: "desk:eng", teamId: "team:desk:eng", name: "Engineering", tagline: "", color: "var(--chart-1)" },
  { deptId: "desk:pd", teamId: "team:desk:pd", name: "Product & Design", tagline: "", color: "var(--chart-2)" },
  { deptId: "desk:gtm", teamId: "team:desk:gtm", name: "Go-to-Market", tagline: "", color: "var(--chart-3)" },
];

function render(
  deptList: DeptLite[],
  currentTeamId: string | null,
  onNavDept: (id: string) => void = () => {},
  emptyState = false,
) {
  const currentDept = deptList.find((d) => d.teamId === currentTeamId) ?? null;
  act(() => {
    root.render(
      createElement(KnowledgeGraphFullscreen, {
        deptList,
        currentTeamId,
        currentDept,
        toolWiki: null,
        emptyState,
        onNavDept,
        onBack: () => {},
        // `children` is a required prop here, not a JSX convenience — passing
        // it positionally to `createElement` satisfies React and not the type.
        children: createElement("svg"),
      }),
    );
  });
}

/** The desk chips, in the order they are drawn. */
function chips(): HTMLButtonElement[] {
  return [...host.querySelectorAll("button")].filter((b) =>
    (b.getAttribute("title") ?? "").includes("bring this desk forward"),
  ) as HTMLButtonElement[];
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

describe("the desk selector", () => {
  it("names every desk, at rest, with nothing focused", () => {
    render(DESKS, null);
    expect(host.textContent).toContain("Desks");
    expect(chips().map((b) => b.textContent)).toEqual([
      "Engineering",
      "Product & Design",
      "Go-to-Market",
    ]);
  });

  it("names them in the host's own desk order", () => {
    // The order is the company's, never re-sorted here — the same rule ring 1
    // follows. A selector that alphabetised would disagree with the wheel.
    render([...DESKS].reverse(), null);
    expect(chips().map((b) => b.textContent)).toEqual([
      "Go-to-Market",
      "Product & Design",
      "Engineering",
    ]);
  });

  it("marks the focused desk, and only that one", () => {
    render(DESKS, "team:desk:pd");
    expect(chips().map((b) => b.getAttribute("aria-current"))).toEqual([null, "true", null]);
  });

  it("marks nothing when the wheel is at rest", () => {
    render(DESKS, null);
    expect(chips().every((b) => b.getAttribute("aria-current") === null)).toBe(true);
  });

  it("brings a desk forward when its chip is clicked", () => {
    const seen: string[] = [];
    render(DESKS, null, (id) => seen.push(id));
    act(() => {
      chips()[2].dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(seen).toEqual(["team:desk:gtm"]);
  });

  it("calls the action a desk action in each chip tooltip", () => {
    render(DESKS, null);
    expect(chips().map((b) => b.title)).toEqual([
      "Engineering — bring this desk forward",
      "Product & Design — bring this desk forward",
      "Go-to-Market — bring this desk forward",
    ]);
  });

  it("says a company has no desks rather than instructing it to pick one", () => {
    // "Pick a desk / 0 DESKS" asked for something impossible, next to
    // nothing to pick — which is the first screen a newly provisioned company
    // sees.
    render([], null);
    expect(chips()).toHaveLength(0);
    expect(host.textContent).toContain("No desks yet");
    expect(host.textContent).not.toContain("Pick a desk");
  });

  it("explains a loaded empty overview and sends the operator to create a desk", () => {
    render([], null, () => {}, true);

    expect(host.textContent).toContain("This graph shows how your company's desks, teammates, work, and workflows connect.");
    const createDesk = host.querySelector('a[href="#/company/desks"]');
    expect(createDesk?.textContent).toBe("Create a desk");
    expect(host.querySelector('[aria-label="Previous department"]')).toBeNull();
    expect(host.querySelector('[aria-label="Next department"]')).toBeNull();
    expect(host.querySelector("svg")).toBeNull();
  });
});
