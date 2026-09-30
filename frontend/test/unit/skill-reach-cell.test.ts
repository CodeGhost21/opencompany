// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { SkillAgentScope } from "@/api/skills";
import { avatarFor } from "@/lib/team";
import {
  SkillReachCell,
  fitCount,
  holders,
  reachName,
} from "@/views/skills/skill-reach-cell";

/**
 * Who reads a skill, as faces.
 *
 * The geometry is the MCP cell's, because the two sit in the same console and a
 * stack that packed differently would read as a different kind of answer. What
 * is not shared is which teammates are in it: a skill's scope stores three
 * states where a server's reach is binary, and a disabled skill reaches nobody
 * whatever the scope says.
 */

const scope = (
  id: string,
  state: SkillAgentScope["state"],
  held: boolean,
): SkillAgentScope => ({
  id,
  state,
  holds: held,
});

describe("how many faces fit", () => {
  it("shows them all before anything has measured the column", () => {
    expect(fitCount(7, null)).toBe(7);
    expect(fitCount(7, 0)).toBe(7);
  });

  it("never overflows a company small enough to fit", () => {
    expect(fitCount(2, 200)).toBe(2);
  });

  it("is a width question rather than a fixed three", () => {
    expect(fitCount(12, 400)).toBeGreaterThan(3);
    expect(fitCount(12, 140)).toBeLessThan(fitCount(12, 400));
  });
});

describe("which teammates are faces", () => {
  it("draws the ones that hold it, not the ones merely scoped to it", () => {
    // `inherited` and `excluded` both hold nothing while a skill is disabled, so
    // membership follows `holds` rather than the stored state.
    const agents = [
      scope("ceo", "inherited", true),
      scope("engineer", "excluded", false),
      scope("writer", "included", true),
    ];
    expect(holders(agents).map((a) => a.id)).toEqual(["ceo", "writer"]);
  });

  it("treats an absent scope as nobody rather than throwing", () => {
    expect(holders(undefined)).toEqual([]);
  });
});

describe("the name beside a face", () => {
  const team = [
    { id: "page_builder", name: "Page Builder", role: "Builder" },
    { id: "ops", name: "   ", role: "Operations" },
    { id: "bare", role: "Researcher" },
  ];

  it("prefers the display name the roster carries", () => {
    expect(reachName("page_builder", team)).toBe("Page Builder");
  });

  it("falls back to the role, then to the id, rather than rendering blank", () => {
    // A company that names only roles is the case `TeamMemberDto.name` documents.
    expect(reachName("ops", team)).toBe("Operations");
    expect(reachName("bare", team)).toBe("Researcher");
    expect(reachName("nobody", team)).toBe("nobody");
    expect(reachName("nobody", null)).toBe("nobody");
  });
});

describe("what the cell renders", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  const render = (node: Parameters<Root["render"]>[0]) =>
    act(() => root.render(node));

  it("says a disabled skill reaches nobody instead of showing it as a ratio", () => {
    // "0 of 6" would read as a scope that excluded everybody. The skill is off.
    render(
      createElement(SkillReachCell, {
        agents: [scope("ceo", "inherited", false)],
        team: null,
        enabled: false,
        onOverflow: () => undefined,
      }),
    );
    expect(host.textContent).toContain("Hidden from agents");
    expect(host.textContent).not.toContain("of");
  });

  it("carries the ratio beside the faces, which cannot express it alone", () => {
    render(
      createElement(SkillReachCell, {
        agents: [
          scope("ceo", "inherited", true),
          scope("engineer", "excluded", false),
          scope("writer", "included", true),
        ],
        team: [{ id: "ceo", name: "Chief Executive", role: "Chief" }],
        enabled: true,
        onOverflow: () => undefined,
      }),
    );
    expect(host.textContent).toContain("2 of 3");
    expect(host.querySelectorAll("img").length).toBe(2);
  });

  it("draws the mascot hashed from the id, never a fetched avatar", () => {
    render(
      createElement(SkillReachCell, {
        agents: [scope("page_builder", "inherited", true)],
        team: null,
        enabled: true,
        onOverflow: () => undefined,
      }),
    );
    const img = host.querySelector("img");
    expect(avatarFor("page_builder")).toMatch(/^tiny:/);
    expect(img?.getAttribute("src")).toContain("blob-");
  });

  it("withholds the overflow control until something has measured the column", () => {
    render(
      createElement(SkillReachCell, {
        agents: Array.from({ length: 9 }, (_, i) =>
          scope(`a${i}`, "inherited", true),
        ),
        team: null,
        enabled: true,
        onOverflow: () => undefined,
      }),
    );
    expect(
      host.querySelector('[data-testid="skill-reach-overflow"]'),
    ).toBeNull();
    expect(host.querySelectorAll("img").length).toBe(9);
  });
});
