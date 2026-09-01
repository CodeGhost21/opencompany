// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { WINDOW_TITLE_BAR_HEIGHT, WINDOW_CONTROLS_WIDTH } from "@/components/window-chrome";
import { TITLE_BAR_LADDER, WindowTitleBar } from "@/components/window-title-bar";

/**
 * Where macOS actually draws the traffic lights, read from the Tauri config
 * rather than restated here.
 *
 * The row's height and its leading inset are both *derived* from these two
 * numbers, so a test that asserted them against the constants it is checking
 * would agree with any value — including a wrong one. Reading the config is
 * what makes the assertion mean something: it fails when the two files stop
 * agreeing, which is the only way this can actually break.
 */
// Vitest runs with the console package as its cwd, so the repo root is one up.
const TAURI = JSON.parse(
  readFileSync(resolve(process.cwd(), "../src-tauri/tauri.conf.json"), "utf8"),
) as { app: { windows: { trafficLightPosition: { x: number; y: number } }[] } };
const LIGHTS = TAURI.app.windows[0].trafficLightPosition;
/** macOS draws three of them, 12px across, on a 20px pitch. */
const LIGHT_SIZE = 12;
const LIGHT_PITCH = 20;

/**
 * The window's title row.
 *
 * Four properties, each of which is load-bearing and none of which is visible
 * from reading the JSX:
 *
 * 1. **It exists in the browser too.** Only the traffic-light inset is gated on
 *    the desktop runtime. A row that rendered only under Tauri would be a
 *    layout the web build never gets, and the two would drift.
 * 2. **The lights are cleared, and only where they float.** 72px of reserved
 *    space on the macOS desktop; nothing at all anywhere else, where reserving
 *    it would be a hole in the corner of every browser tab.
 * 3. **Empty space drags.** `data-tauri-drag-region` is opt-in per element, so
 *    the spacer between the two controls has to carry it or the only draggable
 *    part of the row is the sliver the flex gaps leave.
 * 4. **Switcher before profile in the DOM**, so tab order reads left-to-right
 *    across the row rather than jumping to the far side and back.
 */

let host: HTMLDivElement;
let root: Root | null = null;

function asDesktop(platform: string) {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  Object.defineProperty(navigator, "platform", { configurable: true, value: platform });
}

function render(node: Parameters<Root["render"]>[0]) {
  act(() => root!.render(node));
}

/** The row, with identifiable stand-ins for the controls it carries. */
function bar(
  autonomy?: ReactNode,
  { profile = true, jumps = true }: { profile?: boolean; jumps?: boolean } = {},
) {
  return createElement(WindowTitleBar, {
    switcher: createElement("button", { type: "button", "data-testid": "stub-switcher" }, "Co"),
    overview: jumps
      ? createElement("button", { type: "button", "data-testid": "stub-overview" }, "Home")
      : undefined,
    approvals: jumps
      ? createElement("button", { type: "button", "data-testid": "stub-approvals" }, "2")
      : undefined,
    autonomy,
    profile: profile
      ? createElement("button", { type: "button", "data-testid": "stub-profile" }, "Me")
      : null,
  });
}

/** One of the row's three right-hand groups, by its test id. */
function group(name: "go" | "state" | "you") {
  return host.querySelector(`[data-testid=title-bar-group-${name}]`) as HTMLElement;
}

/**
 * A stand-in for the autonomy pill. Inert on purpose, and NOT a claim that the
 * real pill is: it is a control that opens a menu and carries no
 * `data-tauri-drag-region` of its own. This row is a pure layout component that
 * takes the pill as an opaque `ReactNode`, so what is under test here is where
 * the slot sits in the row — never what the control in it does.
 */
function stubAutonomy() {
  return createElement("span", { "data-testid": "stub-autonomy" }, "Auto");
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

describe("the window title row", () => {
  it("renders in a browser, carrying both controls", () => {
    render(bar());

    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement | null;
    expect(row).not.toBeNull();
    expect(row?.querySelector("[data-testid=stub-switcher]")).not.toBeNull();
    expect(row?.querySelector("[data-testid=stub-profile]")).not.toBeNull();
  });

  it("is one centred flex row at the traffic lights' height", () => {
    render(bar());

    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement;
    // One rule centres every item, rather than each item carrying its own
    // offset — which is what stops being true when a font loads late or an
    // item with a different intrinsic height joins the row.
    expect(row.className).toContain("items-center");
    expect(row.className).toContain("flex");

    // The centre line the OS gives us: the lights' top offset plus half a
    // light. Everything in the row is centred on the row's own middle, so the
    // row's height has to be twice that or the lights sit off it — and no
    // amount of per-item nudging fixes a row of the wrong height.
    const lightsCentre = LIGHTS.y + LIGHT_SIZE / 2;
    expect(WINDOW_TITLE_BAR_HEIGHT).toBe(2 * lightsCentre);
    expect(row.style.height).toBe(`${2 * lightsCentre}px`);
    // Which is to say: the row's middle IS the lights' middle.
    expect(WINDOW_TITLE_BAR_HEIGHT / 2).toBe(lightsCentre);
  });

  it("reserves nothing for the lights in a browser", () => {
    render(bar());
    expect(host.querySelector("[data-testid=window-controls-inset]")).toBeNull();
  });

  it("insets around the lights on the macOS desktop", () => {
    asDesktop("MacIntel");
    render(bar());

    const inset = host.querySelector("[data-testid=window-controls-inset]") as HTMLElement | null;
    expect(inset).not.toBeNull();
    // Wide enough to clear the last of the three lights, measured from the
    // config rather than restated: anything narrower puts the switcher under
    // a button you can no longer click.
    const lightsRight = LIGHTS.x + 2 * LIGHT_PITCH + LIGHT_SIZE;
    expect(WINDOW_CONTROLS_WIDTH).toBe(lightsRight);
    expect(inset?.style.width).toBe(`${lightsRight}px`);

    // And it stands BEFORE the switcher, which is the whole point: the lights
    // are at the window's left edge, so anything that does not precede the
    // switcher leaves the switcher underneath them.
    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement;
    const switcher = row.querySelector("[data-testid=stub-switcher]") as HTMLElement;
    expect(inset!.compareDocumentPosition(switcher) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("leaves the empty middle draggable", () => {
    render(bar());

    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement;
    // The row itself, so a press that lands on its own padding drags.
    expect(row.hasAttribute("data-tauri-drag-region")).toBe(true);
    // And the spacer, because the attribute is not inherited: Tauri drags only
    // when the pressed element is itself marked, and a press in the middle of
    // the row lands on the spacer rather than on the row.
    const spacer = row.querySelector(":scope > [data-tauri-drag-region][aria-hidden=true]");
    expect(spacer).not.toBeNull();
    expect((spacer as HTMLElement).className).toContain("flex-1");
  });

  it("puts the autonomy slot after the draggable middle and before the profile", () => {
    render(bar(stubAutonomy()));

    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement;
    const spacer = row.querySelector(
      ":scope > [data-tauri-drag-region][aria-hidden=true]",
    ) as HTMLElement;
    const autonomy = row.querySelector("[data-testid=stub-autonomy]") as HTMLElement;
    const profile = row.querySelector("[data-testid=stub-profile]") as HTMLElement;

    expect(autonomy).not.toBeNull();
    // Right-aligned: it must fall on the far side of the elastic spacer, or it
    // sits beside the switcher at the left end of the row instead.
    expect(
      spacer.compareDocumentPosition(autonomy) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // And still before the avatar, which stays last.
    expect(
      autonomy.compareDocumentPosition(profile) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });


  it("groups the right-hand end into go, state and you, in that order", () => {
    render(bar(stubAutonomy()));

    // Three groups, and the two jumps inside the first of them. The row reads
    // as three objects rather than five loose controls, which is the whole
    // reason the hairlines exist.
    expect(group("go").querySelector("[data-testid=stub-overview]")).not.toBeNull();
    expect(group("go").querySelector("[data-testid=stub-approvals]")).not.toBeNull();
    expect(group("state").querySelector("[data-testid=stub-autonomy]")).not.toBeNull();
    expect(group("you").querySelector("[data-testid=stub-profile]")).not.toBeNull();

    const order = (a: HTMLElement, b: HTMLElement) =>
      Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
    const spacer = host.querySelector(
      "[data-testid=window-title-bar] > [data-tauri-drag-region][aria-hidden=true]",
    ) as HTMLElement;
    // All three fall on the far side of the elastic spacer, or they sit beside
    // the switcher at the left-hand end of the row instead of at the right.
    expect(order(spacer, group("go"))).toBe(true);
    expect(order(group("go"), group("state"))).toBe(true);
    expect(order(group("state"), group("you"))).toBe(true);
  });

  it("draws a hairline before the second and third groups and none before the first", () => {
    render(bar(stubAutonomy()));

    // The first group needs no rule: the elastic spacer already separates it
    // from the switcher, and a hairline against that gap would read as an edge
    // rather than as a join.
    expect(group("go").className).not.toContain("before:w-px");
    for (const name of ["state", "you"] as const) {
      const cls = group(name).className;
      expect(cls).toContain("before:w-px");
      // A token, never a hex: `--chrome-border` is the border for the layer
      // this row stands on. `assert-design-tokens.sh` fails a raw colour here.
      expect(cls).toContain("before:bg-chrome-border");
    }
  });

  it("takes the hairline away with the group it introduces", () => {
    // The autonomy pill renders null when the tier is unknown, and the profile
    // control renders null on a host with no sign-in — but the ELEMENT handed
    // to this row is truthy either way, so the layout cannot ask. `empty:hidden`
    // is what answers it: a pseudo-element does not make its host non-empty, so
    // the rule dies with the group instead of standing beside nothing.
    render(bar(null, { profile: false }));

    for (const name of ["state", "you"] as const) {
      expect(group(name).children.length).toBe(0);
      expect(group(name).className).toContain("empty:hidden");
    }
  });

  it("hides the Overview slot below md and never the approvals one", () => {
    render(bar(stubAutonomy()));

    // The ladder's third rung, and the one asymmetry in the row: Overview is a
    // destination you choose, a pending count is one that chooses you.
    const slot = host.querySelector("[data-testid=title-bar-overview-slot]") as HTMLElement;
    expect(slot.className).toContain("hidden");
    expect(slot.className).toContain("md:inline-flex");
    // Approvals sits in the group directly, with no responsive wrapper at all —
    // there is no width at which it goes.
    const approvals = group("go").querySelector("[data-testid=stub-approvals]") as HTMLElement;
    expect(approvals.parentElement).toBe(group("go"));
  });

  it("decides the whole ladder in one place", () => {
    // The point of `TITLE_BAR_LADDER` is that reading it is reading the ladder.
    // Three rungs, in order, at 1280 / 1024 / 768 — and each consumed by the
    // component that owns the item rather than restated there.
    expect(TITLE_BAR_LADDER.autonomySentence).toContain("xl:");
    expect(TITLE_BAR_LADDER.companyName).toContain("lg:");
    expect(TITLE_BAR_LADDER.overview).toContain("md:");
    for (const rung of Object.values(TITLE_BAR_LADDER)) {
      // Every rung hides by default and reveals at its breakpoint, so the
      // narrow window is the one that needs no class to be correct.
      expect(rung.startsWith("hidden ")).toBe(true);
    }

    const read = (rel: string) =>
      readFileSync(resolve(process.cwd(), "src/components", rel), "utf8");
    // The two rungs consumed elsewhere are IMPORTED, not re-typed. A literal
    // breakpoint in either file is the scattering this constant exists to stop.
    for (const file of ["autonomy-pill.tsx", "host-switcher.tsx"]) {
      const source = read(file);
      expect(source).toContain("TITLE_BAR_LADDER");
      expect(source).not.toMatch(/className="hidden [a-z]{2}:/);
    }
  });

  it("puts the switcher before the profile control in the DOM", () => {
    render(bar());

    const row = host.querySelector("[data-testid=window-title-bar]") as HTMLElement;
    const switcher = row.querySelector("[data-testid=stub-switcher]") as HTMLElement;
    const profile = row.querySelector("[data-testid=stub-profile]") as HTMLElement;
    expect(
      switcher.compareDocumentPosition(profile) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });
});
