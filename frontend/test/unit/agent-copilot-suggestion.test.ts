// @vitest-environment jsdom
//
// Issue #1776: the copilot suggests, and the operator decides.
//
// These assert the property the whole feature rests on and that nothing else
// can check: a draft reaches the form only through Use it, and reaches storage
// only through a Save the operator presses afterwards. Everything else here —
// the gates, the refusal copy — exists so an operator is never offered a
// control that can only fail.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { refusalNotice, type ProfileDraft } from "@/api/agent-copilot";
import { AgentFields } from "@/views/team/AgentFields";
import { FieldCopilot } from "@/views/team/FieldCopilot";
import { emptyDraft } from "@/lib/agent";

let container: HTMLDivElement;
let root: Root;

function render(element: ReturnType<typeof createElement>) {
  act(() => root.render(element));
}

function testid(id: string): HTMLElement | null {
  return container.querySelector(`[data-testid="${id}"]`);
}

function click(id: string) {
  const el = testid(id);
  if (!el) throw new Error(`no element with data-testid=${id}`);
  act(() => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Lets a pending draft settle inside `act`, so React has applied its state. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

const DRAFTED: ProfileDraft = {
  field: "instructions",
  text: "Confirm the budget before launching. Report ROAS weekly.",
  source: "model",
};

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the teammate copilot suggests; the operator keeps or discards", () => {
  it("puts a draft beside the field, never in it, until Use it is pressed", async () => {
    const accepted: string[] = [];
    render(
      createElement(FieldCopilot, {
        field: "instructions",
        onDraft: async () => DRAFTED,
        onAccept: (text: string) => accepted.push(text),
      }),
    );

    click("agent-copilot-open-instructions");
    click("agent-copilot-draft-instructions");
    await settle();

    expect(testid("agent-copilot-suggestion-instructions")?.textContent).toContain(
      "Confirm the budget before launching",
    );
    expect(accepted).toEqual([]);

    click("agent-copilot-accept-instructions");
    expect(accepted).toEqual([DRAFTED.text]);
  });

  it("throws the draft away on Discard, leaving the field untouched", async () => {
    const accepted: string[] = [];
    render(
      createElement(FieldCopilot, {
        field: "instructions",
        onDraft: async () => DRAFTED,
        onAccept: (text: string) => accepted.push(text),
      }),
    );

    click("agent-copilot-open-instructions");
    click("agent-copilot-draft-instructions");
    await settle();
    click("agent-copilot-discard-instructions");

    expect(testid("agent-copilot-suggestion-instructions")).toBeNull();
    expect(accepted).toEqual([]);
  });

  /// A refusal names which of the three happened, so the sentence can name the
  /// operator's next move. Rendering it like a draft — or with one vague
  /// sentence — is what sends someone to check a credential that works.
  it("says why there is no draft, and offers nothing to accept", async () => {
    render(
      createElement(FieldCopilot, {
        field: "description",
        onDraft: async (): Promise<ProfileDraft> => ({
          field: "description",
          source: "unavailable",
          reason: "no_model",
        }),
        onAccept: () => {},
      }),
    );

    click("agent-copilot-open-description");
    click("agent-copilot-draft-description");
    await settle();

    expect(testid("agent-copilot-notice-description")?.textContent).toContain(
      "no model configured",
    );
    expect(testid("agent-copilot-suggestion-description")).toBeNull();
    expect(testid("agent-copilot-accept-description")).toBeNull();
  });

  it("does not offer to draft when there is nothing to draft with", () => {
    render(
      createElement(FieldCopilot, {
        field: "description",
        onDraft: async () => DRAFTED,
        onAccept: () => {},
        disabled: true,
        disabledNotice: "No model is configured, so the copilot can't draft yet.",
      }),
    );

    const open = testid("agent-copilot-open-description") as HTMLButtonElement | null;
    expect(open?.disabled).toBe(true);
    expect(container.textContent).toContain("No model is configured");
  });

  /// The refusal copy is keyed by reason on purpose: three causes, three next
  /// moves. An unknown reason says what happened without guessing which.
  it("names a different next move for each reason", () => {
    expect(refusalNotice("no_model")).toContain("Settings → Inference");
    expect(refusalNotice("model_unreachable")).toContain("Try again");
    expect(refusalNotice("unreadable")).toContain("add a note");
    expect(refusalNotice(undefined)).not.toContain("Settings → Inference");
  });
});

describe("where the copilot is offered at all", () => {
  const draft = emptyDraft();

  it("appears under the prose fields and nowhere else", () => {
    render(
      createElement(AgentFields, {
        idPrefix: "t",
        draft,
        onChange: () => {},
        copilot: (key: string) =>
          createElement("span", { "data-testid": `slot-${key}` }, key),
      }),
    );

    // A mandate and a persona are prose an operator wants help with.
    expect(testid("slot-description")).not.toBeNull();
    expect(testid("slot-instructions")).not.toBeNull();
    // A name is two words, and a ROLE is what delegation grounds on — drafting
    // one would change who the company routes work to.
    expect(testid("slot-name")).toBeNull();
    expect(testid("slot-role")).toBeNull();
  });

  it("is not offered on a field this host will not let you save", () => {
    render(
      createElement(AgentFields, {
        idPrefix: "t",
        draft,
        onChange: () => {},
        readOnly: (key: string) => key === "instructions",
        copilot: (key: string) =>
          createElement("span", { "data-testid": `slot-${key}` }, key),
      }),
    );

    expect(testid("slot-description")).not.toBeNull();
    expect(testid("slot-instructions")).toBeNull();
  });
});
