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
});
