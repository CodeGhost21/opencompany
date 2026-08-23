// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { KnowledgeGraph } from "@/views/overview/kg/KnowledgeGraph";
import type { KnowledgeGraph as Graph } from "@/views/overview/kg/model";

/**
 * Issue #1312: a task title was shortened to 18 characters before the label
 * planner could measure it. Hover makes a task a top-priority candidate, so
 * this renders the smallest graph that exposes the on-canvas label and guards
 * both the planner input and the text node it ultimately produces.
 */

const TASK_TITLE = "Draft Q3 pricing experiment";

const graph: Graph = {
  nodes: [
    { id: "self", kind: "self", label: "Acme", ring: 0 },
    { id: "team:pricing", kind: "team", label: "Pricing", ring: 1, color: "var(--accent)", order: 0 },
    { id: "task:pricing", kind: "task", label: TASK_TITLE, ring: 2 },
    { id: "emp:operator", kind: "employee", label: "Operator", ring: 3 },
  ],
  edges: [
    { source: "self", target: "team:pricing", kind: "pillar" },
    { source: "team:pricing", target: "task:pricing", kind: "sop" },
    { source: "task:pricing", target: "emp:operator", kind: "does" },
  ],
};

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root.render(createElement(KnowledgeGraph, { graph })));
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

describe("task labels", () => {
  it("renders the complete title when the task is named", async () => {
    // The graph starts its d3 layout after mounting, so wait for its first
    // tick to populate the SVG nodes.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 25));
    });
    const title = [...host.querySelectorAll("title")].find((el) => el.textContent === TASK_TITLE);
    expect(title, "the task node must be rendered").toBeDefined();

    act(() => {
      title!.parentElement!.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
    });

    const labels = [...host.querySelectorAll("text")].map((el) => el.textContent);
    expect(labels).toContain(TASK_TITLE);
    expect(labels).not.toContain("Draft Q3 pricing e…");
  });
});
