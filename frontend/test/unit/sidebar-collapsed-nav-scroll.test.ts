import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The nav list must stay reachable when it does not fit — collapsed rail
 * included (issue #1931 review).
 *
 * At the desktop's supported minimum window height, the macOS traffic-light
 * strip plus a full nav list can exceed the rail's height. `SidebarContent`
 * used to answer that with `overflow-hidden`, which clipped the last rows out
 * of reach — not merely cut off, but not pointer-reachable either.
 *
 * # Why this no longer pins a collapsed-only rule
 *
 * The first fix was `group-data-[collapsible=icon]:overflow-y-auto` on
 * `SidebarContent` — the overflow answered once for the collapsed state.
 *
 * The whole column scrolls now, Slack-style: `sidebar-inner` carries one
 * `overflow-y-auto` and every region inside it is `flex-none`. That fixes the
 * collapsed rail *by construction* — there is one scrolling box, in every
 * state — so a rule keyed on `collapsible=icon` would be a second answer to a
 * question already settled, and its absence is not the regression it used to
 * be.
 *
 * What would be a regression is either half coming undone: the column losing
 * its scroller, or this region growing a clip of its own again. Both are pinned
 * below.
 *
 * jsdom evaluates none of these selectors (they are plain CSS keyed off
 * `data-*` attributes this suite never triggers a real layout pass for), so
 * this pins the source contract — the same idiom
 * `responsive-two-rail-band.test.ts` uses for a media-query fact jsdom cannot
 * evaluate either.
 */

const here = dirname(fileURLToPath(import.meta.url));
const sidebar = readFileSync(resolve(here, "../../src/components/ui/sidebar.tsx"), "utf8");

/** The `className` string of the element carrying `data-slot="<slot>"`. */
function classesFor(slot: string): string {
  const at = sidebar.indexOf(`data-slot="${slot}"`);
  expect(at, `no element with data-slot="${slot}"`).toBeGreaterThan(-1);
  const after = sidebar.slice(at);
  const match = after.match(/className=(?:"([^"]*)"|\{[^}]*?"([^"]*)"[\s\S]*?\})/);
  return match ? (match[1] ?? match[2] ?? "") : "";
}

describe("the sidebar column scrolls, in every state", () => {
  it("puts one scroller on the column itself, not on a region inside it", () => {
    const inner = classesFor("sidebar-inner");
    expect(inner).toContain("overflow-y-auto");
    // Unconditional. A `collapsible=icon` variant here would mean the expanded
    // and collapsed states scroll for different reasons, which is how the
    // collapsed one came to clip in the first place.
    expect(inner).not.toContain("group-data-[collapsible=icon]:overflow");
  });

  it("keeps the scrollbar out of sight until it is reached for", () => {
    // The column is standing furniture behind every view, so a permanent bar
    // is 10px of chrome on screen at all times. `scrollbar-on-hover`
    // (`index.css`) reveals the thumb on hover and never disables scrolling.
    expect(classesFor("sidebar-inner")).toContain("scrollbar-on-hover");
  });

  it("never clips the nav list again", () => {
    // The original bug, stated against the region that had it. It carries no
    // overflow of its own now — neither a scroll nor a clip — because the
    // column above it owns both.
    const content = classesFor("sidebar-content");
    expect(content).not.toContain("overflow-hidden");
    expect(content).not.toContain("group-data-[collapsible=icon]:overflow-hidden");
    // And it must not absorb the slack, or the channel list below it stops
    // being the thing that scrolls.
    expect(content).toContain("flex-none");
  });
});
