import { describe, expect, it } from "vitest";

import type { WorkflowGraph } from "@/api/workflows";
import {
  noNodeResultsNotice,
  nodeOutputFor,
  parseNodeMessages,
  parseRunNodes,
  parseSingleNode,
} from "@/views/workflows/run-output";

/**
 * The shared per-node run-output parse (issue #596).
 *
 * One parser feeds three surfaces — the live run drawer, the durable run
 * inspector for a past run, and the pre-publish approvals card — so it has to be
 * defensive: a run's `output` is typed `unknown`, and older/edge runs may be a
 * bare string, missing `nodes`, or carry malformed node values. These tests pin
 * that a real run reads back and a garbled one degrades instead of throwing.
 */
describe("parseNodeMessages", () => {
  it("extracts an agent reply's text and ref from a node's items", () => {
    const raw = {
      items: [{ json: { text: "the draft tweet", agent_ref: "copywriter" } }],
    };
    expect(parseNodeMessages(raw)).toEqual([
      { text: "the draft tweet", agentRef: "copywriter" },
    ]);
  });

  it("prefers the outer value but falls back to the nested json.json.<key>", () => {
    const raw = { items: [{ json: { json: { text: "nested only" } } }] };
    expect(parseNodeMessages(raw)).toEqual([{ text: "nested only", agentRef: null }]);
  });

  it("drops items that carry neither text nor an agent ref", () => {
    const raw = { items: [{ json: { unrelated: 1 } }, { json: { text: "keep" } }] };
    expect(parseNodeMessages(raw)).toEqual([{ text: "keep", agentRef: null }]);
  });

  it("returns an empty list for a node with no items — the inspector's empty state", () => {
    expect(parseNodeMessages({ items: [] })).toEqual([]);
    expect(parseNodeMessages({})).toEqual([]);
    expect(parseNodeMessages("not an object")).toEqual([]);
    expect(parseNodeMessages(null)).toEqual([]);
  });
});

describe("nodeOutputFor", () => {
  const runOutput = { nodes: { writer: { items: [1] }, publish: { items: [2] } } };

  it("reads one node out of a full run output (`{ nodes: {…} }`)", () => {
    expect(nodeOutputFor(runOutput, "writer")).toEqual({ items: [1] });
  });

  it("reads one node out of an already-unwrapped nodes map (the durable record)", () => {
    expect(nodeOutputFor(runOutput.nodes, "publish")).toEqual({ items: [2] });
  });

  it("returns undefined for a node the run has no output for", () => {
    expect(nodeOutputFor(runOutput, "missing")).toBeUndefined();
    expect(nodeOutputFor("garbage", "writer")).toBeUndefined();
    expect(nodeOutputFor(null, "writer")).toBeUndefined();
  });
});

describe("parseRunNodes", () => {
  const graph = {
    id: "wf",
    name: "WF",
    nodes: [
      { id: "start", kind: "trigger", name: "Start" },
      { id: "writer", kind: "agent", name: "Draft Writer" },
    ],
    edges: [],
  } as unknown as WorkflowGraph;

  it("parses a real run's nodes, ordered by the graph, with display names", () => {
    const output = {
      nodes: {
        writer: { items: [{ json: { text: "hi", agent_ref: "copywriter" } }] },
        start: { items: [] },
      },
    };
    const parsed = parseRunNodes(output, graph);
    expect(parsed).not.toBeNull();
    // Graph order wins: start before writer.
    expect(parsed!.map((n) => n.id)).toEqual(["start", "writer"]);
    const writer = parsed!.find((n) => n.id === "writer")!;
    expect(writer.name).toBe("Draft Writer");
    expect(writer.messages).toEqual([{ text: "hi", agentRef: "copywriter" }]);
  });

  it("appends output-only node ids the graph does not know, after the graph's", () => {
    const output = { nodes: { writer: { items: [] }, ghost: { items: [] } } };
    const parsed = parseRunNodes(output, graph);
    expect(parsed!.map((n) => n.id)).toEqual(["writer", "ghost"]);
  });

  it("returns null when the output has no `nodes` map — caller shows raw JSON", () => {
    expect(parseRunNodes("a bare string", graph)).toBeNull();
    expect(parseRunNodes({ notNodes: {} }, graph)).toBeNull();
    expect(parseRunNodes(null, null)).toBeNull();
  });

  it("falls back to the node id as the name when no graph is loaded", () => {
    const parsed = parseRunNodes({ nodes: { writer: { items: [] } } }, null);
    expect(parsed![0]).toEqual({ id: "writer", name: "writer", port: null, messages: [] });
  });
});

describe("parseSingleNode", () => {
  it("parses one node's value with its display name and branch port", () => {
    const result = parseSingleNode("route", "Router", {
      items: [{ json: { text: "went left" } }],
      port: "yes",
    });
    expect(result).toEqual({
      id: "route",
      name: "Router",
      port: "yes",
      messages: [{ text: "went left", agentRef: null }],
    });
  });
});

/**
 * B-005 and B-039: what the drawer says when a run produced no per-node cards.
 *
 * One sentence used to cover every way of getting here — "The run finished, but
 * its output didn't match the expected node shape" — and it was wrong twice at
 * once for the two commonest ones. A run parked on an approval has not
 * finished, and `{"nodes": {}}` is the correct shape for a run that reached no
 * node; a stopped run has not finished either, and its `null` output is not a
 * malformed one. Both surfaces said so on the same screen while this paragraph
 * contradicted them.
 */
describe("noNodeResultsNotice", () => {
  it("says nothing at all when the run produced cards", () => {
    const cards = parseRunNodes({ nodes: { a: { items: [{ json: { text: "x" } }] } } }, null);
    expect(noNodeResultsNotice(cards, { nodes: {} }, "ok")).toBeNull();
  });

  it("calls an empty nodes map a run that produced nothing, not a bad shape", () => {
    const notice = noNodeResultsNotice([], { nodes: {} }, "awaiting-approval");
    expect(notice?.message).toBe(
      "No step produced output before this run stopped for an approval.",
    );
    expect(notice?.showRaw).toBe(false);
  });

  it("never claims a stopped run finished, and does not push its null output forward", () => {
    const notice = noNodeResultsNotice(parseRunNodes(null, null), null, "stopped");
    expect(notice?.message).toBe("You stopped this run before any step produced output.");
    expect(notice?.showRaw).toBe(false);
  });

  it("still reports a genuine shape fault, and opens the raw output for it", () => {
    // Present and wrong — the only thing a shape complaint is true of.
    const notice = noNodeResultsNotice(parseRunNodes("a bare string", null), "a bare string", "ok");
    expect(notice?.message).toContain("didn't match the shape");
    expect(notice?.showRaw).toBe(true);
  });

  it("says only what it knows when the host sent no verdict", () => {
    const notice = noNodeResultsNotice([], { nodes: {} }, undefined);
    expect(notice?.message).toBe("This run produced no step output.");
  });

  it("never claims a failed run finished either (tinysweeper: untested-branch)", () => {
    // Same wrongness as "stopped" — the old sentence said "The run finished"
    // for this arm too. A regression back to it would pass every other test
    // here, since none of them pass `verdict: "failed"` through.
    const notice = noNodeResultsNotice(parseRunNodes(null, null), null, "failed");
    expect(notice?.message).toBe("This run failed before any step produced output.");
    expect(notice?.showRaw).toBe(false);
  });
});
