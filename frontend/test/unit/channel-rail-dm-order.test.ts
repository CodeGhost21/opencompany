// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ChannelRail } from "@/views/room/ChannelRail";
import type { Channel, ChannelSection } from "@/views/room/model";

/**
 * The Direct messages order follows `latestMessageAt`, so a message re-sorts
 * the list. While focus (or the pointer) is inside the rail that order is held,
 * so a row cannot slide under a click and land it on the wrong DM (the #1414
 * hazard, reused via `useStableList`); it reconciles once the operator leaves.
 * Row CONTENT (unread badges) is not held — only the order.
 */

const dm = (id: string): Channel => ({ id, name: id, kind: "dm", purpose: "" });
const sections = (ids: string[]): ChannelSection[] => [
  { id: "dms", label: "Direct messages", channels: ids.map(dm) },
];

let container: HTMLDivElement;
let root: Root;

const render = (ids: string[], unread: Record<string, number> = {}) =>
  act(() =>
    root.render(
      createElement(ChannelRail, {
        sections: sections(ids),
        activeId: null,
        unread,
        onSelect: () => {},
      }),
    ),
  );

const order = () =>
  Array.from(container.querySelectorAll("li button")).map((b) => b.textContent?.trim() ?? "");

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

describe("ChannelRail DM order", () => {
  it("re-sorts immediately when nothing in the rail is hovered or focused", () => {
    render(["ann", "bob", "cy"]);
    render(["cy", "ann", "bob"]);
    expect(order()).toEqual(["cy", "ann", "bob"]);
  });

  it("holds the order while focus is in the rail, then reconciles on blur", () => {
    render(["ann", "bob", "cy"]);
    const first = container.querySelector<HTMLButtonElement>("li button")!;
    act(() => first.focus());
    render(["cy", "ann", "bob"]);
    expect(order()).toEqual(["ann", "bob", "cy"]);
    act(() => first.blur());
    expect(order()).toEqual(["cy", "ann", "bob"]);
  });

  it("keeps row content live while the order is held", () => {
    render(["ann", "bob", "cy"]);
    const first = container.querySelector<HTMLButtonElement>("li button")!;
    act(() => first.focus());
    render(["cy", "ann", "bob"], { cy: 3 });
    expect(order()[2]).toContain("3");
  });

  it("puts a DM that appears mid-hold last rather than shifting the rows", () => {
    render(["ann", "bob"]);
    const first = container.querySelector<HTMLButtonElement>("li button")!;
    act(() => first.focus());
    render(["new", "ann", "bob"]);
    expect(order()).toEqual(["ann", "bob", "new"]);
  });
});
