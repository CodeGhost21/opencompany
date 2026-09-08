// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { ChatView } from "@/views/ChatView";

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));
vi.mock("sonner", () => ({
  toast: Object.assign(toasts.base, { success: toasts.success, error: toasts.error, warning: vi.fn(), info: vi.fn() }),
}));

/**
 * `ChatView.applyBudget`/`resetBudget` — reached only through
 * `MembersPane`'s `canEditBudget={isAdmin && fromHost}` gate
 * (`cov-chat-members-budget-auth.test.ts` pins that gate itself) — had no
 * test for what they actually do once opened: `client.setTeamBudget` and
 * `client.clearTeamBudgetOverride` are both documented admin-only writes, and
 * a failed one must surface on screen rather than leave the dialog looking
 * like nothing happened.
 */

const DESK_DTO = { id: "main", name: "main", description: "The main channel", members: [] as string[] };
const MEMBER_DTO = { id: "m1", name: "Ada", role: "engineer" };

function clientAs(opts: {
  setTeamBudget?: (id: string, cap: number | null) => Promise<unknown>;
}): OpenCompanyClient {
  const named: Record<string, unknown> = {
    scopeFor: () => "/api/v1/companies/acme",
    get: (path: string) => {
      if (path.endsWith("/auth/me")) {
        return Promise.resolve({ id: "u1", email: "a@b.c", role: "admin", company: "acme" });
      }
      return Promise.resolve([]);
    },
    listDesks: () => Promise.resolve([DESK_DTO]),
    listTeam: () => Promise.resolve([MEMBER_DTO]),
    getOperatorChannel: () =>
      Promise.resolve({ id: "operator", name: "Operator", description: "Workflow reports and notifications" }),
    setTeamBudget: vi.fn(
      opts.setTeamBudget ??
        ((_id: string, cap: number | null) => Promise.resolve({ ...MEMBER_DTO, budgetUsdDaily: cap ?? undefined })),
    ),
  };
  return new Proxy(named, {
    get: (target, prop: string) => target[prop] ?? (() => Promise.resolve([])),
  }) as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia = ((query: string) => ({
    matches: query.includes("min-width"),
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  Object.defineProperty(window, "innerWidth", { value: 1440, writable: true });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  toasts.base.mockClear();
  toasts.success.mockClear();
  toasts.error.mockClear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

function tree(client: OpenCompanyClient): ReactNode {
  const view = createElement(ChatView, {
    client,
    company: "acme",
    sub: "main",
    onNavigate: vi.fn(),
    transcripts: {},
    setTranscripts: vi.fn(),
    resolveTypingNames: () => [],
    scopeRef: { current: { connection: "local", company: "acme", client } },
  });
  return createElement(ConnectionScopeProvider, {
    scope: { connection: "local", company: "acme" },
    children: view,
  });
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  });
}

async function mount(client: OpenCompanyClient) {
  await act(async () => {
    root.render(tree(client));
  });
  await flush();
}

function menuItem(testid: string): HTMLElement | null {
  return document.body.querySelector(`[data-testid="${testid}"]`);
}

async function openBudgetDialog() {
  const membersToggle = [...container.querySelectorAll("button")].find((b) =>
    (b.textContent ?? "").includes("teammates"),
  ) as HTMLButtonElement;
  await act(async () => membersToggle.click());
  await flush();

  const actions = container.querySelector('[aria-label="Actions for Ada"]') as HTMLButtonElement;
  await act(async () => actions.click());
  await flush();

  const edit = menuItem("team-budget-edit");
  await act(async () => edit?.click());
  await flush();
}

function setInput(el: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  act(() => {
    setter.call(el, text);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("setting a teammate's daily cap from chat", () => {
  it("writes the cap through client.setTeamBudget and confirms it", async () => {
    const client = clientAs({});
    await mount(client);
    await openBudgetDialog();

    const input = menuItem("team-budget-input") as HTMLInputElement;
    expect(input).not.toBeNull();
    setInput(input, "5");

    const save = menuItem("team-budget-save") as HTMLButtonElement;
    await act(async () => save.click());
    await flush();

    expect(client.setTeamBudget).toHaveBeenCalledWith("m1", 5, "acme");
    expect(toasts.success).toHaveBeenCalledWith("Daily cap set to $5.00.");
    expect(toasts.error).not.toHaveBeenCalled();
  });

  it("reports a refused write rather than leaving the dialog silent", async () => {
    const client = clientAs({
      setTeamBudget: () => Promise.reject(new ApiError(403, "forbidden", "only an admin can do that")),
    });
    await mount(client);
    await openBudgetDialog();

    const input = menuItem("team-budget-input") as HTMLInputElement;
    setInput(input, "5");

    const save = menuItem("team-budget-save") as HTMLButtonElement;
    await act(async () => save.click());
    await flush();

    expect(toasts.error).toHaveBeenCalledWith("only an admin can do that");
    expect(toasts.success).not.toHaveBeenCalled();
  });

  it("names a host with no console-budget route rather than the raw 404", async () => {
    const client = clientAs({
      setTeamBudget: () => Promise.reject(new ApiError(404, "not_found", "no route")),
    });
    await mount(client);
    await openBudgetDialog();

    const input = menuItem("team-budget-input") as HTMLInputElement;
    setInput(input, "5");

    const save = menuItem("team-budget-save") as HTMLButtonElement;
    await act(async () => save.click());
    await flush();

    expect(toasts.error).toHaveBeenCalledWith("This host doesn't support console budgets yet.");
  });
});
