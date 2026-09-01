// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { RoomRailSlotProvider, useRoomRailSlot } from "@/components/room-rail";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * Picking a channel closes the sidebar's mobile sheet (codex P2 review, #1987).
 *
 * On a phone the sidebar is a sheet over the whole screen, and since the
 * four-row restructure the channel list is painted *inside* it. Navigating
 * without closing it leaves the operator staring at the list they just chose
 * from while the transcript they chose sits underneath — they have to dismiss
 * the sheet by hand to see the thing they asked for. Every other row in this
 * sidebar already closes it (`SidebarNavigation` calls `setOpenMobile(false)`),
 * and the channel list is one of its sections now, so it has to behave the same.
 *
 * `dismiss` is the other half of `reveal`, and it follows the same rule the rest
 * of this codebase follows for a control that would do nothing: it is
 * **absent** rather than present-and-inert at every width where the rail is a
 * column beside the transcript. That asymmetry is the thing worth pinning — a
 * `dismiss` that were always defined would silently close the desktop sidebar
 * every time an operator clicked a channel.
 */

const here = dirname(fileURLToPath(import.meta.url));
const chatView = readFileSync(resolve(here, "../../src/views/ChatView.tsx"), "utf8");

let container: HTMLDivElement;
let root: Root;

/** The slot the provider hands down, captured from inside the tree. */
let slot: ReturnType<typeof useRoomRailSlot> | null = null;

function Probe() {
  slot = useRoomRailSlot();
  return null;
}

function render(): ReactNode {
  act(() =>
    root.render(
      createElement(
        SidebarProvider,
        null,
        createElement(RoomRailSlotProvider, null, createElement(Probe)),
      ),
    ),
  );
  return null;
}

/** `useIsMobile` reads `window.innerWidth`, not the media query's `matches`. */
function setViewportWidth(width: number) {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    writable: true,
    value: width,
  });
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
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  slot = null;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the Room slot's mobile sheet controls", () => {
  it("offers reveal AND dismiss on a phone", () => {
    setViewportWidth(500);
    render();
    expect(slot?.reveal, "the chat header opens the sheet with this").toBeTypeOf("function");
    expect(slot?.dismiss, "picking a channel closes the sheet with this").toBeTypeOf("function");
  });

  it("actually closes the sheet, so the transcript is what you see next", () => {
    setViewportWidth(500);
    render();

    // Open it the way the chat header does.
    act(() => slot?.reveal?.());
    expect(slot?.covering, "the sheet is over the transcript once revealed").toBe(true);

    // Then pick a channel, which is what `ChatView.selectChannel` does.
    act(() => slot?.dismiss?.());
    expect(slot?.covering, "the sheet is gone, so the transcript is on screen").toBe(false);
  });

  it("offers neither at a width where the rail is a column beside the transcript", () => {
    setViewportWidth(1280);
    render();
    // Absent, not inert: an always-defined `dismiss` would collapse the desktop
    // sidebar on every channel click, which is the bug the asymmetry prevents.
    expect(slot?.reveal).toBeUndefined();
    expect(slot?.dismiss).toBeUndefined();
  });
});

describe("ChatView hands channel selection to that control", () => {
  /**
   * The body of `selectChannel`, with comments stripped.
   *
   * Stripped because a `toContain` against raw source is satisfied by a
   * **commented-out** call that runs nothing — the same shape that left
   * `#/pages` unreachable for four months (issue #1311), and the trap
   * `sidebar-sections.test.ts` calls out at the top of this suite's sibling.
   */
  const body = (() => {
    const start = chatView.indexOf("function selectChannel(");
    expect(start, "ChatView still has a selectChannel").toBeGreaterThan(-1);
    const open = chatView.indexOf("{", start);
    const end = chatView.indexOf("\n  }", open);
    return chatView
      .slice(open, end)
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n");
  })();

  it("navigates and then closes the sheet", () => {
    expect(body).toContain("onNavigate(id)");
    expect(body).toContain("roomRail.dismiss?.()");
  });

  it("calls it optionally, so desktop — where it is undefined — is a no-op", () => {
    // `roomRail.dismiss()` without the `?.` throws on every desktop click.
    expect(body).not.toMatch(/roomRail\.dismiss\(\)/);
  });
});
