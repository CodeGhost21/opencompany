// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { SidebarUtilityBar } from "@/components/sidebar-controls";
import { NAV_SECTIONS } from "@/components/sidebar-navigation";
import { TITLE_BAR_LADDER } from "@/components/window-title-bar";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * Overview is reachable at every width, in exactly one place.
 *
 * This is a regression test for an intersection rather than for either change
 * that caused it. #1980 moved Overview out of the sidebar into a title-row
 * glyph and made it the FIRST rung of the degradation ladder
 * (`hidden md:inline-flex`) — correct on its own terms: a destination you
 * choose can go before a count that chooses you. The four-row restructure then
 * removed Overview's sidebar row — also correct on its own terms.
 *
 * Together they left phone-sized viewports with no UI path to the page at all.
 * Confirmed in a browser at 390px: the title-row slot was `display: none`, the
 * mobile sheet held only Room / Company / Connections / Flows and the three
 * utilities, and zero controls named Overview existed anywhere on the page. The
 * only way in was to know the address and type it.
 *
 * What this pins is the COMPLEMENTARITY, not either class: the sidebar's
 * fallback must appear exactly where the title row's does not. Asserting only
 * "the fallback has md:hidden" would still pass if someone later relaxed the
 * ladder to show Overview at all widths, and Overview would then be in two
 * places at once on a phone.
 */

let container: HTMLDivElement;
let root: Root;

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
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(view = "chat") {
  act(() =>
    root.render(
      createElement(
        SidebarProvider,
        null,
        createElement(SidebarUtilityBar, { view: view as never, onNavigate: () => {} }),
      ),
    ),
  );
}

describe("Overview survives having lost its nav row", () => {
  it("is not one of the four fixed sections", () => {
    // The premise. If Overview ever comes back as a row, this whole fallback
    // should go with it rather than becoming a second way in.
    expect(NAV_SECTIONS.map((s) => s.view)).not.toContain("overview");
  });

  it("offers a sidebar fallback exactly where the title row drops it", () => {
    render();
    const fallback = container.querySelector("[data-testid=sidebar-overview-fallback]");
    expect(fallback, "the mobile sheet has no way to reach Overview").not.toBeNull();

    // The complement, asserted as a relationship. The ladder reveals at `md`;
    // the fallback must hide at exactly the same breakpoint.
    const item = fallback!.closest("li")!;
    expect(TITLE_BAR_LADDER.overview).toBe("hidden md:inline-flex");
    expect(item.className).toContain("md:hidden");
  });

  it("marks itself current when Overview is the open view", () => {
    render("overview");
    const fallback = container.querySelector("[data-testid=sidebar-overview-fallback]")!;
    expect(fallback.getAttribute("aria-current")).toBe("page");
  });

  it("says nothing about the current page when it is not open", () => {
    render("chat");
    const fallback = container.querySelector("[data-testid=sidebar-overview-fallback]")!;
    // Absent rather than `aria-current="false"`, which some readers announce.
    expect(fallback.getAttribute("aria-current")).toBeNull();
  });
});
