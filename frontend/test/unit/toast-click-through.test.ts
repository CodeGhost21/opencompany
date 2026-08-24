// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";

import { relayToastClick, relayToastPointerDown } from "@/lib/toast-click-through";

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

/** A primary-button press at a known position, the gesture the relay serves. */
function press(element: Element, x = 40, y = 50, pointerId = 7): void {
  element.dispatchEvent(
    new PointerEvent("pointerdown", {
      bubbles: true,
      cancelable: true,
      clientX: x,
      clientY: y,
      pointerId,
      pointerType: "mouse",
      isPrimary: true,
      button: 0,
      buttons: 1,
    }),
  );
}

/** A primary-button click at a known position. */
function clickAt(element: Element, x: number, y: number, pointerId?: number): void {
  const event = new MouseEvent("click", {
    bubbles: true,
    cancelable: true,
    clientX: x,
    clientY: y,
    button: 0,
  });
  if (pointerId !== undefined) {
    Object.defineProperty(event, "pointerId", { value: pointerId });
  }
  element.dispatchEvent(event);
}

/** A page frame beneath the toast, relaying back to a stubbed `postMessage`. */
function frameBeneath(): { frame: HTMLIFrameElement; posted: ReturnType<typeof vi.fn> } {
  const frame = document.createElement("iframe");
  const posted = vi.fn();
  Object.defineProperty(frame, "contentWindow", { value: { postMessage: posted } });
  // jsdom has no layout; pin the frame's viewport offset so the relay's
  // frame-relative coordinates are observable.
  frame.getBoundingClientRect = () =>
    ({ left: 10, top: 20, width: 100, height: 100 }) as DOMRect;
  document.body.append(frame);
  return { frame, posted };
}

function toast(): HTMLElement {
  const toaster = document.createElement("section");
  toaster.dataset.sonnerToaster = "";
  const toast = document.createElement("article");
  toast.dataset.sonnerToast = "";
  toaster.append(toast);
  document.body.append(toaster);
  return toast;
}

describe("toast click-through", () => {
  it("relays a click on toast text to the page control below", () => {
    const below = document.createElement("button");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("click", relayToastClick);
    text.click();

    expect(clicked).toHaveBeenCalledOnce();
  });

  it("focuses the control a relayed click on a leaf inside it activates", () => {
    const below = document.createElement("button");
    const leaf = document.createElement("span");
    below.append(leaf);
    document.body.append(below);
    mockElementFromPoint(leaf);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("click", relayToastClick);
    text.click();

    // A native click on the icon/leaf inside the button focuses the button; the
    // relay must do the same rather than leaving keyboard focus where it was.
    expect(document.activeElement).toBe(below);
  });

  it("relays a click on toast text to an SVG control below", () => {
    const below = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const clicked = vi.fn();
    below.addEventListener("click", clicked);
    document.body.append(below);
    mockElementFromPoint(below);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("click", relayToastClick);
    text.click();

    expect(clicked).toHaveBeenCalledOnce();
  });

  it("leaves a toast action button as its own click target", () => {
    const below = document.createElement("button");
    const belowClicked = vi.fn();
    below.addEventListener("click", belowClicked);
    document.body.append(below);
    mockElementFromPoint(below);

    const action = document.createElement("button");
    const actionClicked = vi.fn();
    action.addEventListener("click", actionClicked);
    toast().append(action);
    action.addEventListener("click", relayToastClick);
    action.click();

    expect(actionClicked).toHaveBeenCalledOnce();
    expect(belowClicked).not.toHaveBeenCalled();
  });

  it("relays a press on toast text to the button below with the real gesture", () => {
    const below = document.createElement("button");
    const pressed = vi.fn();
    below.addEventListener("pointerdown", pressed);
    document.body.append(below);
    mockElementFromPoint(below);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);

    expect(pressed).toHaveBeenCalledOnce();
    // The synthetic pointerdown carries the original coordinates and pointer
    // id, so the control beneath sees the same gesture it would have seen.
    expect(pressed).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 40, clientY: 50, pointerId: 7 }),
    );
  });

  it("relays a press on toast text to an SVG control below", () => {
    const below = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const pressed = vi.fn();
    below.addEventListener("pointerdown", pressed);
    document.body.append(below);
    mockElementFromPoint(below);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text, 20, 30, 3);

    expect(pressed).toHaveBeenCalledOnce();
    expect(pressed).toHaveBeenCalledWith(
      expect.objectContaining({ clientX: 20, clientY: 30, pointerId: 3 }),
    );
  });

  it("leaves a toast action button as its own press target", () => {
    const below = document.createElement("button");
    const belowPressed = vi.fn();
    below.addEventListener("pointerdown", belowPressed);
    document.body.append(below);
    mockElementFromPoint(below);

    const action = document.createElement("button");
    const actionPressed = vi.fn();
    action.addEventListener("pointerdown", actionPressed);
    toast().append(action);
    action.addEventListener("pointerdown", relayToastPointerDown);
    press(action);

    expect(actionPressed).toHaveBeenCalledOnce();
    expect(belowPressed).not.toHaveBeenCalled();
  });

  it("does not relay a press when nothing but the toaster is beneath", () => {
    // The point under the press is still inside the toaster (another toast,
    // or the toaster's own chrome): relaying there would hand the gesture back
    // to the very element the toast is covering.
    const text = document.createElement("span");
    const toaster = toast();
    toaster.append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    mockElementFromPoint(toaster);

    press(text);

    // Nothing to assert a side effect on — the relay must simply not throw or
    // dispatch. The `elementFromPoint` mock would have received the call, and
    // the guard after it returns without dispatching.
    expect(document.elementFromPoint).toHaveBeenCalledOnce();
  });

  it("relays a click on toast text into a page frame beneath", () => {
    // A control rendered inside the Pages view is another document's element;
    // `elementFromPoint` answers with the frame host, and the relay must hand
    // the click to the frame over the bridge rather than click the host
    // (which nothing in the frame receives) or focus it (which steals
    // keyboard focus into the embedded document).
    const { frame, posted } = frameBeneath();
    const frameClicked = vi.fn();
    frame.addEventListener("click", frameClicked);
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("click", relayToastClick);
    clickAt(text, 40, 50, 7);

    expect(posted).toHaveBeenCalledWith(
      expect.objectContaining({ type: "oc:relay-click", x: 30, y: 30, pointerId: 7 }),
      "*",
    );
    expect(frameClicked).not.toHaveBeenCalled();
    expect(document.activeElement).not.toBe(frame);
  });

  it("relays a press on toast text into a page frame beneath", () => {
    const { frame, posted } = frameBeneath();
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);

    expect(posted).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "oc:relay-pointerdown",
        x: 30,
        y: 30,
        pointerId: 7,
        pointerType: "mouse",
        isPrimary: true,
        button: 0,
        buttons: 1,
      }),
      "*",
    );
  });

  it("forwards the tail of a frame press into the same frame", () => {
    // Pointer capture cannot reach into another document, so the rest of a
    // press that started on a toast over a frame lands back here and must be
    // relayed the same way the press was — otherwise a frame-side drag or
    // press-state control would receive a pointerdown it can never release.
    const { frame, posted } = frameBeneath();
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);

    window.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: 60,
        clientY: 70,
        pointerId: 7,
        pointerType: "mouse",
        isPrimary: true,
        button: -1,
        buttons: 1,
      }),
    );

    expect(posted).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "oc:relay-pointermove", x: 50, y: 50, pointerId: 7 }),
      "*",
    );
  });

  it("closes a frame press out on pointerup", () => {
    const { frame, posted } = frameBeneath();
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);

    window.dispatchEvent(
      new PointerEvent("pointerup", {
        clientX: 60,
        clientY: 70,
        pointerId: 7,
        pointerType: "mouse",
        button: 0,
        buttons: 0,
      }),
    );

    expect(posted).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "oc:relay-pointerup", x: 50, y: 50, pointerId: 7 }),
      "*",
    );

    // The press is over: a later move is no longer forwarded to the frame.
    window.dispatchEvent(
      new PointerEvent("pointermove", {
        clientX: 80,
        clientY: 90,
        pointerId: 7,
        pointerType: "mouse",
        button: -1,
        buttons: 0,
      }),
    );
    expect(posted).toHaveBeenCalledTimes(2); // pointerdown + pointerup only
  });

  it("closes a frame press out on pointercancel", () => {
    const { frame, posted } = frameBeneath();
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);

    window.dispatchEvent(
      new PointerEvent("pointercancel", { clientX: 60, clientY: 70, pointerId: 7 }),
    );

    expect(posted).toHaveBeenLastCalledWith(
      expect.objectContaining({ type: "oc:relay-pointercancel", x: 50, y: 50, pointerId: 7 }),
      "*",
    );

    // The press is over: a later move is no longer forwarded to the frame.
    window.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 80, clientY: 90, pointerId: 7 }),
    );
    expect(posted).toHaveBeenCalledTimes(2); // pointerdown + pointercancel only
  });

  it("drops the tail of a frame press when the frame leaves the document", () => {
    const { frame, posted } = frameBeneath();
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);
    press(text);
    posted.mockClear();

    // The Pages view closed mid-press: nothing is left to deliver the tail to.
    frame.remove();
    window.dispatchEvent(
      new PointerEvent("pointermove", { clientX: 60, clientY: 70, pointerId: 7 }),
    );
    window.dispatchEvent(new PointerEvent("pointerup", { clientX: 60, clientY: 70, pointerId: 7 }));

    expect(posted).not.toHaveBeenCalled();
  });

  it("does not relay a press into a frame with no loaded document", () => {
    // A detached or not-yet-loaded frame has no `contentWindow` to hand the
    // gesture to; the relay must simply give up rather than throw.
    const frame = document.createElement("iframe");
    Object.defineProperty(frame, "contentWindow", { value: null });
    document.body.append(frame);
    mockElementFromPoint(frame);

    const text = document.createElement("span");
    toast().append(text);
    text.addEventListener("pointerdown", relayToastPointerDown);

    expect(() => press(text)).not.toThrow();
  });
});
