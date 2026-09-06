import { describe, expect, it } from "vitest";

import { derivePromptCard, newTaskBody } from "@/views/CreateTaskDialog";

/**
 * Issue #1106: the create dialog collects an owner.
 *
 * Assignee was moved out of this dialog by #301, on the reasoning that the host
 * defaults it and the card edits it afterwards. What that missed is what the
 * default *does*: an unassigned card is routed by a planning pass that picks
 * from the roster, and when two teammates plausibly fit it picked one with
 * nothing recorded about the other. Collecting it here is the pre-empt — an
 * operator who already knows the owner never reaches that ambiguity.
 *
 * The rule these pin is what the body **omits**. Optional controls are sent
 * only when they differ from the host's default, so adding them cannot change
 * what a card created without touching them posts.
 */
describe("newTaskBody", () => {
  it("omits the owner entirely when the card is left unassigned", () => {
    const body = newTaskBody({
      prompt: "ship the thing",
      deliverable: "once",
      priority: "medium",
      assignee: "",
    });
    expect(body).not.toBeNull();
    expect(body).not.toHaveProperty("assignee");
  });

  /**
   * The load-bearing one: a card created without touching either new control
   * posts the prompt and nothing else. `column` is absent for the same reason it
   * always was (#301) — the server's intake default is what keeps the human drag
   * into In progress the only thing that spends. `title` is absent because the
   * host names the card; the browser cutting the prompt into a headline is what
   * made a board read as a list of half-sentences.
   */
  it("posts only the prompt when neither control is touched", () => {
    expect(
      newTaskBody({ prompt: "ship the thing", deliverable: "once", priority: "medium", assignee: "" }),
    ).toEqual({ note: "ship the thing" });
  });

  it("carries a chosen priority", () => {
    const body = newTaskBody({ prompt: "ship the thing", deliverable: "once", priority: "high", assignee: "" });
    expect(body?.priority).toBe("high");
  });

  it("carries a chosen teammate verbatim", () => {
    const body = newTaskBody({
      prompt: "fetch trending tweets about agent harnesses",
      deliverable: "once",
      priority: "medium",
      assignee: "devrel",
    });
    expect(body?.assignee).toBe("devrel");
  });

  /**
   * A desk stays a desk. `AssigneeResolution::links_working_agent` is true only
   * for `Unassigned | Agent(_)`, precisely so a card assigned to a desk is never
   * silently rewritten to that desk's lead — so this body must not resolve one
   * either, the same invariant `AssigneeSelect` holds.
   */
  it("carries a chosen desk verbatim, without resolving it to a lead", () => {
    const body = newTaskBody({
      prompt: "ship it",
      deliverable: "once",
      priority: "medium",
      assignee: "engineering",
    });
    expect(body?.assignee).toBe("engineering");
  });

  it("carries the owner alongside a workflow deliverable", () => {
    const body = newTaskBody({
      prompt: "ship it",
      deliverable: "workflow",
      priority: "low",
      assignee: "devrel",
    });
    expect(body?.deliverable).toBe("workflow");
    expect(body?.priority).toBe("low");
    expect(body?.assignee).toBe("devrel");
  });

  it("refuses a prompt with nothing in it to name a card from", () => {
    expect(
      newTaskBody({ prompt: "   \n  ", deliverable: "once", priority: "medium", assignee: "devrel" }),
    ).toBeNull();
  });

  /**
   * The operator's full text surviving on the card is what the planner reads,
   * and it is now always the note — the dialog sends no title, so the host names
   * the card instead of the browser cutting the prompt at its first line.
   */
  it("sends the whole prompt as the note and lets the host name the card", () => {
    const prompt = `${"a".repeat(100)}\nsecond line`;
    const body = newTaskBody({ prompt, deliverable: "once", priority: "medium", assignee: "" });
    expect(body?.note).toBe(prompt);
    expect(body).not.toHaveProperty("title");
    expect(derivePromptCard(prompt).title).toBeUndefined();
  });
});
