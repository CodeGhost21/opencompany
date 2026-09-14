// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { AuthMode, Me } from "@/api/auth";
import type { OpenCompanyClient } from "@/api/client";
import { addConnection, getConnection, resetConnections } from "@/connections/registry";
import type { ConnectionId } from "@/connections/types";
import { AccountCard } from "@/views/SettingsView";

/**
 * Signing out, from General settings' Account card.
 *
 * Two facts, and they are separate:
 *
 * - The card exists only where signing out means something. A `none`-mode
 *   company resolves its principal from the request, so `/auth/me` answers and
 *   an identity is there to render — but `auth/logout` refuses with
 *   `auth_mode` and there is no sign-in screen to land on. A card with a button
 *   whose only outcome is an error is worse than no card.
 * - Pressing it revokes the session *and* puts the connection back to
 *   `unauthenticated`, which is the state `ConnectionConsole` renders `Login`
 *   from. Only doing the first would leave the console drawing a signed-out
 *   company's shell until some later request happened to 401.
 */

function meFor(company: string): Me {
  return {
    id: `${company}-me`,
    email: `ada@${company}.test`,
    displayName: "Ada Lovelace",
    role: "admin",
    company,
    hasPassword: false,
    mustChangePassword: false,
  };
}

function host(mode: AuthMode, logoutFails = false) {
  const posts: string[] = [];
  const client = {
    scopeFor: (company: string | null) =>
      company === null ? "/api/v1/company" : `/api/v1/companies/${company}`,
    get: async (path: string) => {
      if (path.endsWith("/auth/config")) {
        return { mode, passwords: mode === "email", magicLink: true };
      }
      return meFor("alpha");
    },
    post: async (path: string) => {
      posts.push(path);
      if (logoutFails) throw new Error("the host could not be reached");
      return { ok: true };
    },
  } as unknown as OpenCompanyClient;
  return { client, posts };
}

let container: HTMLDivElement;
let root: Root;
let connectionId: ConnectionId;

async function show(client: OpenCompanyClient) {
  await act(async () => {
    root.render(createElement(AccountCard, { client, company: "alpha", connectionId }));
  });
}

function signOutButton(): HTMLButtonElement | null {
  return container.querySelector('[data-testid="settings-sign-out"]');
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  localStorage.clear();
  resetConnections();
  connectionId = addConnection({ baseUrl: "https://acme.example" });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  resetConnections();
  localStorage.clear();
});

describe("the Account card in General settings", () => {
  it("names who you are signed in as, and offers Sign out", async () => {
    const { client } = host("email");
    await show(client);
    expect(container.textContent).toContain("Ada Lovelace");
    expect(container.textContent).toContain("ada@alpha.test");
    expect(signOutButton()).toBeTruthy();
  });

  it("renders nothing on a company with no sign-in", async () => {
    const { client } = host("none");
    await show(client);
    expect(container.querySelector('[data-testid="settings-account"]')).toBeNull();
  });

  it("revokes the session and hands the connection back to the sign-in screen", async () => {
    const { client, posts } = host("email");
    await show(client);

    await act(async () => {
      signOutButton()?.click();
    });

    expect(posts).toEqual(["/api/v1/companies/alpha/auth/logout"]);
    // What `ConnectionConsole` reads to draw `Login` instead of the shell.
    expect(getConnection(connectionId)?.status).toBe("unauthenticated");
    // And nothing is carried forward for a reload to sign back in with.
    expect(getConnection(connectionId)?.credential).toEqual({ kind: "cookie" });
  });

  it("stays signed in when the host refuses the logout", async () => {
    const { client } = host("email", true);
    await show(client);

    await act(async () => {
      signOutButton()?.click();
    });

    // The local credential must survive a failed revoke: dropping it would put
    // this console on a sign-in screen while the host still honours the session.
    expect(getConnection(connectionId)?.status).not.toBe("unauthenticated");
    expect(signOutButton()?.disabled).toBe(false);
  });
});
