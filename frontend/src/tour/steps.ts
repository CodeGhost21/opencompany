import type { Step } from "react-joyride";

import type { View } from "@/components/app-shell";
import { MAIN_THREAD_ID } from "@/lib/chat";

/**
 * One stop on the guided tour: a spotlight target plus the console view it
 * belongs to, so the controller can switch panes before highlighting.
 *
 * Anchoring strategy: most stops target a **sidebar nav** element
 * (`[data-tour="nav-<view>"]`), which sidesteps the lazy/Suspense race entirely
 * — navigating still swaps the main pane so the operator sees the view, but the
 * spotlight never waits on a code-split chunk. Only the chat composer anchors to
 * content, and Chat is not lazy.
 *
 * A content anchor has to be **addressed**, not just navigated to: a stop that
 * names only its view inherits whatever sub-page was last open there, and a
 * content anchor the remembered sub-page does not render is skipped in silence.
 * Both chat composer stops therefore carry an explicit `sub`. That matters more
 * since #1984, which stopped rendering a composer at all on the read-only
 * Operator feed — with Room remembering the last channel, an operator whose last
 * visit was `#Operator` would otherwise have had BOTH composer stops skip, and
 * a seven-stop tour would have silently taught five.
 *
 * Each nav stop therefore has to name a row that EXISTS. The anchor is
 * `nav-<view>`, not `nav-<label>`, because the two are deliberately allowed to
 * differ — the `chat` view's row says "Room", the `workflows` view's says
 * "Flows" — and an anchor should not move when a word does. What it must track
 * is the nav table (`components/sidebar-navigation.tsx`).
 *
 * ## Rebuilt, not patched, for the four-section sidebar
 *
 * Four of the eight stops this file used to hold pointed at rows that stopped
 * existing: `nav-approvals` and the two Overview stops (Approvals and Overview
 * are chrome in the window's title row now), and `nav-workflows`' stop was
 * titled "Workflows". A missing anchor degrades to a **skipped step** rather
 * than an error — `waitForTarget` below resolves `false` and the tour moves on
 * — so a stale tour does not break, it quietly teaches half the product and
 * nothing reports it. That is why this was rewritten rather than repaired, and
 * why `tour-anchors.test.ts` now asserts every stop's anchor against the nav
 * table instead of trusting the browser to complain.
 *
 * Seven stops, not eight. The Overview stop is gone rather than re-anchored:
 * Overview is not a place you navigate to from the sidebar any more, and a tour
 * that teaches a destination an operator cannot then find is worse than one
 * that leaves it for them to discover where it now lives.
 */
export interface TourStop {
  view: View;
  /** A section's sub-page, when the stop lives inside one (`#/connections/…`). */
  sub?: string;
  target: string;
  title: string;
  body: string;
  placement?: Step["placement"];
}

export const TOUR: TourStop[] = [
  {
    view: "chat",
    target: '[data-tour="sidebar"]',
    placement: "right",
    title: "Welcome to your company",
    body: "Four places: the Room you talk in, your Company, what it's Connected to, and the Flows it repeats. Open one and what's inside it appears underneath.",
  },
  {
    // `sub` is not optional here, and neither composer stop below may drop it.
    //
    // A bare `setView("chat")` restores whichever channel the operator was last
    // on — `app-shell`'s `lastSubByViewRef`, and `ChatView`'s own
    // `readLastChannel` for a cold start. That can be the read-only `#Operator`
    // feed, which renders no composer at all since PR #1984. The stop would then
    // wait out `targetWaitTimeout` and be **skipped** — silently, because a
    // missing anchor degrades rather than errors (see `waitForTarget`), so the
    // tour would simply teach less and say nothing about it. The last stop below
    // is "You're all set", so the tour would end by vanishing.
    //
    // `main` is the built-in company-wide channel, present in every company from
    // first boot (issue #1743) and always writable. A blueprint that
    // grandfathers a desk onto that line renders it under the desk's own id
    // instead; `ChatView` folds every General spelling onto whichever channel
    // actually holds the line (`generalChannelId`), so this address resolves
    // either way rather than raising issue #370's unknown-channel notice.
    view: "chat",
    sub: MAIN_THREAD_ID,
    target: '[data-tour="chat-composer"]',
    placement: "top",
    title: "Talk to your company",
    body: "Ask for an update or hand off a task in plain language — like messaging a teammate.",
  },
  {
    view: "chat",
    target: '[data-tour="nav-chat"]',
    placement: "right",
    title: "Your AI staff",
    body: "Every channel and direct message is listed here while you're in the Room. The teammates that do the work each have one.",
  },
  {
    view: "company",
    target: '[data-tour="nav-company"]',
    placement: "right",
    title: "Your company",
    body: "Who's on it, what they're working on, the files they keep, what they remember, and what it all costs — five pages under one row.",
  },
  {
    view: "workflows",
    target: '[data-tour="nav-workflows"]',
    placement: "right",
    title: "Flows",
    body: "Turn recurring work into a repeatable flow — a graph of steps your teammates run end to end.",
  },
  {
    // The accounts page is `#/connections/apps` since it left the settings
    // rail, so the stop navigates there and spotlights the row that leads to it.
    view: "connections",
    sub: "apps",
    target: '[data-tour="nav-connections"]',
    placement: "right",
    title: "Connect your tools",
    body: "Plug in the tools your company already uses — Gmail, Slack, Notion — so your teammates can act for real.",
  },
  {
    // Addressed, for the reason the first composer stop above gives at length.
    view: "chat",
    sub: MAIN_THREAD_ID,
    target: '[data-tour="chat-composer"]',
    placement: "top",
    title: "You're all set",
    body: "That's the tour. Say hi to your company to get going — you can replay this anytime from Settings.",
  },
];

/**
 * Poll for a step's target to mount. Route swaps and lazy Suspense chunks
 * resolve a tick or two after navigation, so a spotlight can't anchor on the
 * same frame it navigates. Resolves `false` on timeout so a missing anchor
 * degrades to a skipped step instead of wedging the tour. Mirrors OpenHuman's
 * `waitForTarget` (walkthroughSteps.ts).
 */
export function waitForTarget(selector: string, timeoutMs = 5000): Promise<boolean> {
  return new Promise((resolve) => {
    const start = Date.now();
    const tick = () => {
      if (document.querySelector(selector)) {
        resolve(true);
        return;
      }
      if (Date.now() - start > timeoutMs) {
        resolve(false);
        return;
      }
      window.setTimeout(tick, 50);
    };
    tick();
  });
}
