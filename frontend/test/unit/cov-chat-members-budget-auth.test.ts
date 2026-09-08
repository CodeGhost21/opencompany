// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MembersPane } from "@/views/chat/MembersPane";
import type { TeamMember } from "@/lib/team";

/**
 * `MemberRow`'s budget menu is gated `canEditBudget={isAdmin && fromHost}`
 * (`ChatView.tsx`) — a member with no admin session must not see "Set daily
 * budget…" at all, since `PUT …/team/{id}/budget` is admin-only on the host.
 */

const MEMBER: TeamMember = {
  id: "m1",
  name: "Ada",
  role: "engineer",
  description: "",
  tone: "blue",
  avatar: "ada",
  inboxEnabled: false,
  effectiveTools: [],
  desks: [],
};

function baseProps(canEditBudget: boolean) {
  return {
    channelMembers: null,
    others: [MEMBER],
    people: [],
    loading: false,
    fromHost: true,
    onToggleInbox: vi.fn(),
    onRemove: vi.fn(),
    onAdd: vi.fn(),
    onMessage: vi.fn(),
    canEditBudget,
    onEditBudget: vi.fn(),
    onRemoveCap: vi.fn(),
    onResetBudget: vi.fn(),
    setByLabel: () => undefined,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function openMenu() {
  const trigger = container.querySelector('[aria-label="Actions for Ada"]') as HTMLButtonElement;
  await act(async () => trigger.click());
}

// The menu content renders through a `MenuPrimitive.Portal` (base-ui), not
// into `container`.
function menuAt(testid: string): HTMLElement | null {
  return document.body.querySelector(`[data-testid="${testid}"]`);
}

describe("MembersPane budget menu, by canEditBudget (ChatView's isAdmin && fromHost)", () => {
  it("offers no budget entry point to a member", async () => {
    await act(async () => {
      root.render(createElement(MembersPane, baseProps(false)));
    });
    await openMenu();

    expect(menuAt("team-budget-edit")).toBeNull();
    expect(menuAt("team-budget-remove")).toBeNull();
    expect(menuAt("team-budget-reset")).toBeNull();
  });

  it("offers the budget entry point to an admin", async () => {
    await act(async () => {
      root.render(createElement(MembersPane, baseProps(true)));
    });
    await openMenu();

    expect(menuAt("team-budget-edit")).not.toBeNull();
  });

  it("still withholds the budget entry point from an admin viewing a starter-roster teammate (fromHost false)", async () => {
    // `canEditBudget` is computed as `isAdmin && fromHost` in ChatView — a
    // starter-roster row has no budget record on the host to edit.
    await act(async () => {
      root.render(createElement(MembersPane, baseProps(false)));
    });
    await openMenu();

    expect(menuAt("team-budget-edit")).toBeNull();
  });
});
