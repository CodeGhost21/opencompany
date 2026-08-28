import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * Expanding the compact rail keeps keyboard focus on a rail toggle
 * (issue #1340 focus review).
 *
 * The compact rail's expand button only exists in the collapsed branch, so
 * expanding it unmounts the very control that carried focus and a keyboard
 * user falls out to the document. The shell cannot know the button's DOM
 * position (the header owns the toggle that is mounted on *both* density
 * states), so the expand action hands focus to that header toggle instead.
 *
 * A jsdom render of `ChatView` cannot prove this — it needs the whole client
 * and every hook. So this guards the *wiring contract* the fix rests on, the
 * same source-contract idiom as `responsive-two-rail-band.test.ts`: the rail
 * fires `onExpand`, the shell hands focus to the header toggle's ref, and the
 * header actually puts that ref on the density toggle.
 */

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("expanding the compact rail preserves focus (issue #1340)", () => {
  const rail = read("views/chat/ChannelRail.tsx");
  const chatView = read("views/ChatView.tsx");
  const chatHeader = read("views/chat/ChatHeader.tsx");

  it("keeps the compact rail's expand button the only expand affordance in the collapsed branch", () => {
    // The button only renders while collapsed, which is exactly why expanding
    // it strips focus — the premise the rest of this wiring exists to fix.
    const idx = rail.indexOf('aria-label="Expand channels"');
    expect(idx).toBeGreaterThan(-1);
    const button = rail.slice(Math.max(0, idx - 400), idx);
    expect(button).toContain("onClick={onExpand}");
  });

  it("hands focus to the header's toggle ref when expanding", () => {
    // The ref starts life in the shell, next to the collapse state it pairs with.
    expect(chatView).toContain("const channelsToggleRef = useRef<HTMLButtonElement>(null);");
    // And the expand half of the toggle focuses it — the compact rail button is
    // about to unmount, so focus moves to the toggle that survives the switch.
    // `next` is the rail's new collapsed state, so expanding is `!next`.
    expect(chatView).toContain("if (!next) channelsToggleRef.current?.focus();");
  });

  it("passes that ref to the header", () => {
    expect(chatView).toContain("channelsToggleRef={channelsToggleRef}");
  });

  it("puts the ref on the header's density toggle, which is mounted in both states", () => {
    // Anchor on the density toggle and confirm the ref lands on *it*, not on
    // the mobile "Show channels" button a few lines above.
    const idx = chatHeader.indexOf(
      'aria-label={channelsCollapsed ? "Expand channels" : "Collapse channels"}',
    );
    expect(idx).toBeGreaterThan(-1);
    const button = chatHeader.slice(Math.max(0, idx - 400), idx);
    expect(button).toContain("ref={channelsToggleRef}");
  });
});
