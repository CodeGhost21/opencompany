// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { SidebarProvider, useSidebar } from "@/components/ui/sidebar";

/**
 * The sidebar's density survives a reload — and therefore so does the channel
 * list's, now that the two are the same setting.
 *
 * `ChatView` used to persist its own `collapsed` flag per scope in
 * `localStorage` (`lib/chat-rail.ts`). The four-row restructure deleted that
 * helper and its test on the grounds that the channel list is a section of the
 * app sidebar now, so "the sidebar's state IS the rail's state — one control,
 * one persisted preference (the sidebar's cookie)".
 *
 * The cookie was write-only. `SidebarProvider` set `sidebar_state` on every
 * toggle, but nothing read it back: upstream shadcn reads that cookie on the
 * server and passes it in as `defaultOpen`, and this console has no server
 * render. So the sentence above was false in the one direction that mattered,
 * and the swap turned a persistent preference into a session-only one — an
 * operator collapsed the rail, reloaded, and got it back (codex P2 review).
 *
 * Verified in a browser before the fix: collapse → `data-state="collapsed"` and
 * `sidebar_state=false` written; reload → `data-state="expanded"`.
 *
 * These tests drive the provider rather than reading source, because the whole
 * failure was that a write existed and a read did not — source text would have
 * shown the cookie being set and looked correct.
 */

let container: HTMLDivElement;
let root: Root;
let api: ReturnType<typeof useSidebar> | null = null;

function Probe() {
  api = useSidebar();
  return null;
}

/** A fresh provider tree — the unit-test equivalent of reloading the page. */
function mount() {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => root.render(createElement(SidebarProvider, null, createElement(Probe))));
}

function unmount() {
  act(() => root.unmount());
  container.remove();
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
  // Clear the cookie between tests so each starts from "never chosen".
  document.cookie = "sidebar_state=; path=/; max-age=0";
  api = null;
});

afterEach(() => {
  document.cookie = "sidebar_state=; path=/; max-age=0";
});

describe("the sidebar's collapsed state persists", () => {
  it("opens by default when nothing has been chosen", () => {
    mount();
    expect(api?.state).toBe("expanded");
    unmount();
  });

  it("comes back collapsed after a remount", () => {
    mount();
    act(() => api?.setOpen(false));
    expect(api?.state, "collapsing did not take effect").toBe("collapsed");
    expect(document.cookie).toContain("sidebar_state=false");
    unmount();

    // The reload. A fresh provider must read what the last one wrote — this is
    // the assertion the deleted `chat-rail-preference.test.ts` used to make for
    // the rail, and which nothing made for the sidebar.
    mount();
    expect(api?.state, "the collapsed preference did not survive a reload").toBe("collapsed");
    unmount();
  });

  it("comes back expanded once re-opened", () => {
    document.cookie = "sidebar_state=false; path=/";
    mount();
    expect(api?.state).toBe("collapsed");
    act(() => api?.setOpen(true));
    unmount();

    mount();
    expect(api?.state).toBe("expanded");
    unmount();
  });

  it("ignores a cookie that is not a boolean rather than collapsing on it", () => {
    document.cookie = "sidebar_state=banana; path=/";
    mount();
    expect(api?.state).toBe("expanded");
    unmount();
  });
});
