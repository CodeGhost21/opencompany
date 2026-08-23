// @vitest-environment jsdom

// The frame side of the toast click relay (issue #1303): when a toast covers
// a control inside the console's sandboxed Pages view, the parent document
// cannot dispatch a DOM event into the frame — so the console posts the
// gesture's coordinates over the existing bridge and the page SDK turns them
// back into a real click or press on whatever element is beneath the point in
// this document. These tests exercise that listener, which lives at module
// scope in `pages-sdk/client.ts` alongside the bridge's `oc:init` handler.

import { afterEach, describe, expect, it, vi } from "vitest";

import "../../pages-sdk/client";

afterEach(() => {
  document.body.replaceChildren();
  delete (document as Partial<Document>).elementFromPoint;
  vi.restoreAllMocks();
});

function mockElementFromPoint(element: Element): void {
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn(() => element),
  });
}

/** Dispatch a message to the page window from an arbitrary `event.source`. */
function relayFrom(source: MessageEventSource | null, data: unknown): void {
  const event = new MessageEvent("message", { data });
  // `source` is a getter on the prototype; pin the caller on the instance so
  // the listener's `event.source !== window.parent` guard is observable in
  // jsdom, where `window.parent === window`.
  Object.defineProperty(event, "source", { value: source });
  window.dispatchEvent(event);
}

describe("the page frame's toast-relay listener", () => {
  it("clicks the element under a relayed click point", () => {
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, { type: "oc:relay-click", x: 12, y: 34 });

    expect(clicked).toHaveBeenCalledOnce();
  });

  it("dispatches a pointerdown for a relayed press", () => {
    const below = document.createElement("button");
    const pressed = vi.fn();
    below.addEventListener("pointerdown", pressed);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      isPrimary: true,
      button: 0,
      buttons: 1,
    });

    expect(pressed).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 12, clientY: 34, pointerId: 7 }),
    );
  });

  it("ignores relay messages from anyone but the console frame", () => {
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    // A frame the page embeds itself would surface as its own window, not
    // `window.parent`; its messages must not trigger clicks in the page.
    relayFrom({} as MessageEventSource, { type: "oc:relay-click", x: 12, y: 34 });

    expect(clicked).not.toHaveBeenCalled();
  });

  it("ignores malformed relay messages", () => {
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, { type: "oc:relay-click" });
    relayFrom(window.parent, { type: "oc:not-a-relay", x: 12, y: 34 });

    expect(clicked).not.toHaveBeenCalled();
  });

  it("is a no-op when nothing is beneath the relayed point", () => {
    // A coordinate outside the document (a point over the frame's own chrome,
    // or a stale frame-relative offset) has no element beneath it; the relay
    // must simply give up rather than throw.
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => null),
    });

    expect(() =>
      relayFrom(window.parent, { type: "oc:relay-click", x: 12, y: 34 }),
    ).not.toThrow();
  });
});
