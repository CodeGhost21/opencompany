// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { TurnStep } from "@/api/types";
import { GENERIC_LABEL, WorkingIndicator, runningStepLabel } from "@/views/chat/WorkingIndicator";

/**
 * The working indicator (issue #787), following OpenHuman's handling: the line
 * is derived from the run's real state rather than being decorative.
 *
 * Two of these are promises that regress silently. The indicator must never
 * name a *settled* step — a stale name reads as current, which is worse than
 * saying little — and the assistive text must not follow the visible label,
 * because a live region re-announcing every step transition is noise.
 */

let container: HTMLDivElement;
let root: Root;
let reduceMotion = false;

function stubMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: query.includes("prefers-reduced-motion") ? reduceMotion : false,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
      onchange: null,
    }),
  });
}

function step(over: Partial<TurnStep> & Pick<TurnStep, "status" | "label">): TurnStep {
  return { kind: "tool_call", ...over } as TurnStep;
}

function render(props: { srLabel?: string; steps?: TurnStep[] } = {}) {
  act(() => {
    root.render(
      createElement(WorkingIndicator, { srLabel: props.srLabel ?? "Replying…", steps: props.steps }),
    );
  });
}

/** The visible line, excluding the screen-reader-only text. */
function visibleLabel(): string {
  return container.querySelector("[aria-hidden].truncate")?.textContent ?? "";
}

function srText(): string {
  return container.querySelector(".sr-only")?.textContent ?? "";
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  reduceMotion = false;
  stubMatchMedia();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("what the line says", () => {
  it("names the step in flight, in the host's own words", () => {
    render({ steps: [step({ status: "ok", label: "Read the brief" }), step({ status: "running", label: "Draft the reply" })] });
    expect(visibleLabel()).toBe("Draft the reply");
  });

  it("names the newest running step when several are open", () => {
    render({
      steps: [step({ status: "running", label: "First" }), step({ status: "running", label: "Second" })],
    });
    expect(visibleLabel()).toBe("Second");
  });

  it("falls back to the generic line when nothing is running", () => {
    // Every step settled while the reply is still being written. Naming the
    // last one would present finished work as current.
    render({ steps: [step({ status: "ok", label: "Read the brief" })] });
    expect(visibleLabel()).toBe(GENERIC_LABEL);
  });

  it("falls back when the surface has no steps at all", () => {
    render({});
    expect(visibleLabel()).toBe(GENERIC_LABEL);
  });

  it("falls back rather than render a blank line for an empty label", () => {
    render({ steps: [step({ status: "running", label: "   " })] });
    expect(visibleLabel()).toBe(GENERIC_LABEL);
  });

  it("does not treat a parked step as running", () => {
    // `awaiting_approval` is waiting on a person, not working (#411).
    render({ steps: [step({ status: "awaiting_approval", label: "Send the email" })] });
    expect(visibleLabel()).toBe(GENERIC_LABEL);
  });
});

describe("the assistive text", () => {
  it("stays stable while the visible label changes", () => {
    render({ steps: [step({ status: "running", label: "Draft the reply" })] });
    expect(srText()).toBe("Replying…");

    render({ steps: [step({ status: "running", label: "Something else entirely" })] });
    expect(visibleLabel()).toBe("Something else entirely");
    expect(srText()).toBe("Replying…");
  });

  it("hides the visible label from assistive tech", () => {
    render({ steps: [step({ status: "running", label: "Draft the reply" })] });
    expect(container.querySelector(".truncate")?.getAttribute("aria-hidden")).toBe("true");
  });
});

describe("prefers-reduced-motion", () => {
  it("stops the pulse but still shows the state", () => {
    reduceMotion = true;
    render({ steps: [step({ status: "running", label: "Draft the reply" })] });
    expect(container.querySelectorAll(".animate-pulse").length).toBe(0);
    expect(visibleLabel()).toBe("Draft the reply");
  });

  it("pulses when motion is allowed", () => {
    render({ steps: [step({ status: "running", label: "Draft the reply" })] });
    expect(container.querySelectorAll(".animate-pulse").length).toBe(1);
  });
});

describe("runningStepLabel", () => {
  it("is undefined for every shape that has nothing in flight", () => {
    expect(runningStepLabel(undefined)).toBeUndefined();
    expect(runningStepLabel([])).toBeUndefined();
    expect(runningStepLabel([step({ status: "error", label: "Broke" })])).toBeUndefined();
  });
});
