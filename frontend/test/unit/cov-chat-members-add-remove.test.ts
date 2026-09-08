// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMember } from "@/lib/team";
import { AddMemberDialog } from "@/views/chat/AddMemberDialog";
import { MembersPane } from "@/views/chat/MembersPane";

/**
 * Add/remove a teammate from chat's member pane. `POST {scope}/team`
 * (`add_member`) and `DELETE {scope}/team/{agent_id}` (`remove_member`) are
 * both `scoped(…)` — any company member, not admin-only (a `budget_usd_daily`
 * at creation is the one thing that needs an admin, and the chat dialog never
 * collects one — `NewMemberFields` has no such field). `MembersPane` carries
 * no role check around "Add teammate" or "Remove from roster" the way it
 * does around the budget menu (`canEditBudget`, `cov-chat-members-budget-
 * auth.test.ts`) — this pins both stay live for a plain member, and that a
 * refused add is not silently swallowed.
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

function paneProps(overrides: Record<string, unknown> = {}) {
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
    canEditBudget: false,
    onEditBudget: vi.fn(),
    onRemoveCap: vi.fn(),
    onResetBudget: vi.fn(),
    setByLabel: () => undefined,
    ...overrides,
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

async function openMemberMenu() {
  const trigger = container.querySelector('[aria-label="Actions for Ada"]') as HTMLButtonElement;
  await act(async () => trigger.click());
}

function menuItem(text: string): HTMLElement | undefined {
  return Array.from(document.body.querySelectorAll('[role="menuitem"]')).find((el) =>
    (el.textContent ?? "").includes(text),
  ) as HTMLElement | undefined;
}

describe("MembersPane's Add/Remove, for a plain member (canEditBudget: false)", () => {
  it("still offers Add teammate — no admin gate on this control", async () => {
    await act(async () => {
      root.render(createElement(MembersPane, paneProps()));
    });

    const add = container.querySelector('[aria-label="Add teammate"]') as HTMLButtonElement;
    expect(add).not.toBeNull();
    expect(add.disabled).toBe(false);

    await act(async () => add.click());
    expect(paneProps().onAdd).not.toBeUndefined(); // sanity: prop exists
  });

  it("wires the Add trigger straight to onAdd", async () => {
    const onAdd = vi.fn();
    await act(async () => {
      root.render(createElement(MembersPane, paneProps({ onAdd })));
    });

    const add = container.querySelector('[aria-label="Add teammate"]') as HTMLButtonElement;
    await act(async () => add.click());

    expect(onAdd).toHaveBeenCalled();
  });

  it("still offers Remove from roster, beside a budget menu that stays absent", async () => {
    await act(async () => {
      root.render(createElement(MembersPane, paneProps()));
    });
    await openMemberMenu();

    expect(menuItem("Remove from roster")).not.toBeUndefined();
    expect(menuItem("Set daily budget")).toBeUndefined();
  });

  it("wires Remove straight to onRemove with the member's id", async () => {
    const onRemove = vi.fn();
    await act(async () => {
      root.render(createElement(MembersPane, paneProps({ onRemove })));
    });
    await openMemberMenu();
    await act(async () => menuItem("Remove from roster")?.click());

    expect(onRemove).toHaveBeenCalledWith("m1");
  });
});

describe("AddMemberDialog, when the write is refused", () => {
  function clientAs(): OpenCompanyClient {
    return {
      scopeFor: () => "/api/v1/companies/acme",
      get: (path: string) =>
        path.endsWith("/inference") ? Promise.resolve({ cognition: "echo" }) : Promise.resolve({}),
    } as unknown as OpenCompanyClient;
  }

  async function flush() {
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
  }

  it("keeps the dialog open and the typed fields intact for a retry, rather than closing on a write that never landed", async () => {
    const onAdd = vi.fn(async () => false);
    const onOpenChange = vi.fn();
    await act(async () => {
      root.render(
        createElement(AddMemberDialog, {
          open: true,
          onOpenChange,
          onAdd,
          client: clientAs(),
          company: "acme",
        }),
      );
    });
    await flush();

    const setInput = (el: HTMLInputElement, text: string) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      act(() => {
        setter.call(el, text);
        el.dispatchEvent(new Event("input", { bubbles: true }));
      });
    };
    setInput(document.body.querySelector("#member-name") as HTMLInputElement, "Nova");
    setInput(document.body.querySelector("#member-role") as HTMLInputElement, "Growth Marketer");

    const create = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent === "Add teammate" || b.textContent === "Adding…",
    ) as HTMLButtonElement;
    await act(async () => create.click());
    await flush();

    expect(onAdd).toHaveBeenCalledWith({
      name: "Nova",
      role: "Growth Marketer",
      description: "",
      inbox: false,
    });
    // Not closed on a failed write — the caller's own toast (ChatView.addMember)
    // is the visible error; this dialog's honest half is staying open and
    // retryable rather than claiming the write landed.
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    expect((document.body.querySelector("#member-name") as HTMLInputElement).value).toBe("Nova");
    const retry = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent === "Add teammate",
    ) as HTMLButtonElement | undefined;
    expect(retry).not.toBeUndefined();
    expect(retry?.disabled).toBe(false);
  });
});
