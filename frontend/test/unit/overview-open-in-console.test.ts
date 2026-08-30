import { describe, expect, it } from "vitest";

import { departmentIdOfDesk } from "@/views/overview/kg/adapter";
import { destinationFor, MEMORY_DESTINATION } from "@/views/overview/kg/open-in-console";

/**
 * Issue #1308: the graph was the console's landing page and its only terminal
 * surface. It drew every teammate, every open board card, every workflow and
 * every desk the company declares, and linked to none of them — an operator
 * could learn that a teammate had never run and then had to leave through the
 * sidebar and find them again by hand.
 *
 * The lookup is pure so the whole table can be asserted without a graph, a
 * host or a browser. What it is really guarding is the join: a node id carries
 * the host's own id (`emp:<agentId>`, `team:desk:<deskId>`), and the address
 * has to be built from *that* id rather than from anything re-derived.
 */
describe("where a graph node lives in the console", () => {
  it("sends a teammate to their own page", () => {
    expect(destinationFor("emp:frontend_engineer")).toEqual({
      hash: "#/team/frontend_engineer",
      label: "Open teammate",
    });
  });

  it("sends an SOP task to its board card", () => {
    expect(destinationFor("task:tsk_01H9")).toEqual({
      hash: "#/tasks/tsk_01H9",
      label: "Open card",
    });
  });

  it("sends a workflow to the flow it names", () => {
    expect(destinationFor("flow:nightly-digest")).toEqual({
      hash: "#/workflows/nightly-digest",
      label: "Open workflow",
    });
  });

  it("sends a human to the people list", () => {
    // There is no per-person address, so the label names the list rather than
    // pretending to name the row.
    expect(destinationFor("person:mithil@example.com")).toEqual({
      hash: "#/settings/people",
      label: "Open people",
    });
  });

  it("sends a note to the Brain", () => {
    expect(MEMORY_DESTINATION).toEqual({ hash: "#/memory", label: "Open Brain" });
  });

  describe("a department", () => {
    it("unwraps the desk id the adapter wrapped", () => {
      // The graph holds `team:` + whatever `departmentIdOfDesk` produced.
      // Building the address from the raw department id would send the
      // operator to `#/company/desk:eng`, which names no desk. Composing the
      // id here rather than hardcoding it keeps this honest if the prefix
      // ever changes.
      const nodeId = `team:${departmentIdOfDesk("eng")}`;
      expect(nodeId).toBe("team:desk:eng");
      expect(destinationFor(nodeId)).toEqual({
        hash: "#/company/eng",
        label: "Open desk",
      });
    });

    it("has no page when it was not built from a desk", () => {
      // `UNPLACED` and anything else the adapter did not derive from a desk
      // has no desk behind it, so there is nothing to open. Answering `null`
      // is the honest result; guessing at `#/company` would send an operator
      // to a page that does not contain what they clicked.
      expect(destinationFor("team:unplaced")).toBeNull();
    });
  });

  describe("nodes the console has no address for", () => {
    it.each([
      // A grant is a string in `company.toml`, not a record with a page.
      ["tool:workspace.*", "a tool grant"],
      ["tool:slack@desk:eng", "a tool split per desk"],
      // A stage is a node inside a saved graph; the flow has an address, the
      // node within it does not.
      ["step:nightly-digest:2", "a workflow stage"],
      // The company core is the page you are already on.
      ["self", "the company itself"],
    ])("answers null for %s (%s)", (nodeId) => {
      expect(destinationFor(nodeId)).toBeNull();
    });
  });

  describe("ids the host supplied", () => {
    it("encodes a segment that would otherwise change the address", () => {
      // A desk named with a slash, or an id carrying a `#`, would write a
      // different address than the one it names.
      expect(destinationFor("team:desk:sales/ops")?.hash).toBe("#/company/sales%2Fops");
      expect(destinationFor("task:a#b")?.hash).toBe("#/tasks/a%23b");
    });

    it("answers null rather than a truncated address for an empty id", () => {
      expect(destinationFor("emp:")).toBeNull();
      expect(destinationFor("task:")).toBeNull();
      expect(destinationFor("flow:")).toBeNull();
      expect(destinationFor("team:desk:")).toBeNull();
    });
  });
});
