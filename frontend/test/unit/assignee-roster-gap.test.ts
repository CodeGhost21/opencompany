// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { AssigneeSelect, rosterGapNotice } from "@/views/AssigneeSelect";

/**
 * A roster read that **failed** and a company that genuinely has **nobody** are
 * two different facts, and the picker rendered them identically: one row,
 * Unassigned.
 *
 * That matters because Unassigned is not the absence of a choice — it hands the
 * card to the orchestrator, which is a real write. An operator looking at a
 * one-row picker has to be able to tell "there is nobody else to pick" from "we
 * could not find out who else there is", and the component's own comment ("a
 * picker with no desks is still a picker") is sound reasoning for the first
 * case and wrong for the second.
 *
 * The same distinction the Overview draws before it claims "No desks yet"
 * (issue #1313, `overview-empty-state.test.ts`).
 */

const DESKS = [{ id: "engineering", name: "Engineering", members: ["eng-lead"] }];
const TEAM = [{ id: "eng-lead", name: "Eng Lead", role: "engineer" }];

function fakeClient(over: {
  desks?: () => Promise<unknown>;
  team?: () => Promise<unknown>;
} = {}) {
  return {
    scopeFor: (company: string | null) => `/api/v1/company/${company ?? "acme"}`,
    listDesks: vi.fn(over.desks ?? (async () => [])),
    listTeam: vi.fn(over.team ?? (async () => [])),
  } as unknown as OpenCompanyClient;
}

const fails = () => Promise.reject(new Error("host unreachable"));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

/** Mount the picker and open its popup, which is where the roster is read. */
async function open(client: OpenCompanyClient, value = "") {
  await act(async () => {
    root.render(
      createElement(AssigneeSelect, {
        id: "assignee",
        client,
        company: "acme",
        value,
        onChange: vi.fn(),
      }),
    );
  });
  // The two roster reads settle in microtasks and the state they set lands a
  // tick later; give React those ticks rather than assuming one flush covers
  // both.
  await act(async () => {});
  await act(async () => {});
  await act(async () => {
    (document.querySelector("#assignee") as HTMLElement).click();
  });
}

function popupText(): string {
  return document.body.textContent ?? "";
}

describe("rosterGapNotice", () => {
  it("says nothing when both halves answered", () => {
    expect(rosterGapNotice({ desks: false, team: false })).toBeNull();
  });

  /**
   * Each line has to survive being the *only* thing in the list beside
   * Unassigned, so each one names which half is missing and says the list is
   * short rather than complete.
   */
  it("names the missing half, and calls the list incomplete rather than empty", () => {
    // Each single-half line names *its* half and not the other. Reporting both
    // whenever either failed would tell the operator the desks are missing from
    // a list that is showing every one of them.
    expect(rosterGapNotice({ desks: true, team: false })).toContain("desks");
    expect(rosterGapNotice({ desks: true, team: false })).not.toContain("teammates");
    expect(rosterGapNotice({ desks: false, team: true })).toContain("teammates");
    expect(rosterGapNotice({ desks: false, team: true })).not.toContain("desks");
    for (const failed of [
      { desks: true, team: false },
      { desks: false, team: true },
      { desks: true, team: true },
    ]) {
      expect(rosterGapNotice(failed)).toContain("incomplete, not empty");
    }
  });
});

describe("the picker for a roster it could not read", () => {
  it("stays quiet for a company that genuinely has nobody", async () => {
    await open(fakeClient());
    // Both reads answered `[]`. There is nothing wrong here to report, and a
    // warning would be a false alarm on every brand-new company.
    expect(document.querySelector('[data-testid="assignee-roster-gap"]')).toBeNull();
    expect(popupText()).toContain("Unassigned");
  });

  it("says so when both halves failed, instead of looking like an empty company", async () => {
    await open(fakeClient({ desks: fails, team: fails }));

    const gap = document.querySelector('[data-testid="assignee-roster-gap"]');
    expect(gap).not.toBeNull();
    expect(gap?.textContent).toContain("incomplete, not empty");
  });

  it("says so when only the teammates read failed, though desks arrived", async () => {
    await open(fakeClient({ desks: async () => DESKS, team: fails }));

    expect(popupText()).toContain("Engineering");
    const gap = document.querySelector('[data-testid="assignee-roster-gap"]')?.textContent ?? "";
    expect(gap).toContain("teammates");
    // The desks are right there in the list; saying they are missing too would
    // be a second wrong answer on top of the one this fixes.
    expect(gap).not.toContain("desks");
  });

  /**
   * The regression the old heuristic actually shipped. "Off-roster" was
   * inferred from *either list being non-empty*, which held for the case it was
   * written against and broke on the mixed one: with `/desks` answering and
   * `/team` rejected, a perfectly current teammate id was flagged "not on
   * roster" against a roster half of which never arrived.
   */
  it("does not flag a value as off-roster against a roster half that never arrived", async () => {
    await open(fakeClient({ desks: async () => DESKS, team: fails }), "eng-lead");

    expect(popupText()).not.toContain("not on roster");
  });

  it("still flags a genuinely off-roster value when both halves answered", async () => {
    await open(fakeClient({ desks: async () => DESKS, team: async () => TEAM }), "someone-else");

    expect(popupText()).toContain("not on roster");
    expect(document.querySelector('[data-testid="assignee-roster-gap"]')).toBeNull();
  });
});
