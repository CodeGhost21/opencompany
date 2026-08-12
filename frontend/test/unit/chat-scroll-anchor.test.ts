// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ChatMessage } from "@/lib/chat";
import { MessageTimeline } from "@/views/chat/MessageTimeline";
import {
  buildTimeline,
  buildTimelineItems,
  type Channel,
  type TimelineItem,
} from "@/views/chat/model";

/**
 * Where a channel opens, and what is allowed to move it (issue #757).
 *
 * The transcript used to animate to the bottom on arrival: one effect, always
 * `behavior: "smooth"`, firing on mount as well as on growth. Opening a channel
 * therefore painted an un-anchored position and then slid down, and the longer
 * the transcript the longer the slide.
 *
 * jsdom performs no layout, so `scrollHeight` and `clientHeight` are 0 and
 * `scrollTo` does not exist. The geometry below is stubbed on the prototype for
 * exactly that reason — these tests are about *which* anchoring call the
 * component makes and when, which is the part that was wrong. They cannot and
 * do not claim anything about real pixel positions.
 */

const CONTENT_HEIGHT = 4000;
const VIEWPORT_HEIGHT = 800;
/** Every `scrollTo` the component made, in order. */
let calls: Array<{ top: number; behavior?: string }> = [];
let scrollTop = 0;
let container: HTMLDivElement;
let root: Root;

/** What each stubbed property looked like before, so it can be put back. */
const saved = new Map<string, PropertyDescriptor | undefined>();

const STUBS: Record<string, PropertyDescriptor> = {
  scrollHeight: { get: () => CONTENT_HEIGHT, configurable: true },
  clientHeight: { get: () => VIEWPORT_HEIGHT, configurable: true },
  scrollTop: {
    get: () => scrollTop,
    set: (v: number) => {
      scrollTop = v;
    },
    configurable: true,
  },
  scrollTo: {
    value: (opts: { top: number; behavior?: string }) => {
      calls.push(opts);
      scrollTop = opts.top;
    },
    configurable: true,
    writable: true,
  },
};

function stubGeometry() {
  for (const [prop, desc] of Object.entries(STUBS)) {
    saved.set(prop, Object.getOwnPropertyDescriptor(Element.prototype, prop));
    Object.defineProperty(Element.prototype, prop, desc);
  }
}

/**
 * Put the prototype back. Without this the stubs outlive the file inside a
 * shared worker, and the next suite to touch a scroll container inherits a
 * 4000px document it never asked for.
 */
function restoreGeometry() {
  for (const [prop, desc] of saved) {
    if (desc) Object.defineProperty(Element.prototype, prop, desc);
    else delete (Element.prototype as unknown as Record<string, unknown>)[prop];
  }
  saved.clear();
}

function channel(id: string): Channel {
  return { id, name: id, kind: "channel", purpose: "" };
}

/**
 * `n` message rows, built through the real timeline constructors so the rows
 * render as the app renders them. Only the count matters to the effects under
 * test, but a hand-rolled row shape would drift from `TimelineEntry` and fail
 * inside `MessageRow` rather than in an assertion.
 */
function items(n: number, ch: Channel): TimelineItem[] {
  const messages: ChatMessage[] = Array.from({ length: n }, (_, i) => ({
    id: `m${i}`,
    from: "you",
    text: `line ${i}`,
    at: 1_700_000_000_000 + i * 1_000,
  }));
  return buildTimelineItems(buildTimeline(messages, ch), []);
}

// `createElement` rather than JSX because the unit suite's vitest `include` is
// `*.test.ts` — a `.tsx` file is silently not collected, which reads as a
// passing suite.
function render(ch: Channel, rows: TimelineItem[]) {
  act(() => {
    root.render(
      createElement(MessageTimeline, {
        channel: ch,
        items: rows,
        openThreadId: null,
        typing: false,
        onOpenThread: () => {},
        onReact: () => {},
      }),
    );
  });
}

/** The scrolling body is the component's outermost element. */
function scroller(): HTMLElement {
  return container.firstElementChild as HTMLElement;
}

beforeEach(() => {
  // React only treats `act` as a real boundary when this is set; without it
  // effects still run but React warns, and the warning is the honest signal
  // that the flush is not being awaited the way the app flushes it.
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  calls = [];
  scrollTop = 0;
  stubGeometry();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  restoreGeometry();
});

describe("channel arrival", () => {
  it("anchors instantly and never animates on the way in", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));

    // The jump is a direct `scrollTop` write, not an animated `scrollTo`.
    expect(scrollTop).toBe(CONTENT_HEIGHT);
    expect(calls).toHaveLength(0);
  });

  it("re-anchors on a switch between channels holding the same number of rows", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));
    // Simulate the operator having scrolled up in the first channel.
    scrollTop = 0;

    // Same row count on purpose: the old effect keyed on `items.length` alone,
    // so this switch produced no effect at all and the new channel inherited
    // the previous scroll offset.
    const next = channel("product-design");
    render(next, items(40, next));

    expect(scrollTop).toBe(CONTENT_HEIGHT);
  });
});

describe("growth while the channel is open", () => {
  it("follows a new row when the operator is parked at the bottom", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));
    calls = [];

    render(ch, items(41, ch));

    expect(calls).toEqual([{ top: CONTENT_HEIGHT, behavior: "smooth" }]);
  });

  it("leaves the viewport alone when the operator has scrolled up to read", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));
    calls = [];

    // Scrolled well away from the bottom, and the component told so by the
    // scroll event its own handler listens for.
    scrollTop = 100;
    act(() => {
      scroller().dispatchEvent(new Event("scroll", { bubbles: true }));
    });

    render(ch, items(41, ch));

    expect(calls).toHaveLength(0);
    expect(scrollTop).toBe(100);
  });

  it("resumes following once the operator returns to the bottom", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));

    scrollTop = 100;
    act(() => {
      scroller().dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    render(ch, items(41, ch));
    expect(calls).toHaveLength(0);

    // Back to the bottom. `scrollHeight - scrollTop - clientHeight` is 0 here,
    // comfortably inside the slack the component allows.
    scrollTop = CONTENT_HEIGHT - VIEWPORT_HEIGHT;
    act(() => {
      scroller().dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    render(ch, items(42, ch));

    expect(calls).toEqual([{ top: CONTENT_HEIGHT, behavior: "smooth" }]);
  });

  it("still follows when the view is a few pixels short of the bottom", () => {
    const ch = channel("engineering");
    render(ch, items(40, ch));
    calls = [];

    // Sub-pixel layout leaves a small remainder in a real browser; a strict
    // equality test would read this as "scrolled away" and stop following.
    scrollTop = CONTENT_HEIGHT - VIEWPORT_HEIGHT - 8;
    act(() => {
      scroller().dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    render(ch, items(41, ch));

    expect(calls).toEqual([{ top: CONTENT_HEIGHT, behavior: "smooth" }]);
  });
});
