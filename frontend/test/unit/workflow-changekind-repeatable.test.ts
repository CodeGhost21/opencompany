import { describe, expect, it } from "vitest";

import { changeKind, type DraftNode } from "@/views/WorkflowCreateDialog";

// #850 review finding: `changeKind` reset every other kind-specific field
// (config/configDraft/configExtra, agent, schedule, destination) but left
// `repeatable` untouched. The host rejects `repeatable` on any node kind
// other than `tool_call`/`http_request`, and this dialog has no control to
// clear it once a node stops being a call — so a node loaded with
// `repeatable: false`, switched to e.g. `transform`, and submitted would
// carry a value the host refuses, with no way for the author to see or
// clear it first.
//
// `updateNode` applies a kind change the same way the dialog does — spread-
// merging `changeKind`'s patch onto the existing row (`{ ...r, ...fields }`)
// — so the patch has to carry an explicit `repeatable: undefined` key to
// clear a previously-set value; merely omitting the key from the patch object
// leaves the row's prior value untouched.
function callNode(kind: "tool_call" | "http_request"): DraftNode {
  return {
    key: "k1",
    id: "publish",
    kind,
    name: "Publish",
    summary: "",
    agent: "",
    schedule: "",
    destinationKind: "",
    destinationTarget: "",
    configDraft: {},
    repeatable: false,
  };
}

describe("changeKind — repeatable reset", () => {
  it("clears a declared repeatable when switching away from a call kind", () => {
    const row = callNode("tool_call");
    const next: DraftNode = { ...row, ...changeKind("transform") };
    expect(next.repeatable).toBeUndefined();
  });

  it("clears repeatable even when switching between the two call kinds", () => {
    // Matches the file's stated convention for every other kind-specific
    // field: reset unconditionally on any kind change, rather than special-
    // casing the pair of kinds that could both hold a value.
    const row = callNode("tool_call");
    const next: DraftNode = { ...row, ...changeKind("http_request") };
    expect(next.repeatable).toBeUndefined();
  });

  it("clears repeatable when switching into a call kind from a non-call kind", () => {
    const row: DraftNode = { ...callNode("tool_call"), kind: "transform", repeatable: undefined };
    const next: DraftNode = { ...row, ...changeKind("tool_call") };
    expect(next.repeatable).toBeUndefined();
  });
});
