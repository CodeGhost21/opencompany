// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog";

/**
 * The dialog footer is opaque, because it is `sticky`.
 *
 * `DialogContent` is `max-h-[85vh] overflow-y-auto` and `DialogFooter` is
 * `sticky bottom-[-1rem]`, so on any dialog whose body outgrows the viewport
 * the body keeps scrolling underneath the footer rather than pushing it down.
 * That is the intended behaviour — the submit button stays reachable — and it
 * only works if the bar the content passes behind is actually opaque.
 *
 * It was `bg-muted/50`. The desk creator (`DeskCreateDialog`) renders one
 * bordered row per roster teammate, so a company with a dozen agents scrolls,
 * and the rows that ran under the footer stayed legible *through* Cancel and
 * Create desk: teammate names printed across both buttons. It read as a
 * rendering fault, and it made the two controls the dialog exists for harder
 * to read than the list behind them.
 *
 * Asserted on the mounted node rather than by reading the source, so a future
 * `className` override that reintroduces a translucency modifier is caught
 * too — the class list is what the browser sees.
 */

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function footer(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[data-slot="dialog-footer"]');
}

function mount() {
  act(() => {
    root.render(
      createElement(
        Dialog,
        { open: true },
        createElement(
          DialogContent,
          { showCloseButton: false },
          createElement(DialogFooter, null, "Create desk"),
        ),
      ),
    );
  });
}

describe("dialog footer", () => {
  it("is sticky, so content scrolls behind it", () => {
    mount();
    expect(footer()?.className).toContain("sticky");
  });

  it("paints an opaque background", () => {
    mount();
    const classes = footer()?.className.split(/\s+/) ?? [];
    expect(classes).toContain("bg-muted");
  });

  it("carries no alpha modifier on its background", () => {
    mount();
    // `bg-muted/50` and friends: a slash on the background utility is what let
    // the scrolled roster read through the buttons.
    const translucent = (footer()?.className.split(/\s+/) ?? []).filter((c) =>
      /^bg-.+\/\d+$/.test(c),
    );
    expect(translucent).toEqual([]);
  });
});
