// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/views/overview/kg/KnowledgeGraphFullscreen", () => ({
  KnowledgeGraphFullscreen: ({ children }: { children: ReactNode }) => children,
}));

import { KnowledgeGraph } from "@/views/overview/kg/KnowledgeGraph";

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

describe("the overview graph keyboard control", () => {
  it("offers one tab stop and names each interactive node", () => {
    act(() => {
      root.render(
        createElement(KnowledgeGraph, {
          graph: {
            nodes: [
              { id: "self", kind: "self", label: "Acme", ring: 0 },
              { id: "team:desk:eng", kind: "team", label: "Engineering", ring: 1 },
            ],
            edges: [{ source: "self", target: "team:desk:eng", kind: "pillar" }],
          },
        }),
      );
    });

    const graph = host.querySelector('svg[role="application"]');
    expect(graph?.getAttribute("aria-label")).toContain("arrow keys");

    const nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
    expect(nodes).toHaveLength(2);
    expect(nodes.filter((node) => node.tabIndex === 0)).toHaveLength(1);
    expect(nodes.map((node) => node.getAttribute("aria-label"))).toEqual([
      "Notes: Acme. Press Enter or Space to select.",
      "Pillars: Engineering. Press Enter or Space to select.",
    ]);

    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(nodes[1].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[1]);
  });

  it("walks self → its memory notes → the departments once the vault is open", () => {
    act(() => {
      root.render(
        createElement(KnowledgeGraph, {
          graph: {
            nodes: [
              { id: "self", kind: "self", label: "Acme", ring: 0 },
              { id: "team:desk:eng", kind: "team", label: "Engineering", ring: 1 },
            ],
            edges: [{ source: "self", target: "team:desk:eng", kind: "pillar" }],
          },
          // A real vault: one page, one folder hub. Only the opened core exposes
          // its notes as buttons, so a collapsed vault must not add tab stops.
          memory: {
            nodes: [
              {
                id: "note:onboarding", type: "page", label: "Onboarding", folder: "ops",
                excerpt: "", wordCount: 12, chunks: 2, vx: 0.2, vy: -0.2, cluster: 0, links: 1,
              },
              {
                id: "note:vault", type: "folder", label: "Vault", folder: "",
                excerpt: "", wordCount: 0, chunks: 1, vx: -0.3, vy: 0.1, cluster: 1, links: 2,
              },
            ],
            edges: [{ source: "note:onboarding", target: "note:vault", type: "similar" }],
          },
        }),
      );
    });

    // Collapsed, the notes are backdrop for the core's single click target:
    // the same two tab stops as without a vault.
    let nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
    expect(nodes).toHaveLength(2);

    // Opening the vault from the keyboard: Enter on the core dives in.
    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
    expect(nodes).toHaveLength(4);
    expect(nodes.filter((node) => node.tabIndex === 0)).toHaveLength(1);
    expect(nodes.map((node) => node.getAttribute("aria-label"))).toEqual([
      "Notes: Acme. Press Enter or Space to select.",
      "Memory note: Onboarding. Press Enter or Space to open.",
      "Memory note: Vault. Press Enter or Space to open.",
      "Pillars: Engineering. Press Enter or Space to select.",
    ]);

    // ArrowRight walks self → first note → second note → the department,
    // not self → the department with the vault skipped.
    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(nodes[1].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[1]);

    act(() => {
      nodes[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(nodes[2].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[2]);

    act(() => {
      nodes[2].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(nodes[3].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[3]);

    // The walk is a ring: ArrowLeft from the department returns to the vault.
    act(() => {
      nodes[3].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    });
    expect(nodes[2].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[2]);
  });
});
