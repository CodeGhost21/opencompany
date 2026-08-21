// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { WorkflowNode } from "@/api/workflows";
import { NodeDetailPanel } from "@/views/workflows/NodeDetailPanel";

/**
 * #850 review finding: a node whose only declared detail is
 * `repeatable: false` rendered the "not repeated on approval" badge AND the
 * "This node has no extra details…" empty state at the same time, because the
 * empty-state predicate never checked `repeatable`. The badge is a real
 * detail — the empty state must not also claim there is none.
 */

let container: HTMLDivElement;
let root: Root;

function render(node: WorkflowNode) {
  act(() => {
    root.render(createElement(NodeDetailPanel, { node, onClose: () => {} }));
  });
}

function baseNode(fields: Partial<WorkflowNode> = {}): WorkflowNode {
  return {
    id: "publish",
    kind: "tool_call",
    name: "Publish",
    ...fields,
  };
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("NodeDetailPanel — repeatable and the empty-details state", () => {
  it("does not show the empty-details message for a node whose only detail is repeatable:false", () => {
    render(baseNode({ repeatable: false }));
    expect(container.querySelector('[data-testid="node-not-repeated"]')).not.toBeNull();
    expect(container.textContent).not.toContain("This node has no extra details");
  });

  it("still shows the empty-details message for a node with genuinely no details", () => {
    render(baseNode());
    expect(container.querySelector('[data-testid="node-not-repeated"]')).toBeNull();
    expect(container.textContent).toContain("This node has no extra details");
  });

  it("does not show the empty-details message when another field also carries a detail", () => {
    render(baseNode({ repeatable: false, onError: "continue" }));
    expect(container.textContent).not.toContain("This node has no extra details");
  });
});
