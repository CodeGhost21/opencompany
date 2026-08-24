// @vitest-environment jsdom

import { act, createElement, Fragment, useEffect, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/views/overview/kg/KnowledgeGraphFullscreen", () => ({
  KnowledgeGraphFullscreen: ({
    children,
    extraDetail,
    coreOpen,
    onCollapseCore,
  }: {
    children: ReactNode;
    extraDetail?: ReactNode;
    coreOpen?: boolean;
    onCollapseCore?: () => void;
  }) => {
    // The real fullscreen chrome owns Escape while the vault is open: it
    // collapses the core instead of exiting. The mock must keep that single
    // contract so tests can drive the collapse the way a user does.
    useEffect(() => {
      const onKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") onCollapseCore?.();
      };
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, [coreOpen, onCollapseCore]);
    return createElement(Fragment, null, children, extraDetail);
  },
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
      "Desks: Engineering. Press Enter or Space to select.",
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

    // With the vault open the core sheds its button role (its notes are the
    // interactive targets now), so the roving set spans buttons + the group.
    nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    expect(nodes).toHaveLength(4);
    expect(nodes[0].getAttribute("role")).toBe("group");
    expect(nodes.filter((node) => node.tabIndex === 0)).toHaveLength(1);
    expect(nodes.map((node) => node.getAttribute("aria-label"))).toEqual([
      "Notes: Acme. 2 notes.",
      "Memory note: Onboarding. Press Enter or Space to open.",
      "Memory note: Vault. Press Enter or Space to open.",
      "Desks: Engineering. Press Enter or Space to select.",
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

  it("opens a memory note's detail card when Enter is pressed on it", () => {
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
              {
                id: "note:onboarding", type: "page", label: "Onboarding", folder: "ops",
                excerpt: "Welcome aboard", wordCount: 12, chunks: 2, vx: 0.2, vy: -0.2, cluster: 0, links: 1,
              },
            ],
            edges: [],
          },
        }),
      );
    });

    // Open the vault from the keyboard, exposing the note as a button.
    let nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    expect(nodes).toHaveLength(3);
    expect(nodes[1].getAttribute("aria-label")).toBe(
      "Memory note: Onboarding. Press Enter or Space to open.",
    );

    // The aria label promises opening — the detail card is not up yet.
    expect(host.textContent).not.toContain("Welcome aboard");

    // Enter on the note opens its detail card (the excerpt is card-only text).
    act(() => {
      nodes[1].focus();
      nodes[1].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(host.textContent).toContain("Welcome aboard");
  });

  it("moves focus to the core when Escape collapses the vault under a focused note", async () => {
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
              {
                id: "note:onboarding", type: "page", label: "Onboarding", folder: "ops",
                excerpt: "", wordCount: 12, chunks: 2, vx: 0.2, vy: -0.2, cluster: 0, links: 1,
              },
            ],
            edges: [],
          },
        }),
      );
    });

    let nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    act(() => {
      nodes[0].focus();
      nodes[0].dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    // Park the roving focus on the note, then Escape collapses the vault and
    // unmounts it — focus must land back on the core, not <body>.
    nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    act(() => {
      nodes[1].focus();
    });
    expect(document.activeElement).toBe(nodes[1]);

    // Escape in the fullscreen chrome collapses the vault (the mock keeps the
    // real contract), stranding focus on the just-removed note.
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });

    nodes = [...host.querySelectorAll<SVGGElement>('g[role="button"], g[role="group"]')];
    expect(nodes).toHaveLength(2);
    expect(nodes[0].getAttribute("role")).toBe("button");
    expect(nodes[0].tabIndex).toBe(0);
    expect(document.activeElement).toBe(nodes[0]);
  });
});
