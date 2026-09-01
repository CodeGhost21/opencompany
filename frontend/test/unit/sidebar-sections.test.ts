// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import {
  NAV_SECTIONS,
  SidebarNavigation,
  childActive,
  sectionOwning,
  type NavSection,
} from "@/components/sidebar-navigation";
import { SidebarProvider } from "@/components/ui/sidebar";
import type { View } from "@/lib/console-routes";

/**
 * The sidebar is sections with sub-navigation under them, not a flat list.
 *
 * These assertions are written to survive mutation, which for a nav table means
 * two specific traps. `toContain('{ view: "x" }')` is satisfied by a **commented
 * out** row that renders nothing — the exact shape that left `#/pages`
 * unreachable for four months (issue #1311) — so nothing here reads source text.
 * And a label assertion that only checks membership is satisfied by a row that
 * is also still somewhere else, so the tables are compared whole and in order.
 */

let container: HTMLDivElement;
let root: Root;

function render(view: View, sub: string | null = null) {
  act(() =>
    root.render(
      createElement(
        SidebarProvider,
        null,
        createElement(SidebarNavigation, { view, sub, onNavigate: () => {} }),
      ),
    ),
  );
}

/** Every row label the sidebar actually paints, parents and children alike. */
function renderedRows(): string[] {
  return [...container.querySelectorAll("[data-sidebar='menu-button'], [data-sidebar='menu-sub-button']")].map(
    (el) => el.textContent?.trim() ?? "",
  );
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom ships no `matchMedia`, and `SidebarProvider` asks it whether this is a
  // phone. Answer "no" — the sub-navigation this file is about is the desktop
  // column, not the mobile sheet.
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the sidebar's section table", () => {
  it("is exactly these top-level rows, in this order", () => {
    expect(NAV_SECTIONS.map((section) => section.label)).toEqual([
      "Overview",
      "Company",
      "Chat",
      "Approvals",
      "Workflows",
      "Observatory",
    ]);
  });

  it("files the company's five surfaces under Company, in this order", () => {
    const company = NAV_SECTIONS.find((section) => section.label === "Company")!;
    expect(company.children?.map((child) => [child.label, child.view])).toEqual([
      ["Agents", "company"],
      ["Work", "ledgers"],
      ["Workspace", "workspace"],
      ["Brain", "brain"],
      ["Finance", "finances"],
    ]);
  });

  it("gives every section a distinct view, so two rows can never light at once", () => {
    const views = NAV_SECTIONS.map((section) => section.view);
    expect(new Set(views).size).toBe(views.length);
  });
});

describe("which section an address belongs to", () => {
  it("claims the deep-link surfaces that have no row of their own", () => {
    // A task card is a card on Work's board; a teammate is a seat on the org
    // chart; a desk transcript is the surface Chat replaces. Each keeps its
    // section open rather than emptying the sidebar.
    expect(sectionOwning("tasks")?.label).toBe("Company");
    expect(sectionOwning("team")?.label).toBe("Company");
    expect(sectionOwning("conversation")?.label).toBe("Chat");
  });

  it("claims a section's children for that section", () => {
    for (const view of ["company", "ledgers", "workspace", "brain", "finances"] as View[]) {
      expect(sectionOwning(view)?.label).toBe("Company");
    }
  });

  it("claims nothing for the surfaces that are deliberately not in the nav", () => {
    // Settings and Feedback live in the sidebar's footer; Pages is direct-URL
    // only (#1171, #1172); `not-found` is nowhere by design.
    for (const view of ["settings", "feedback", "pages", "not-found"] as View[]) {
      expect(sectionOwning(view)).toBeUndefined();
    }
  });
});

describe("which child row is open", () => {
  const company = NAV_SECTIONS.find((section) => section.label === "Company") as NavSection;
  const child = (label: string) => company.children!.find((c) => c.label === label)!;

  it("keeps a child lit across every sub-page of its own view", () => {
    // `#/ledgers/goals` is a declared list on the same Work surface.
    expect(childActive(company, child("Work"), "ledgers", "goals")).toBe(true);
    expect(childActive(company, child("Workspace"), "workspace", "node-7")).toBe(true);
  });

  it("lights exactly one child per address", () => {
    const lit = company.children!.filter((c) => childActive(company, c, "brain", null));
    expect(lit.map((c) => c.label)).toEqual(["Brain"]);
  });

  it("lights the child a deep-link surface belongs to", () => {
    expect(childActive(company, child("Work"), "tasks", "task-1")).toBe(true);
    expect(childActive(company, child("Agents"), "team", "agent-1")).toBe(true);
  });
});

describe("the rendered sidebar", () => {
  it("shows a section's contents only while that section is the one you are in", () => {
    render("company");
    expect(renderedRows()).toEqual([
      "Overview",
      "Company",
      "Agents",
      "Work",
      "Workspace",
      "Brain",
      "Finance",
      "Chat",
      "Approvals",
      "Workflows",
      "Observatory",
    ]);

    render("chat");
    expect(renderedRows()).toEqual([
      "Overview",
      "Company",
      "Chat",
      "Approvals",
      "Workflows",
      "Observatory",
    ]);
  });

  it("marks the open child as the current page and leaves the parent unlit", () => {
    render("workspace");
    const current = [...container.querySelectorAll('[aria-current="page"]')].map(
      (el) => el.textContent?.trim(),
    );
    expect(current).toEqual(["Workspace"]);

    const parent = [...container.querySelectorAll("[data-sidebar='menu-button']")].find(
      (el) => el.textContent?.trim() === "Company",
    )!;
    expect(parent.hasAttribute("data-active")).toBe(false);
    // Still visibly the section you are in, through the resting dim rather than
    // through a second accent fill.
    expect(parent.hasAttribute("data-section-active")).toBe(true);
  });

  it("lights a section's own row when nothing under it is open", () => {
    render("chat");
    const row = [...container.querySelectorAll("[data-sidebar='menu-button']")].find(
      (el) => el.textContent?.trim() === "Chat",
    )!;
    expect(row.hasAttribute("data-active")).toBe(true);
  });

  it("puts no heading in the sidebar, at runtime and not only in source", () => {
    // The complement of `nav-rail-headings.test.ts`, which is a source guard and
    // cannot see what a portal or a child component contributes (issue #1392).
    render("company");
    expect(container.querySelectorAll("h1, h2, h3, h4, h5, h6")).toHaveLength(0);
  });
});
