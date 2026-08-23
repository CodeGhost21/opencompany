// @vitest-environment jsdom

import { act, createElement, type ReactElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useReturnFocus } from "@/components/ui/return-focus";

/**
 * Regression coverage for #1387's return-focus capture.
 *
 * `useReturnFocus` remembers what to refocus when a trigger-less dialog or
 * sheet closes. Two properties are load-bearing and neither is visible from the
 * rendered output, so they are pinned here rather than in the browser suite:
 *
 * 1. The capture happens on the **closed → open transition**. A popup that
 *    stays mounted while closed (the sidebar's `Sheet`) would otherwise pin
 *    whatever was focused during an unrelated ancestor rerender.
 * 2. For a controlled root the `open` prop is authoritative. Base UI reports a
 *    dismissal through `onOpenChange` before the consumer has agreed to it, and
 *    consumers do refuse — `SetupDialog` passes a no-op handler and
 *    `DeskCreateDialog` swallows it mid-submit. Believing the callback would
 *    make the next render look like a fresh opening and capture an element
 *    inside the still-open popup, so the eventual real close would restore
 *    focus to something that no longer exists.
 */

type Harness = {
  target: React.RefObject<HTMLElement | null>;
  handleOpenChange: ReturnType<typeof useReturnFocus>["handleOpenChange"];
};

let container: HTMLDivElement;
let root: Root;
let harness: Harness;

function Probe({
  open,
  onOpenChange,
}: {
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  harness = useReturnFocus(open, onOpenChange as never) as Harness;
  return null;
}

function render(element: ReactElement) {
  act(() => {
    root.render(element);
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
  document.body.innerHTML = "";
});

function button(id: string) {
  const el = document.createElement("button");
  el.id = id;
  document.body.append(el);
  return el;
}

describe("useReturnFocus", () => {
  it("captures the control that was focused when a controlled popup opened", () => {
    const opener = button("opener");
    render(createElement(Probe, { open: false }));

    opener.focus();
    render(createElement(Probe, { open: true }));

    expect(harness.target.current).toBe(opener);
  });

  it("ignores what was focused during a rerender while the popup is closed", () => {
    const search = button("search");
    const opener = button("opener");
    render(createElement(Probe, { open: false }));

    // The always-mounted case: typing in an unrelated field rerenders the
    // popup's owner long before anyone opens it.
    search.focus();
    render(createElement(Probe, { open: false }));
    expect(harness.target.current).toBeNull();

    opener.focus();
    render(createElement(Probe, { open: true }));
    expect(harness.target.current).toBe(opener);
  });

  it("treats a mount that is already open as that popup's opening", () => {
    // `CreateTaskDialog` returns `null` while closed, so its root never renders
    // in the closed state at all.
    const opener = button("opener");
    opener.focus();
    render(createElement(Probe, { open: true }));

    expect(harness.target.current).toBe(opener);
  });

  it("keeps the opener when a controlled consumer refuses the dismissal", () => {
    const opener = button("opener");
    render(createElement(Probe, { open: false }));

    opener.focus();
    render(createElement(Probe, { open: true, onOpenChange: () => {} }));
    expect(harness.target.current).toBe(opener);

    // Escape asks to close; the consumer declines, so `open` stays true.
    const insideThePopup = button("inside");
    act(() => {
      harness.handleOpenChange(false, undefined as never);
    });
    insideThePopup.focus();
    render(createElement(Probe, { open: true, onOpenChange: () => {} }));

    expect(harness.target.current).toBe(opener);
  });

  it("drops a stale target when the next opening has nothing focused", () => {
    const opener = button("opener");
    render(createElement(Probe, { open: false }));

    opener.focus();
    render(createElement(Probe, { open: true }));
    expect(harness.target.current).toBe(opener);

    // The dialog closes and its opener leaves the document — a row that was
    // deleted, a toolbar that rerendered — so focus falls back to `<body>` and
    // the next opening comes from code rather than from a control.
    render(createElement(Probe, { open: false }));
    opener.remove();
    expect(document.activeElement).toBe(document.body);
    render(createElement(Probe, { open: true }));

    // Null, not the detached opener: Base UI's own fallback is a better answer
    // than focusing something that is no longer on the page.
    expect(harness.target.current).toBeNull();
  });

  it("recaptures for an uncontrolled root, which only reports through the callback", () => {
    const opener = button("opener");
    render(createElement(Probe, {}));

    opener.focus();
    act(() => {
      harness.handleOpenChange(true, undefined as never);
    });
    expect(harness.target.current).toBe(opener);

    // Closing and reopening from a different control moves the target.
    act(() => {
      harness.handleOpenChange(false, undefined as never);
    });
    const other = button("other");
    other.focus();
    act(() => {
      harness.handleOpenChange(true, undefined as never);
    });
    expect(harness.target.current).toBe(other);
  });

  it("forwards every change to the consumer's own handler", () => {
    const seen: boolean[] = [];
    render(
      createElement(Probe, {
        open: false,
        onOpenChange: (next: boolean) => seen.push(next),
      })
    );

    act(() => {
      harness.handleOpenChange(true, undefined as never);
      harness.handleOpenChange(false, undefined as never);
    });

    expect(seen).toEqual([true, false]);
  });
});
