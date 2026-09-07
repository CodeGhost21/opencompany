// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useMediaQuery } from "@/hooks/use-media-query";

let container: HTMLDivElement;
let root: Root;
let last: boolean | null;
let originalMatchMedia: typeof window.matchMedia | undefined;

/**
 * A `MediaQueryList` shaped like the older WebKitGTK builds Tauri v2's floor
 * still admits: no `EventTarget` methods, only the deprecated
 * `addListener`/`removeListener` pair.
 */
class LegacyMediaQueryList {
  matches = false;
  media: string;
  private listeners = new Set<(list: LegacyMediaQueryList) => void>();

  constructor(media: string) {
    this.media = media;
  }

  addListener(fn: (list: LegacyMediaQueryList) => void) {
    this.listeners.add(fn);
  }

  removeListener(fn: (list: LegacyMediaQueryList) => void) {
    this.listeners.delete(fn);
  }

  listenerCount() {
    return this.listeners.size;
  }

  fire() {
    this.matches = true;
    for (const fn of this.listeners) fn(this);
  }
}

function Probe({ query }: { query: string }) {
  last = useMediaQuery(query);
  return null;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  last = null;
  originalMatchMedia = window.matchMedia;
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
  if (originalMatchMedia) {
    window.matchMedia = originalMatchMedia;
  } else {
    // @ts-expect-error jsdom does not define matchMedia by default
    delete window.matchMedia;
  }
});

describe("useMediaQuery against a legacy MediaQueryList", () => {
  it("subscribes via addListener instead of throwing when addEventListener is absent", async () => {
    const legacy = new LegacyMediaQueryList("(min-width: 768px)");
    window.matchMedia = (() => legacy) as unknown as typeof window.matchMedia;

    await act(async () => {
      root.render(createElement(Probe, { query: "(min-width: 768px)" }));
    });

    expect(last).toBe(false);
    expect(legacy.listenerCount()).toBe(1);

    await act(async () => {
      legacy.fire();
    });
    expect(last).toBe(true);

    await act(async () => {
      root.unmount();
    });
    expect(legacy.listenerCount()).toBe(0);
  });
});
