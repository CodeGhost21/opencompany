// Overview is reachable at every width, in exactly one place.
//
// This is a regression test for an intersection rather than for any one change.
// #1980 moved Overview out of the sidebar into a title-row glyph and made it the
// first rung of the degradation ladder (`hidden md:inline-flex`) — correct on
// its own terms: a destination you *choose* can go before a count that chooses
// you. The four-row restructure then removed Overview's sidebar row — also
// correct on its own terms.
//
// Together they left phone-sized viewports with no UI path to the page at all.
// Confirmed in a browser at 390px: the title-row slot was `display: none`, the
// mobile sheet held only Room / Company / Connections / Automations, and zero
// controls named Overview existed anywhere on the page. The only way in was to
// know the address and type it.
//
// # Why this file no longer drives a component
//
// The fix at the time was a second Overview row the sidebar's footer drew
// `md:hidden` — the exact complement of the ladder rung, so the destination was
// on screen once at every width and never twice. This file pinned that
// complementarity.
//
// That footer is gone: Settings, Feedback and Discord became glyphs in the
// title row, and the Overview fallback had nowhere left to live. So the rung
// itself was retired instead — `TITLE_BAR_LADDER.overview` is `inline-flex` at
// every width — and the title-row glyph is now the one, always-present way in.
//
// What survives is the same guarantee stated against the arrangement that
// replaced it, and it is the half that can still regress silently: re-adding a
// breakpoint to that rung would restore the original P1 exactly, and would look
// correct on every desktop viewport while doing it.

import { describe, expect, it } from "vitest";

import { NAV_SECTIONS } from "@/components/sidebar-navigation";
import { TITLE_BAR_LADDER } from "@/components/window-title-bar";

describe("Overview survives having lost its nav row", () => {
  it("is not one of the fixed sidebar sections", () => {
    // The premise. If Overview ever comes back as a row, the title-row glyph
    // should go with it rather than becoming a second way in.
    expect(NAV_SECTIONS.map((s) => s.view)).not.toContain("overview");
  });

  it("keeps its title-row glyph at every width, since nothing backs it up", () => {
    // No breakpoint of any kind. Not "no `hidden`" — a `sm:`/`md:`/`lg:` rung
    // of any shape reintroduces a width at which Overview is unreachable, and
    // there is no sidebar fallback left to cover it.
    expect(TITLE_BAR_LADDER.overview).toBe("inline-flex");
    expect(TITLE_BAR_LADDER.overview).not.toMatch(/\b(hidden|[a-z]{2}:)/);
  });

  it("has no sidebar footer left to hold a fallback", () => {
    // `SidebarUtilityBar` used to draw one. It was deleted rather than left
    // exported and unrendered, so this asserts the module surface: a fallback
    // row reappearing there while the rung above shows at every width would put
    // Overview in two places at once on a phone, which is the failure the
    // original pairing existed to avoid in the other direction.
    const controls = import.meta.glob("@/components/sidebar-controls.tsx", {
      query: "?raw",
      import: "default",
      eager: true,
    }) as Record<string, string>;
    const source = Object.values(controls)[0];
    expect(source, "sidebar-controls.tsx not found").toBeTruthy();
    expect(source).not.toContain("sidebar-overview-fallback");
  });
});
