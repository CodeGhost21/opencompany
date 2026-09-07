// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { VirtualList } from "@/components/virtual-list";

let container: HTMLDivElement;
let root: Root;

const ROW_HEIGHT = 40;

const elementHeights = new WeakMap<Element, number>();

class FakeResizeObserver {
  static instances: FakeResizeObserver[] = [];
  readonly observed = new Set<Element>();

  constructor(readonly callback: ResizeObserverCallback) {
    FakeResizeObserver.instances.push(this);
  }

  observe(el: Element) {
    this.observed.add(el);
  }

  unobserve(el: Element) {
    this.observed.delete(el);
  }

  disconnect() {
    this.observed.clear();
  }
}

function stubRect(height: number) {
  return { top: 0, left: 0, right: 300, bottom: height, width: 300, height, x: 0, y: 0, toJSON() {} };
}

function rows() {
  return [{ id: "r0" }, { id: "r1" }, { id: "r2" }];
}

async function mount() {
  await act(async () => {
    root.render(
      createElement(VirtualList<{ id: string }>, {
        items: rows(),
        scrollElement: null,
        renderItem: (item) => createElement("div", { "data-testid": `cell-${item.id}` }, item.id),
        getKey: (item) => item.id,
        estimateRowHeight: ROW_HEIGHT,
        gap: 0,
        "data-testid": "vl",
      }),
    );
  });
  // The first layout-effect pass moves rows off the estimate and bumps once;
  // that re-render's own layout effect settles with no further diff.
  await act(async () => {});
  await act(async () => {});
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  FakeResizeObserver.instances = [];
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
  Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
    configurable: true,
    value(this: HTMLElement) {
      return stubRect(elementHeights.get(this) ?? ROW_HEIGHT);
    },
  });
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => {
    root.unmount();
  });
  container.remove();
});

describe("VirtualList re-measures a row that resizes without a React render", () => {
  it("subscribes every rendered row to a ResizeObserver", async () => {
    await mount();
    expect(FakeResizeObserver.instances.length).toBe(1);
    expect(FakeResizeObserver.instances[0]!.observed.size).toBe(3);
  });

  it("grows the list height when a row's content resizes with no state change (e.g. a loaded image)", async () => {
    await mount();
    const list = container.querySelector('[data-testid="vl"]') as HTMLDivElement;
    expect(list.style.height).toBe("120px");

    const observer = FakeResizeObserver.instances[0];
    expect(observer).toBeDefined();
    const [row] = observer!.observed;
    expect(row).toBeDefined();

    // The row's Markdown body grows after an image finishes loading — a DOM
    // size change with no React state update behind it, so nothing would
    // re-render on its own.
    elementHeights.set(row!, 90);
    await act(async () => {
      observer!.callback(
        [{ target: row, contentRect: stubRect(90) } as unknown as ResizeObserverEntry],
        observer as unknown as ResizeObserver,
      );
    });

    expect(list.style.height).toBe("170px");
  });
});
