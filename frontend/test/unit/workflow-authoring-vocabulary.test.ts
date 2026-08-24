import { describe, expect, it } from "vitest";

import {
  NODE_KINDS,
  nodeKindLabel,
  WORKFLOW_NODE_KINDS,
} from "@/api/workflows";
import {
  edgesConnectingNodesInOrder,
  type DraftEdge,
} from "@/views/WorkflowCreateDialog";

describe("the shared workflow node-kind vocabulary", () => {
  it("backs every accepted host kind, including split_out", () => {
    expect(NODE_KINDS.map((kind) => kind.value)).toEqual(WORKFLOW_NODE_KINDS);
    expect(WORKFLOW_NODE_KINDS).toContain("split_out");
  });

  it("labels known kinds and keeps newer unknown kinds readable", () => {
    expect(nodeKindLabel("tool_call")).toBe("Tool call");
    expect(nodeKindLabel("output_parser")).toBe("Output parser");
    expect(nodeKindLabel("future_kind")).toBe("Future kind");
    expect(nodeKindLabel("")).toBe("Unknown node kind");
  });
});

describe("connect in order", () => {
  const edge = (key: string, from: string, to: string, label = ""): DraftEdge => ({
    key,
    from,
    to,
    label,
  });

  it("adds a deterministic linear chain while preserving explicit branches", () => {
    const nodes = [{ id: "start" }, { id: "draft" }, { id: "publish" }];
    const branch = edge("branch", "start", "publish", "fast path");

    const connected = edgesConnectingNodesInOrder(nodes, [branch]);

    expect(connected.map(({ from, to, label }) => ({ from, to, label }))).toEqual([
      { from: "start", to: "publish", label: "fast path" },
      { from: "start", to: "draft", label: "" },
      { from: "draft", to: "publish", label: "" },
    ]);
  });

  it("does not duplicate an existing pair or add more on a second press", () => {
    const nodes = [{ id: "start" }, { id: "draft" }, { id: "publish" }];
    const existing = [edge("one", "start", "draft", "already connected")];
    const once = edgesConnectingNodesInOrder(nodes, existing);
    const twice = edgesConnectingNodesInOrder(nodes, once);

    expect(twice).toEqual(once);
    expect(twice).toHaveLength(2);
  });

  it("leaves the graph alone when ids are blank or duplicated", () => {
    const existing = [edge("branch", "start", "publish")];
    expect(edgesConnectingNodesInOrder([{ id: "start" }, { id: "" }], existing)).toEqual(
      existing,
    );
    expect(
      edgesConnectingNodesInOrder([{ id: "same" }, { id: "same" }], existing),
    ).toEqual(existing);
  });
});
