// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ChannelRail } from "@/views/chat/ChannelRail";
import type { ChannelSection } from "@/views/chat/model";

/**
 * The compact rail keeps an unread channel's count in its accessible name
 * (issue #364, P2 review).
 *
 * The expanded row announces unread because the count is text inside the
 * button; the collapsed row draws it as a bare dot, which is invisible to
 * screen readers. The fix puts the same count in the compact button's
 * `aria-label`, so collapsing the rail does not strip the fact from the
 * accessibility tree. These pin that label directly.
 */

const SECTIONS: ChannelSection[] = [
  {
    id: "s1",
    label: "Company",
    channels: [
      { id: "front-desk", name: "Front desk", kind: "channel", purpose: "The front line." },
      { id: "ops", name: "Ops", kind: "channel", purpose: "Where work lands." },
    ],
  },
];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const channelButtons = () =>
  [...container.querySelectorAll<HTMLButtonElement>('nav[aria-label="Channels"] button')].filter(
    (b) => b.getAttribute("aria-label") !== "Expand channels",
  );

describe("collapsed ChannelRail unread labels", () => {
  it("names an unread channel with its count", () => {
    act(() =>
      root.render(
        createElement(ChannelRail, {
          sections: SECTIONS,
          activeId: null,
          unread: { "front-desk": 3 },
          onSelect: () => {},
          collapsed: true,
        }),
      ),
    );

    const buttons = channelButtons();
    expect(buttons.map((b) => b.getAttribute("aria-label"))).toEqual([
      "Front desk, 3 unread",
      "Ops",
    ]);
  });

  it("caps a huge count the way the expanded badge does", () => {
    act(() =>
      root.render(
        createElement(ChannelRail, {
          sections: SECTIONS,
          activeId: null,
          unread: { "front-desk": 142 },
          onSelect: () => {},
          collapsed: true,
        }),
      ),
    );

    expect(channelButtons()[0].getAttribute("aria-label")).toBe("Front desk, 99+ unread");
  });

  it("keeps the active channel's label bare even when unread", () => {
    act(() =>
      root.render(
        createElement(ChannelRail, {
          sections: SECTIONS,
          activeId: "front-desk",
          unread: { "front-desk": 7 },
          onSelect: () => {},
          collapsed: true,
        }),
      ),
    );

    // The unread dot does not render on the channel you are already reading,
    // so the label must not claim unread either.
    expect(channelButtons()[0].getAttribute("aria-label")).toBe("Front desk");
  });
});
