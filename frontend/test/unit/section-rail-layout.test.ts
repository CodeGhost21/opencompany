// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SectionContentRail } from "@/components/section-rail";
import { grandchildActive, NAV_SECTIONS } from "@/components/sidebar-navigation";
import type { View } from "@/lib/console-routes";

/**
 * A section's sub-navigation is the first column of its content area (#2130).
 *
 * The sidebar's middle region is the Room rail on every section now, so the rows
 * that used to live under a section's sidebar entry are drawn here instead. What
 * is worth pinning is what an operator can actually reach and what a spec can
 * actually click:
 *
 *   - the rows are the section's `children`, whole and in order, so a row that
 *     moved out of the sidebar did not go missing on the way;
 *   - a section with no children draws no rail at all, so Room and Flows keep
 *     their full pane;
 *   - there is never more than ONE rail on screen, which is the whole of the
 *     Finance decision and of issue #1383;
 *   - the `data-tour` anchors travelled with the rows. They are how the guided
 *     tour and `list-switcher.spec.ts` find a row, and they are deliberately
 *     pinned to view ids rather than labels — a row that moved surface and
 *     dropped its anchor is a silently skipped tour stop and a spec that clicks
 *     nothing.
 */

let container: HTMLDivElement;
let root: Root;

function render(view: View, sub: string | null = null, onNavigate = () => {}) {
  act(() =>
    root.render(
      createElement(SectionContentRail, {
        view,
        sub,
        onNavigate,
        children: createElement("main", { "data-testid": "page" }, "the page"),
      }),
    ),
  );
}

/** The rail's own rows, in document order — the `lg` column, not the chips. */
function railRows(): string[] {
  const nav = container.querySelector("nav");
  if (!nav) return [];
  return [...nav.querySelectorAll("button")].map((el) =>
    (el.querySelector("span > span")?.textContent ?? "").trim(),
  );
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

describe("which sections get a rail", () => {
  it("draws one for Company, with its five pages whole and in order", () => {
    render("company");
    expect(railRows()).toEqual(["Agents", "Work", "Workspace", "Brain", "Finance"]);
    expect(container.querySelector("nav")?.getAttribute("aria-label")).toBe("Company");
  });

  it("draws one for Connections, with both of its pages", () => {
    render("connections", "mcp");
    expect(railRows()).toEqual(["Apps", "MCP Servers"]);
  });

  it("draws none for Room or Flows, so their pane keeps its full width", () => {
    // Room's sub-navigation is the channel list, which is pinned in the sidebar;
    // Flows has none to move. A rail here would be 240px charged for nothing.
    for (const view of ["chat", "workflows"] as View[]) {
      render(view);
      expect(container.querySelector("nav"), view).toBeNull();
      expect(container.querySelector("[data-testid='page']"), view).not.toBeNull();
    }
  });

  it("draws none for an address filed under no section", () => {
    // Settings and Feedback are footer utilities, Overview and Approvals are
    // chrome in the title row. Settings draws a rail of its own; nothing here
    // should draw a second one over it.
    for (const view of ["settings", "overview", "approvals", "not-found"] as View[]) {
      render(view);
      expect(container.querySelector("nav"), view).toBeNull();
    }
  });

  it("renders the page in every case, rail or no rail", () => {
    for (const view of ["chat", "company", "connections", "workflows", "settings"] as View[]) {
      render(view);
      expect(container.querySelector("[data-testid='page']"), view).not.toBeNull();
    }
  });
});

describe("never two rails at once", () => {
  it("keeps Finance's pages on Company's rail rather than giving them a second", () => {
    // The Finance decision, asserted as the property it is for rather than as a
    // layout preference: sidebar + 240 + 240 + content is the 768–1023px band
    // of issue #1383 reproduced at every width. One rail per section, always.
    render("finances", "wallet");
    expect(container.querySelectorAll("nav")).toHaveLength(1);
    expect(railRows()).toEqual([
      "Agents",
      "Work",
      "Workspace",
      "Brain",
      "Finance",
      "Overview",
      "Invoicing",
      "Wallet",
    ]);
  });

  it("shows a nested page only while its parent is the open row", () => {
    render("company");
    expect(railRows()).not.toContain("Wallet");
    render("brain");
    expect(railRows()).not.toContain("Wallet");
  });

  it("marks exactly one row current, at whichever depth it is", () => {
    render("finances", "invoicing");
    const current = [...container.querySelectorAll('nav [aria-current="page"]')].map((el) =>
      (el.querySelector("span > span")?.textContent ?? "").trim(),
    );
    // Finance is the section row you are in AND Invoicing is the page — both
    // light, which is the same two-register marking the sidebar used to do with
    // its section row and its child row.
    expect(current).toEqual(["Finance", "Invoicing"]);
  });
});

describe("the tour anchors travelled with the rows", () => {
  it("keeps one node per anchor, and keeps the ones specs click", () => {
    render("company");
    const anchors = [...container.querySelectorAll("[data-tour]")].map((el) =>
      el.getAttribute("data-tour"),
    );
    expect(new Set(anchors).size, anchors.join(", ")).toBe(anchors.length);
    // `list-switcher.spec.ts` clicks this one from `#/company`.
    expect(anchors).toContain("nav-ledgers");
    // And it is a button inside the anchor, the shape every selector in the
    // e2e suite is written as (`[data-tour="nav-x"] >> role=button`).
    expect(
      container.querySelector('[data-tour="nav-ledgers"] button')?.textContent,
    ).toContain("Work");
  });

  it("gives the row that shares its section's address no anchor of its own", () => {
    // Agents *is* `#/company`, so an anchor here would put two `nav-company`
    // nodes on screen and every selector written against it becomes a
    // strict-mode violation rather than a click. The sidebar row keeps the name.
    render("company");
    expect(container.querySelectorAll('[data-tour="nav-company"]')).toHaveLength(0);
  });

  it("gives a nested row no anchor, since it would collide with its parent's", () => {
    render("finances");
    const anchors = [...container.querySelectorAll("[data-tour]")].map((el) =>
      el.getAttribute("data-tour"),
    );
    expect(anchors.filter((a) => a === "nav-finances")).toHaveLength(1);
  });
});

describe("clicking a row", () => {
  it("navigates by (view, sub), the same pair the sidebar rows used", () => {
    const onNavigate = vi.fn();
    render("company", null, onNavigate);
    act(() => {
      container
        .querySelector<HTMLButtonElement>('[data-tour="nav-workspace"] button')!
        .click();
    });
    expect(onNavigate).toHaveBeenCalledWith("workspace", undefined);
  });

  it("navigates a nested page to its own segment", () => {
    const onNavigate = vi.fn();
    render("finances", null, onNavigate);
    const wallet = [...container.querySelectorAll<HTMLButtonElement>("nav button")].find((el) =>
      el.textContent?.includes("Wallet"),
    )!;
    act(() => wallet.click());
    expect(onNavigate).toHaveBeenCalledWith("finances", "wallet");
  });
});

describe("grandchildActive", () => {
  const finance = NAV_SECTIONS.find((s) => s.view === "company")!.children!.find(
    (c) => c.view === "finances",
  )!;
  const page = (label: string) => finance.children!.find((c) => c.label === label)!;

  it("lights the first page for the bare address, as the sections do", () => {
    // `#/finances` is Overview for the same reason `#/connections` is Apps: the
    // parent row lands on the bare address and the first page is what it shows.
    expect(grandchildActive(finance, page("Overview"), "finances", null)).toBe(true);
    expect(grandchildActive(finance, page("Wallet"), "finances", null)).toBe(false);
  });

  it("lights the page the segment names", () => {
    expect(grandchildActive(finance, page("Wallet"), "finances", "wallet")).toBe(true);
    expect(grandchildActive(finance, page("Overview"), "finances", "wallet")).toBe(false);
  });

  it("lights nothing outside its own view", () => {
    expect(grandchildActive(finance, page("Overview"), "brain", null)).toBe(false);
  });
});
