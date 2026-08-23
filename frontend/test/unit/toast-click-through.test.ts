// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";

import { relayToastClick } from "@/lib/toast-click-through";

afterEach(() => {
  document.body.replaceChildren();
  delete (document as Partial<Document>).elementFromPoint;
  vi.restoreAllMocks();
});

function mockElementFromPoint(element: HTMLElement): void {
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn(() => element),
  });
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
