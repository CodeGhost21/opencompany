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
  it("clicks the element under a relayed click point with the relayed coordinates", () => {
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, { type: "oc:relay-click", x: 12, y: 34 });

    expect(clicked).toHaveBeenCalledOnce();
    // A canvas, chart or image-style control reads the click coordinates; the
    // relay must not hand it a zeroed click.
    expect(clicked).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 12, clientY: 34 }),
    );
  });

  it("focuses the control a relayed click on a leaf inside it activates", () => {
    const below = document.createElement("button");
    const leaf = document.createElement("span");
    below.append(leaf);
    document.body.append(below);
    mockElementFromPoint(leaf);

    relayFrom(window.parent, { type: "oc:relay-click", x: 12, y: 34 });

    // A native click on the icon/leaf inside the button focuses the button; the
    // relay must do the same rather than leaving keyboard focus where it was.
    expect(document.activeElement).toBe(below);
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

  it("routes the tail of a relayed press to the element that took the press", () => {
    // A press is a whole gesture: the parent relays its pointermove tail too,
    // and each continuation must reach the element that took the press, not
    // the one under the point now — the same retargeting pointer capture gives
    // a same-document control, so a drag keeps tracking its handle.
    const below = document.createElement("button");
    const moved = vi.fn();
    below.addEventListener("pointermove", moved);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });

    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => document.createElement("aside")),
    });
    relayFrom(window.parent, {
      type: "oc:relay-pointermove",
      x: 40,
      y: 50,
      pointerId: 7,
      pointerType: "mouse",
      button: -1,
      buttons: 1,
    });

    expect(moved).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 40, clientY: 50, pointerId: 7 }),
    );
  });

  it("closes a relayed press out on pointerup", () => {
    const below = document.createElement("button");
    const up = vi.fn();
    below.addEventListener("pointerup", up);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    relayFrom(window.parent, {
      type: "oc:relay-pointerup",
      x: 40,
      y: 50,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 0,
    });

    expect(up).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 40, clientY: 50, pointerId: 7 }),
    );

    // The press is over: a later move falls back to the element under the
    // point instead of the released one.
    const other = document.createElement("button");
    const otherMoved = vi.fn();
    other.addEventListener("pointermove", otherMoved);
    document.body.append(other);
    mockElementFromPoint(other);
    relayFrom(window.parent, {
      type: "oc:relay-pointermove",
      x: 55,
      y: 65,
      pointerId: 7,
      pointerType: "mouse",
      button: -1,
      buttons: 0,
    });

    expect(otherMoved).toHaveBeenCalledOnce();
  });

  it("closes a relayed press out on pointercancel", () => {
    const below = document.createElement("button");
    const canceled = vi.fn();
    below.addEventListener("pointercancel", canceled);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    relayFrom(window.parent, {
      type: "oc:relay-pointercancel",
      x: 40,
      y: 50,
      pointerId: 7,
      pointerType: "mouse",
    });

    expect(canceled).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 40, clientY: 50, pointerId: 7 }),
    );
  });

  it("does not relay a click that ends a drag onto the element under the release point", () => {
    // The parent relays a compatibility click after a press's `pointerup`, and
    // the pointer tail has been routed to the element that took the press
    // (capture semantics). A press that moved is a drag; its release must not
    // activate whatever happens to be under the release point.
    const pressTarget = document.createElement("button");
    const pressClicked = vi.fn();
    pressTarget.addEventListener("click", pressClicked);
    document.body.append(pressTarget);
    mockElementFromPoint(pressTarget);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 10,
      y: 10,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });

    const releaseTarget = document.createElement("button");
    const releaseClicked = vi.fn();
    releaseTarget.addEventListener("click", releaseClicked);
    document.body.append(releaseTarget);
    mockElementFromPoint(releaseTarget);

    relayFrom(window.parent, {
      type: "oc:relay-pointerup",
      x: 60,
      y: 60,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 0,
    });
    relayFrom(window.parent, {
      type: "oc:relay-click",
      x: 60,
      y: 60,
      pointerId: 7,
    });

    expect(pressClicked).toHaveBeenCalledOnce();
    expect(releaseClicked).not.toHaveBeenCalled();
  });

  it("relays a click after a press that did not move", () => {
    // The drag test above must not sweep up a plain click: a press released
    // where it started is a click, and the element under the point receives it.
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    relayFrom(window.parent, {
      type: "oc:relay-pointerup",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 0,
    });
    relayFrom(window.parent, {
      type: "oc:relay-click",
      x: 12,
      y: 34,
      pointerId: 7,
    });

    expect(clicked).toHaveBeenCalledOnce();
  });

  it("keeps a click tied to its own pointer press", () => {
    // Pointer ids let the frame correlate a compatibility click with its
    // completed press, even when another pointer press ended more recently.
    // The first drag's click must target its press element, while the second
    // press's click must target its own element.
    const pressTarget = document.createElement("button");
    const pressClicked = vi.fn();
    pressTarget.addEventListener("click", pressClicked);
    document.body.append(pressTarget);

    const releaseTarget = document.createElement("button");
    const releaseClicked = vi.fn();
    releaseTarget.addEventListener("click", releaseClicked);
    document.body.append(releaseTarget);

    // A drag that ends over `releaseTarget`, followed by a fresh press+click on it.
    mockElementFromPoint(pressTarget);
    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 10,
      y: 10,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    mockElementFromPoint(releaseTarget);
    relayFrom(window.parent, {
      type: "oc:relay-pointerup",
      x: 60,
      y: 60,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 0,
    });
    relayFrom(window.parent, {
      type: "oc:relay-click",
      x: 60,
      y: 60,
      pointerId: 7,
    });
    expect(pressClicked).toHaveBeenCalledOnce();
    expect(releaseClicked).not.toHaveBeenCalled();

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 60,
      y: 60,
      pointerId: 8,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    relayFrom(window.parent, {
      type: "oc:relay-pointerup",
      x: 60,
      y: 60,
      pointerId: 8,
      pointerType: "mouse",
      button: 0,
      buttons: 0,
    });
    relayFrom(window.parent, {
      type: "oc:relay-click",
      x: 60,
      y: 60,
      pointerId: 8,
    });

    expect(pressClicked).toHaveBeenCalledOnce();
    expect(releaseClicked).toHaveBeenCalledOnce();
  });

  it("falls back to the point when a relayed press's element is gone", () => {
    // The page re-rendered mid-gesture and the pressed element left the
    // document; the continuation routes by point, like a fresh press would.
    const below = document.createElement("button");
    document.body.append(below);
    mockElementFromPoint(below);

    relayFrom(window.parent, {
      type: "oc:relay-pointerdown",
      x: 12,
      y: 34,
      pointerId: 7,
      pointerType: "mouse",
      button: 0,
      buttons: 1,
    });
    below.remove();

    const other = document.createElement("button");
    const moved = vi.fn();
    other.addEventListener("pointermove", moved);
    document.body.append(other);
    mockElementFromPoint(other);
    relayFrom(window.parent, {
      type: "oc:relay-pointermove",
      x: 40,
      y: 50,
      pointerId: 7,
      pointerType: "mouse",
      button: -1,
      buttons: 1,
    });

    expect(moved).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 40, clientY: 50, pointerId: 7 }),
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
