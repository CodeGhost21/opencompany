// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SidebarUtilityBar } from "@/components/sidebar-controls";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * The sidebar's utility bar: Settings, Feedback and Discord, as three labelled
 * rows at the foot of the column.
 *
 * They were icon-only buttons while the bar sat *above* the destinations, where
 * labelled rows would have pushed the company's own state further down the
 * column every time the nav list grew. In the footer there is nothing left to
 * push, and three unlabelled glyphs under a column whose every other entry says
 * what it is were the cost of that shape. What keeps them from reading as
 * destinations is now position — after the list — rather than a different
 * shape.
 *
 * Rendering rather than pure functions: the accessible NAME is the whole of
 * what a screen reader gets, and it now comes from the row's own visible text
 * rather than an `aria-label` — so these assertions pin the label a sighted
 * operator reads and the one announced to a reader at the same time. On the
 * collapsed rail the text is clipped and the row's tooltip carries it, which is
 * what the nav rows beside it already do.
 */

let host: HTMLDivElement;
let root: Root | null = null;

/** jsdom ships no `matchMedia`, and `SidebarProvider` reaches for it. */
function stubMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
      onchange: null,
    }),
  });
}

function render(view: "overview" | "settings" | "feedback", onNavigate = () => {}) {
  act(() => {
    root!.render(
      createElement(SidebarProvider, {
        children: createElement(SidebarUtilityBar, { view, onNavigate }),
      }),
    );
  });
}

/**
 * The bar's controls, by their accessible name.
 *
 * The name is the row's text content now, not an `aria-label`, so this walks
 * the rendered controls and matches on what is actually on screen. An
 * `aria-label` lookup would have kept passing against a row that had lost its
 * label entirely, which is the regression these tests exist to catch.
 */
function byName(name: string): HTMLElement | null {
  const controls = host.querySelectorAll<HTMLElement>("button, a");
  for (const control of controls) {
    if (control.textContent?.trim() === name) return control;
  }
  return null;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  stubMatchMedia();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
});

describe("the sidebar utility bar", () => {
  it("names every control it carries", () => {
    render("overview");

    // The three, in the order they are read. Discord keeps its full label —
    // "Discord" alone would not say that the control leaves the console.
    //
    // Collapse is deliberately NOT here any more. It used to be, which put the
    // control that hides this panel inside the panel it hides — collapsing took
    // the button with it. It rides the content card's leading corner instead,
    // where it survives both states.
    for (const name of ["Settings", "Feedback", "Join our Discord"]) {
      expect(byName(name), name).not.toBeNull();
    }
  });

  it("is a named group, so the bar itself is addressable", () => {
    render("overview");

    const group = host.querySelector('[role="group"]');
    expect(group?.getAttribute("aria-label")).toBe("Console utilities");
  });

  it("marks the control whose page is open, and only that one", () => {
    render("settings");

    // `aria-current="page"` is what the nav rows announce for the open page,
    // and these are the same kind of claim made by a smaller control.
    expect(byName("Settings")?.getAttribute("aria-current")).toBe("page");
    // Absent rather than "false": some readers announce `aria-current="false"`.
    expect(byName("Feedback")?.getAttribute("aria-current")).toBeNull();

    render("feedback");
    expect(byName("Feedback")?.getAttribute("aria-current")).toBe("page");
    expect(byName("Settings")?.getAttribute("aria-current")).toBeNull();
  });

  it("navigates on a click", () => {
    const onNavigate = vi.fn();
    render("overview", onNavigate);

    act(() => {
      byName("Settings")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onNavigate).toHaveBeenCalledWith("settings");

    act(() => {
      byName("Feedback")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onNavigate).toHaveBeenCalledWith("feedback");
  });

  it("leaves the console for Discord instead of routing to it", () => {
    render("overview");

    const discord = byName("Join our Discord");
    expect(discord?.tagName).toBe("A");
    expect(discord?.getAttribute("href")).toContain("discord");
    // `noreferrer` with `_blank` — an external tab must not get a handle back
    // onto the console's window.
    expect(discord?.getAttribute("target")).toBe("_blank");
    expect(discord?.getAttribute("rel")).toContain("noreferrer");
  });

  it("keeps the guided tour's Settings anchor", () => {
    render("overview");

    // The tour's "Connect your tools" stop spotlights `nav-settings`. It named
    // the nav row until Settings moved onto this bar; the attribute moved with
    // it, or the stop would anchor on nothing and be skipped silently.
    expect(byName("Settings")?.getAttribute("data-tour")).toBe("nav-settings");
  });
});
