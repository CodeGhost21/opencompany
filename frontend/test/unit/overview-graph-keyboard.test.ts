// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/views/overview/kg/KnowledgeGraphFullscreen", () => ({
  KnowledgeGraphFullscreen: ({ children }: { children: React.ReactNode }) => children,
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
  });
});
