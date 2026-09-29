// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { AgentFace } from "@/components/agent-face";
import { AgentStatusDot } from "@/components/agent-status-dot";
import type { AgentPresenceState } from "@/lib/agent-presence";
import * as room from "@/room/store";

/**
 * `AgentStatusDot` draws the six-state answer as a badge, and `AgentFace` reads
 * it from the Room store for whoever opts in. The dot is presentational, so the
 * assertions are on what a screen reader and a test id see: the label, the
 * state, and that `inactive` draws nothing (and never wears `presence-dot`).
 */

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLElement;
let root: Root;

beforeEach(() => {
  room.resetStore();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const dot = () => host.querySelector<HTMLElement>('[data-testid="agent-status-dot"]');

describe("AgentStatusDot", () => {
  const cases: [Exclude<AgentPresenceState, "inactive">, string][] = [
    ["approval", "Waiting for your approval"],
    ["working", "Working"],
    ["typing", "Typing"],
    ["thinking", "Thinking"],
    ["queued", "Queued"],
  ];

  for (const [state, label] of cases) {
    it(`draws ${state} with its label and state`, () => {
      act(() => root.render(createElement(AgentStatusDot, { state })));
      const el = dot();
      expect(el).not.toBeNull();
      expect(el?.getAttribute("role")).toBe("img");
      expect(el?.getAttribute("aria-label")).toBe(label);
      expect(el?.getAttribute("title")).toBe(label);
      expect(el?.dataset.state).toBe(state);
    });
  }

  it("draws nothing for inactive, and never the person dot's testid", () => {
    act(() => root.render(createElement(AgentStatusDot, { state: "inactive" })));
    expect(dot()).toBeNull();
    expect(host.querySelector('[data-testid="presence-dot"]')).toBeNull();
  });

  it("gives each running-colour state its own static shape", () => {
    const shape = (state: AgentPresenceState) => {
      act(() => root.render(createElement(AgentStatusDot, { state })));
      return dot()?.innerHTML ?? "";
    };
    const shapes = new Set(["working", "typing", "thinking"].map((s) => shape(s as AgentPresenceState)));
    expect(shapes.size).toBe(3);
  });
});

describe("AgentFace", () => {
  const face = (props: { agentId?: string | null; chatId?: string | null }) =>
    act(() =>
      root.render(createElement(AgentFace, { ...props, children: createElement("span", null, "A") })),
    );

  it("shows no dot for an inactive agent, and none when no agent is named", () => {
    face({ agentId: "rae" });
    expect(dot()).toBeNull();
    face({});
    expect(dot()).toBeNull();
  });

  it("follows the store: approval from the feed, then thinking on the DM's own thread only", () => {
    face({ agentId: "rae", chatId: "rae" });
    act(() => room.setApprovalAgents({ rae: 1 }));
    expect(dot()?.dataset.state).toBe("approval");
    act(() => room.setApprovalAgents({}));
    expect(dot()).toBeNull();

    act(() => {
      room.setThreadAgents({ rae: "rae" });
      room.setOpenTurns({ rae: [{ queued: false, chatId: "rae" }] });
    });
    expect(dot()?.dataset.state).toBe("thinking");
    // The same agent's state in another chat is not this DM's.
    face({ agentId: "rae", chatId: "other" });
    expect(dot()).toBeNull();
    // Agent-wide, it is.
    face({ agentId: "rae" });
    expect(dot()?.dataset.state).toBe("thinking");
  });

  it("shows queued for a queued turn and clears when the turn settles", () => {
    face({ agentId: "rae", chatId: "rae" });
    act(() => {
      room.setThreadAgents({ rae: "rae" });
      room.setOpenTurns({ rae: [{ queued: true, chatId: "rae" }] });
    });
    expect(dot()?.dataset.state).toBe("queued");
    act(() => room.setOpenTurns({}));
    expect(dot()).toBeNull();
  });
});
