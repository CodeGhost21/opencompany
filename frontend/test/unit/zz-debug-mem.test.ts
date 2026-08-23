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

describe("debug memory notes", () => {
  it("dumps state", () => {
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
          memory: {
            nodes: [
              { id: "note:onboarding", type: "page", label: "Onboarding", folder: "ops", excerpt: "", wordCount: 12, chunks: 2, vx: 0.2, vy: -0.2, cluster: 0, links: 1 },
              { id: "note:vault", type: "folder", label: "Vault", folder: "", excerpt: "", wordCount: 0, chunks: 1, vx: -0.3, vy: 0.1, cluster: 1, links: 2 },
            ],
            edges: [{ source: "note:onboarding", target: "note:vault", type: "similar" }],
          },
        }),
      );
    });
    const dump = (label: string) => {
      const nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
      console.log(label, JSON.stringify(nodes.map((n) => ({ label: n.getAttribute("aria-label"), tab: n.tabIndex, cls: n.getAttribute("class") }))));
      console.log("active:", document.activeElement?.getAttribute("aria-label"), "tag:", document.activeElement?.tagName);
    };
    dump("initial");
    const nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    dump("after enter-expand");
    const nodes2 = [...host.querySelectorAll<SVGGElement>('g[role="button"]')];
    act(() => {
      nodes2[0].focus();
      nodes2[0].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    dump("after arrow-right");
  });
});
