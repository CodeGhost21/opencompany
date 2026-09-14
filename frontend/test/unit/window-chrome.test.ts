// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { WindowControlsInset, WindowDragBar } from "@/components/window-chrome";

/**
 * The desktop window's own chrome, and — now entirely — its absence.
 *
 * `tauri.conf.json` used to run the main window with `titleBarStyle: "Overlay"`:
 * macOS drew no title bar and floated the traffic lights over the web content,
 * so the console put back a band that opts into dragging and reserved 72px so
 * the lights were not sitting on the company switcher.
 *
 * The window is `decorations: true` with no `titleBarStyle` now — an ordinary
 * macOS title bar, with the lights in it — so **neither piece renders anywhere**,
 * and that is what these tests assert.
 *
 * They are kept rather than deleted because the components are kept: flipping
 * `SHELL_DRAWS_ITS_OWN_TITLE_BAR` back in `window-chrome.tsx` restores the whole
 * arrangement, and the last case below is what says that switch still works. A
 * band that renders when the shell is NOT drawing its own chrome is a 28px strip
 * across the top of every page that silently swallows clicks, with nothing on
 * screen to explain it.
 */

let host: HTMLDivElement;
let root: Root | null = null;

/** Present the runtime as the Tauri desktop, on the given platform. */
function asDesktop(platform: string) {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  Object.defineProperty(navigator, "platform", {
    configurable: true,
    value: platform,
  });
}

function render(node: Parameters<Root["render"]>[0]) {
  act(() => root!.render(node));
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI__;
});

describe("the window drag band", () => {
  it("renders nothing in a browser", () => {
    // No `__TAURI__`: there is no window to drag, and a band here would only
    // eat the top of every page.
    render(createElement(WindowDragBar));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });

  it("renders nothing on a desktop that keeps its native title bar", () => {
    // `titleBarStyle: "Overlay"` is a macOS-only style — Windows and Linux draw
    // their real title bar, so reserving a band would waste 28px for nothing.
    asDesktop("Win32");
    render(createElement(WindowDragBar));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });

  it("renders nothing on macOS either, now that the title bar is native", () => {
    // The case that used to assert the band. macOS draws the title bar again,
    // so there is a real one to grab and a band over the content would only
    // swallow the clicks of whatever it covers.
    asDesktop("MacIntel");
    render(createElement(WindowDragBar));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });
});

describe("the traffic-light inset", () => {
  it("reserves nothing where the lights do not float", () => {
    render(createElement(WindowControlsInset));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();

    asDesktop("Linux x86_64");
    render(createElement(WindowControlsInset));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });

  it("reserves nothing on macOS either, now that the lights are in the title bar", () => {
    // The 72px this used to hold is the whole point of the change: reserved
    // while the shell drew its own chrome, it is a hole in the title row the
    // moment macOS draws the lights somewhere else.
    asDesktop("MacIntel");
    render(createElement(WindowControlsInset));
    expect(host.querySelector("[data-tauri-drag-region]")).toBeNull();
  });
});
