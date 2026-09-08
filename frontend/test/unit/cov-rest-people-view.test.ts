// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { ApiError } from "@/api/types";
import type { Person } from "@/api/auth";
import { PeopleView } from "@/views/PeopleView";

const toasts = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock("sonner", () => ({ toast: { success: toasts.success, error: toasts.error, warning: vi.fn(), info: vi.fn() } }));

/**
 * People is admin-only, but courtesy only — `/users` writes 403 a member
 * regardless. Three properties, none pinned anywhere before this:
 *
 * PEOPLE-001: a member gets the notice and nothing else, and — the actual
 * enforcement-adjacent property — the roster is never even fetched for one,
 * so nothing about who else can sign in reaches a member's client at all.
 *
 * PEOPLE-002: the last-admin disable is a client-side courtesy over
 * `activeAdmins.length`, computed from whatever roster happened to load. It
 * can be stale. The property that actually matters is downstream of that:
 * when the roster made a write look safe and the host's own last-admin guard
 * refuses it anyway, the refusal has to surface honestly, not read as success.
 *
 * PEOPLE-003: role change, reset password, and sign-out-everywhere are three
 * independent writes with three independent failure paths, never isolated
 * from each other in a unit test before.
 */

function person(over: Partial<Person> = {}): Person {
  return {
    id: "p1",
    email: "alex@acme.test",
    role: "member",
    status: "active",
    hasPassword: true,
    mustChangePassword: false,
    createdAtMillis: 0,
    ...over,
  };
}

function clientAs(opts: {
  role: "admin" | "member";
  people?: Person[];
  patch?: () => Promise<Person>;
  post?: () => Promise<Person>;
  del?: () => Promise<{ revoked: number }>;
}): OpenCompanyClient {
  const get = vi.fn((path: string) => {
    if (path.endsWith("/auth/me")) {
      return Promise.resolve({ id: "me", email: "me@acme.test", role: opts.role, company: "acme", hasPassword: true, mustChangePassword: false });
    }
    if (path.endsWith("/users")) return Promise.resolve(opts.people ?? []);
    if (path.endsWith("/users/invites")) return Promise.resolve([]);
    return Promise.reject(new Error(`unexpected GET ${path}`));
  });
  return {
    scopeFor: () => "/api/v1/company/acme",
    get,
    patch: vi.fn(opts.patch ?? (() => Promise.resolve(person()))),
    post: vi.fn(opts.post ?? (() => Promise.resolve(person()))),
    del: vi.fn(opts.del ?? (() => Promise.resolve({ revoked: 1 }))),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(PeopleView, { client, company: "acme" }));
  });
}

function menuButtonFor(email: string): HTMLButtonElement {
  const row = Array.from(container.querySelectorAll("div")).find((d) =>
    d.textContent?.includes(email) && d.querySelector('[aria-label^="Manage"]'),
  );
  return row?.querySelector('[aria-label^="Manage"]') as HTMLButtonElement;
}

async function openMenuFor(email: string) {
  await act(async () => {
    menuButtonFor(email).click();
  });
}

function menuItem(label: string): HTMLElement {
  const found = Array.from(document.querySelectorAll('[role="menuitem"]')).find(
    (el) => el.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no "${label}" menu item in:\n${document.body.innerHTML}`);
  return found as HTMLElement;
}

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
  toasts.success.mockClear();
  toasts.error.mockClear();
});

describe("People is admin-only, and a member's client never even sees the roster", () => {
  it("shows a member the notice, no roster, and no invite control", async () => {
    const client = clientAs({ role: "member", people: [person({ email: "somebody@acme.test" })] });
    await show(client);

    expect(container.textContent).toContain("Only an admin can manage people");
    expect(container.textContent).not.toContain("Invite");
    expect(
      (client.get as ReturnType<typeof vi.fn>).mock.calls.some((call: unknown[]) =>
        String(call[0]).endsWith("/users"),
      ),
    ).toBe(false);
  });

  it("gives an admin the roster and the invite control", async () => {
    const client = clientAs({ role: "admin", people: [person({ email: "somebody@acme.test" })] });
    await show(client);

    expect(container.textContent).toContain("somebody@acme.test");
    expect(container.textContent).toContain("Invite");
  });
});

describe("a stale-looking last-admin guard still surfaces the host's real refusal", () => {
  it("does not pretend the demote succeeded when the host refuses it as the last admin", async () => {
    // Two active admins on the loaded roster, so the client-side mirror does
    // NOT mark either as the last admin — the button is live. The host
    // refuses anyway (a concurrent change elsewhere already made this one
    // the last admin), and that refusal must read as a refusal.
    const client = clientAs({
      role: "admin",
      people: [
        person({ id: "p1", email: "alex@acme.test", role: "admin" }),
        person({ id: "p2", email: "sam@acme.test", role: "admin" }),
      ],
      patch: () => Promise.reject(new ApiError(409, "conflict", "this is the company's last admin")),
    });
    await show(client);

    await openMenuFor("alex@acme.test");
    await act(async () => {
      menuItem("Make member").click();
    });

    // A false success is exactly the risk this cell names: the client-side
    // mirror said this write was safe, and the host refused it anyway.
    expect(toasts.error).toHaveBeenCalledWith(expect.stringContaining("last admin"));
    expect(toasts.success).not.toHaveBeenCalled();
  });
});

describe("the three per-person actions fail independently and honestly", () => {
  it("reset password: the toast reports the host's refusal rather than a silent no-op", async () => {
    const client = clientAs({
      role: "admin",
      people: [person()],
      post: () => Promise.reject(new ApiError(400, "bad_request", "cannot set a password for a wallet-only account")),
    });
    await show(client);

    await openMenuFor("alex@acme.test");
    await act(async () => {
      menuItem("Set a temporary password").click();
    });
    const input = document.getElementById("temp-password") as HTMLInputElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setter?.call(input, "a-long-enough-password");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const save = Array.from(document.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "Set it",
    ) as HTMLButtonElement;
    await act(async () => {
      save.click();
    });

    expect(client.post).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith(
      expect.stringContaining("cannot set a password for a wallet-only account"),
    );
    expect(toasts.success).not.toHaveBeenCalled();
  });

  it("sign out everywhere: a refusal is reported rather than silently accepted", async () => {
    const del = vi.fn(() => Promise.reject(new ApiError(500, "server_error", "could not revoke sessions")));
    const client = clientAs({ role: "admin", people: [person()], del });
    await show(client);

    await openMenuFor("alex@acme.test");
    await act(async () => {
      menuItem("Sign out everywhere").click();
    });

    expect(del).toHaveBeenCalledTimes(1);
    expect(toasts.error).toHaveBeenCalledWith(expect.stringContaining("could not revoke sessions"));
    expect(toasts.success).not.toHaveBeenCalled();
  });
});
