// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

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

/** Every row label the sidebar actually paints, in document order. */
function renderedRows(): string[] {
  return [...container.querySelectorAll("[data-sidebar='menu-button']")].map(
    (el) => el.textContent?.trim() ?? "",
  );
}

/** The four fixed rows, read from the group that never changes. */
function fixedRows(): string[] {
  const group = container.querySelectorAll("[data-sidebar='group']")[0];
  return [...group.querySelectorAll("[data-sidebar='menu-button']")].map(
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
      "Room",
      "Company",
      "Connections",
      "Flows",
    ]);
  });

  it("keeps the surfaces that lost their row out of the table entirely", () => {
    // Overview and Approvals moved up into the window's title row; Observatory
    // moved down into Settings. A row left behind as a comment is the failure
    // this codebase has already had once (#1311), so the assertion above is a
    // whole-table equality — a commented row is not a member of it — and this
    // one says the same thing from the other side.
    const views = NAV_SECTIONS.map((section) => section.view as string);
    expect(views).not.toContain("overview");
    expect(views).not.toContain("approvals");
    expect(views).not.toContain("observatory");
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

  it("calls the workflow surface Flows, over the view id every address uses", () => {
    // A view id is an address — every `#/workflows/<id>` a run row points at —
    // and renaming a row is not a reason to break them. "Work" has been the
    // `ledgers` view since #1284 for the same reason. The `data-tour` anchors
    // follow the view id, so the tour and the e2e specs do not move when a word
    // does.
    const flows = NAV_SECTIONS.find((section) => section.label === "Flows")!;
    expect(flows.view).toBe("workflows");

    const room = NAV_SECTIONS.find((section) => section.label === "Room")!;
    expect(room.view).toBe("chat");
  });

  it("keeps the Agents row and the page it reaches called the same thing", () => {
    // A row named one thing leading to a page headed another is the reader's
    // problem, not the code's. "Company > Company" said the word twice and
    // named nothing; both halves are Agents now.
    const company = NAV_SECTIONS.find((section) => section.label === "Company")!;
    expect(company.children?.[0].label).toBe("Agents");
    const source = readFileSync(
      resolve(dirname(fileURLToPath(import.meta.url)), "../../src/views/TeamView.tsx"),
      "utf8",
    );
    expect(source).toContain('title="Agents"');
    expect(source).not.toContain('title="Company"');
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
    expect(sectionOwning("conversation")?.label).toBe("Room");
  });

  it("claims a section's children for that section", () => {
    for (const view of ["company", "ledgers", "workspace", "brain", "finances"] as View[]) {
      expect(sectionOwning(view)?.label).toBe("Company");
    }
  });

  it("claims Connections for both of its sub-pages", () => {
    // Its children share the parent's view and differ only by hash segment, so
    // the section is claimed by the view and the child by the segment.
    expect(sectionOwning("connections")?.label).toBe("Connections");
  });

  it("claims nothing for the surfaces that are deliberately not in the nav", () => {
    // Settings and Feedback live in the sidebar's footer; Overview and
    // Approvals in the window's title row; Observatory under Settings; Pages is
    // direct-URL only (#1171, #1172); `not-found` is nowhere by design.
    for (const view of [
      "settings",
      "feedback",
      "pages",
      "overview",
      "approvals",
      "observatory",
      "not-found",
    ] as View[]) {
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
  it("keeps the four rows fixed and swaps only the block beneath them", () => {
    // Not an accordion. The four never move, never reorder and are never
    // displaced by a sibling's contents — so the contents always come AFTER
    // all four, whichever section is open.
    render("company");
    expect(fixedRows()).toEqual(["Room", "Company", "Connections", "Flows"]);
    expect(renderedRows()).toEqual([
      "Room",
      "Company",
      "Connections",
      "Flows",
      "Agents",
      "Work",
      "Workspace",
      "Brain",
      "Finance",
    ]);

    render("connections", "mcp");
    expect(fixedRows()).toEqual(["Room", "Company", "Connections", "Flows"]);
    expect(renderedRows()).toEqual([
      "Room",
      "Company",
      "Connections",
      "Flows",
      "Apps",
      "MCP Servers",
    ]);

    // Exactly one section's contents at a time: Flows has none, so the block
    // is not there at all.
    render("workflows");
    expect(renderedRows()).toEqual(["Room", "Company", "Connections", "Flows"]);
  });

  it("separates the four from what is filed under them with space, not a rule", () => {
    // A horizontal line here reads as hardware bolted across a column that is
    // already quiet, and it would have been the only rule in it — the console
    // draws none above its footer either. The break is a deliberate gap.
    render("company");
    expect(container.querySelectorAll("[data-sidebar='separator']")).toHaveLength(0);

    const groups = [...container.querySelectorAll("[data-sidebar='group']")];
    expect(groups).toHaveLength(2);
    // Deliberate, not the row rhythm: `py-1` on the group either side of it is
    // 4px, and the break has to be legible at a glance.
    expect(groups[1].className).toContain("pt-5");

    // And no contents block at all for a section that has none, so the gap
    // never appears with nothing under it.
    render("workflows");
    expect(container.querySelectorAll("[data-sidebar='group']")).toHaveLength(1);
  });

  it("marks the section in the fixed block and the page in the block below", () => {
    render("workspace");
    // The two say different things and are read in different registers: which
    // of the four you are in, and which of its pages is open.
    expect(fixedRows().filter((_, i) =>
      [...container.querySelectorAll("[data-sidebar='group']")[0]
        .querySelectorAll("[data-sidebar='menu-button']")][i].hasAttribute("data-active"),
    )).toEqual(["Company"]);

    const current = [...container.querySelectorAll('[aria-current="page"]')].map(
      (el) => el.textContent?.trim(),
    );
    expect(current).toEqual(["Workspace"]);
  });

  it("lights a section's own row when nothing under it is open", () => {
    render("chat");
    const row = [...container.querySelectorAll("[data-sidebar='menu-button']")].find(
      (el) => el.textContent?.trim() === "Room",
    )!;
    expect(row.hasAttribute("data-active")).toBe(true);
  });

  it("gives Room a slot for the channel list, and gives it to no one else", () => {
    render("chat");
    expect(container.querySelectorAll("[data-testid='room-rail-slot']")).toHaveLength(1);

    // The slot is the portal target `ChatView` renders into. Absent while
    // another section is open, which is what makes the list Room's contents
    // rather than standing furniture.
    render("company");
    expect(container.querySelectorAll("[data-testid='room-rail-slot']")).toHaveLength(0);
  });

  it("puts no heading in the sidebar, at runtime and not only in source", () => {
    // The complement of `nav-rail-headings.test.ts`, which is a source guard and
    // cannot see what a portal or a child component contributes (issue #1392).
    render("company");
    expect(container.querySelectorAll("h1, h2, h3, h4, h5, h6")).toHaveLength(0);
  });
});
