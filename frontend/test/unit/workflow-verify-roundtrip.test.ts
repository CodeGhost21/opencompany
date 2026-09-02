import { describe, expect, it } from "vitest";

import type { WorkflowGraph } from "@/api/workflows";
import {
  assembleGraph,
  changeKind,
  draftNodes,
  type DraftNode,
  type GraphDraft,
} from "@/views/WorkflowCreateDialog";

function graph(): WorkflowGraph {
  return {
    id: "verified",
    name: "Verified",
    version: "v1",
    nodes: [
      { id: "start", kind: "trigger", name: "Start" },
      {
        id: "work",
        kind: "agent",
        name: "Work",
        agent: "ceo",
        verify: { criteria: "Answer with a decision grounded in evidence." },
      },
      { id: "done", kind: "output", name: "Done" },
    ],
    edges: [
      { from: "start", to: "work" },
      { from: "work", to: "done" },
    ],
  };
}

describe("semantic verification authoring", () => {
  it("survives load, edit, and save", () => {
    const rows = draftNodes(graph());
    const draft: GraphDraft = {
      id: "verified",
      name: "Verified renamed",
      description: "",
      nodes: rows,
      edges: [
        { key: "e1", from: "start", to: "work", label: "" },
        { key: "e2", from: "work", to: "done", label: "" },
      ],
    };
    const assembled = assembleGraph(draft);
    expect(assembled.ok).toBe(true);
    if (!assembled.ok) return;
    expect(assembled.graph.nodes.find((node) => node.id === "work")?.verify).toEqual({
      criteria: "Answer with a decision grounded in evidence.",
    });
  });

  it("clears the agent-only policy on a kind change", () => {
    const row = draftNodes(graph()).find((node) => node.id === "work") as DraftNode;
    const changed = { ...row, ...changeKind("transform") };
    expect(changed.verify).toBeUndefined();
  });
});
