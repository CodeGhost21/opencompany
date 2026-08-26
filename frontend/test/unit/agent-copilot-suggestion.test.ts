// @vitest-environment jsdom
//
// Issue #1776: the copilot converses, and the operator decides.
//
// These pin the property the whole feature rests on and that nothing else can
// check — a draft reaches the form only through Use it, and reaches storage
// only through a Save pressed afterwards — plus the two things that made it a
// conversation rather than a Draft button: the transcript goes back with every
// turn, and a turn is allowed to ask instead of drafting.

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { refusalNotice, type CopilotTurn, type ProfileDraft } from "@/api/agent-copilot";
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

/** Types into the composer the way React sees it. */
function say(field: string, text: string) {
  const el = testid(`agent-copilot-input-${field}`) as HTMLTextAreaElement;
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  act(() => {
    setter.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** Lets a pending turn settle inside `act`, so React has applied its state. */
async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

const FIRST = "Test features and block releases on open P1s.";
const SECOND = "Test features. Block releases on open P1s.";

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

describe("the teammate copilot converses; the operator keeps or discards", () => {
  /// Opening drafts immediately rather than showing an empty chat. Someone who
  /// opened the copilot on a blank persona box wants something to react to —
  /// asking them to describe it first asks for the thing they could not write.
  it("opens with a first draft, beside the field and not in it", async () => {
    const accepted: string[] = [];
    const asked: CopilotTurn[][] = [];
    render(
      createElement(FieldCopilot, {
        field: "instructions",
        onTurn: async (conversation: CopilotTurn[]) => {
          asked.push(conversation);
          return { field: "instructions", reply: "Here's a first pass.", text: FIRST, source: "model" };
        },
        onAccept: (text: string) => accepted.push(text),
      }),
    );

    click("agent-copilot-open-instructions");
    await settle();

    expect(asked).toEqual([[]]);
    expect(testid("agent-copilot-suggestion-instructions")?.textContent).toContain(FIRST);
    expect(container.textContent).toContain("Here's a first pass.");
    expect(accepted).toEqual([]);
  });

  /// The whole reason this stopped being a Draft button: "shorter" has to mean
  /// shorter than the last version, so the transcript — including the copilot's
  /// own draft — goes back every turn.
  it("carries the whole transcript, drafts included, into the next turn", async () => {
    const asked: CopilotTurn[][] = [];
    let call = 0;
    render(
      createElement(FieldCopilot, {
        field: "instructions",
        onTurn: async (conversation: CopilotTurn[]) => {
          asked.push(conversation);
          call += 1;
          return {
            field: "instructions",
            reply: call === 1 ? "Here's a first pass." : "Split it into two sentences.",
            text: call === 1 ? FIRST : SECOND,
            source: "model",
          } as ProfileDraft;
        },
        onAccept: () => {},
      }),
    );

    click("agent-copilot-open-instructions");
    await settle();
    say("instructions", "shorter sentences");
    click("agent-copilot-send-instructions");
    await settle();

    expect(asked).toHaveLength(2);
    const second = asked[1];
    expect(second).toHaveLength(2);
    expect(second[0].role).toBe("copilot");
    // Its reply AND its draft — iterating on a description of a draft is not
    // iterating on the draft.
    expect(second[0].text).toContain("Here's a first pass.");
    expect(second[0].text).toContain(FIRST);
    expect(second[1]).toEqual({ role: "operator", text: "shorter sentences" });

    // Every drafted turn keeps its card, so an operator can go back to a
    // version they preferred rather than asking for it again. The newest is the
    // last one.
    const cards = container.querySelectorAll(
      '[data-testid="agent-copilot-suggestion-instructions"]',
    );
    expect(cards).toHaveLength(2);
    expect(cards[0].textContent).toContain(FIRST);
    expect(cards[1].textContent).toContain(SECOND);
  });

  it("fills the field only on Use it, and saves nothing", async () => {
    const accepted: string[] = [];
    render(
      createElement(FieldCopilot, {
        field: "instructions",
        onTurn: async () => ({
          field: "instructions" as const,
          reply: "Here you go.",
          text: FIRST,
          source: "model" as const,
        }),
        onAccept: (text: string) => accepted.push(text),
      }),
    );

    click("agent-copilot-open-instructions");
    await settle();
    expect(accepted).toEqual([]);

    click("agent-copilot-accept-instructions");
    expect(accepted).toEqual([FIRST]);
    // Accepting closes the conversation — the draft is in the box now, and the
    // box is where editing belongs.
    expect(testid("agent-copilot-instructions")).toBeNull();
  });

  /// A turn may ASK. This is the thing a one-shot pass structurally could not
  /// do, and the reason it never found out what the operator meant.
  it("lets a turn ask a question and offer nothing to accept", async () => {
    render(
      createElement(FieldCopilot, {
        field: "description",
        onTurn: async () => ({
          field: "description" as const,
          reply: "Do they own returns as well, or just outbound?",
          source: "model" as const,
        }),
        onAccept: () => {},
      }),
    );

    click("agent-copilot-open-description");
    await settle();

    expect(container.textContent).toContain("Do they own returns as well");
    expect(testid("agent-copilot-suggestion-description")).toBeNull();
    expect(testid("agent-copilot-accept-description")).toBeNull();
    // …and it is not mistaken for a failure.
    expect(testid("agent-copilot-notice-description")).toBeNull();
  });

  /// A refusal names which of the three happened, so the sentence can name the
  /// operator's next move. Rendering it like a turn would be worse than useless.
  it("says why there is no answer at all", async () => {
    render(
      createElement(FieldCopilot, {
        field: "description",
        onTurn: async () => ({
          field: "description" as const,
          source: "unavailable" as const,
          reason: "no_model" as const,
        }),
        onAccept: () => {},
      }),
    );

    click("agent-copilot-open-description");
    await settle();

    expect(testid("agent-copilot-notice-description")?.textContent).toContain(
      "no model configured",
    );
    expect(testid("agent-copilot-suggestion-description")).toBeNull();
  });

  it("does not offer the copilot when there is nothing to draft with", () => {
    render(
      createElement(FieldCopilot, {
        field: "description",
        onTurn: async () => ({ field: "description" as const, source: "model" as const }),
        onAccept: () => {},
        disabled: true,
        disabledNotice: "No model is configured, so the copilot can't draft yet.",
      }),
    );

    const open = testid("agent-copilot-open-description") as HTMLButtonElement | null;
    expect(open?.disabled).toBe(true);
    expect(container.textContent).toContain("No model is configured");
  });

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
