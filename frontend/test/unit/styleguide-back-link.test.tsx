// @vitest-environment jsdom
//
// "Back to console" has to return the operator to the console they left, and on
// a desktop holding several hosts that is a scope, not just a page (issue
// #1358). `App` remounts `Console` on the way back, so a bare `#/overview`
// would leave `useHostRoute` to initialize from an absent parameter and land on
// whichever host the bootstrap fallback picks — a silent host switch.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { StyleguideView } from "@/views/StyleguideView";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  window.history.replaceState(null, "", "#/");
});

function render() {
  act(() => root.render(<StyleguideView />));
  const link = container.querySelector<HTMLAnchorElement>(
    '[data-testid="styleguide-header"] a',
  );
  expect(link).not.toBeNull();
  return link!.getAttribute("href");
}

describe("the styleguide's back link", () => {
  it("carries the host scope the styleguide was opened with", () => {
    window.history.replaceState(null, "", "#/styleguide?host=c-2");
    expect(render()).toBe("#/overview?host=c-2");
  });

  it("names no host when the address names none", () => {
    window.history.replaceState(null, "", "#/styleguide");
    expect(render()).toBe("#/overview");
  });
});
