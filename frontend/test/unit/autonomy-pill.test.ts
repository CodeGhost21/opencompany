// @vitest-environment jsdom

import { act, createElement, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Eye, Infinity as InfinityIcon, ShieldCheck, UserCheck, Zap } from "lucide-react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { PolicyStatus } from "@/api/policy";
import {
  AutonomyPill,
  leadSentence,
  tierDescription,
  tierIcon,
  tierLabel,
} from "@/components/autonomy-pill";
import { useAutonomy } from "@/hooks/use-autonomy";
import { ConsoleProvider } from "@/lib/console-context";

/**
 * The autonomy pill in the window's title row.
 *
 * What is actually load-bearing here is not that it draws a pill — it is that
 * every word in it came from the host, that it says *nothing* when it does not
 * know, and that it never claims anything about spending. The last one is not
 * fussiness: this build runs its approval gate with policy HITL disabled
 * (`src/runtime/builder.rs:2476`), so `autoApproveUnderUsd` governs nothing and
 * a pill that rendered it would be stating a number that does not apply.
 *
 * Since the pill became a tier *switcher*, one more property joins them and
 * outranks all of them: **the tier on screen is only ever a value the host
 * returned.** A write that fails must leave the previous tier standing. A pill
 * that says `readonly` while the company is on `full` is worse than no pill.
 */

const toasts = vi.hoisted(() => ({
  base: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
}));

vi.mock("sonner", () => {
  const toast = Object.assign(toasts.base, {
    success: toasts.success,
    error: toasts.error,
    warning: toasts.warning,
    info: toasts.info,
  });
  return { toast };
});

/** The four tiers the runtime accepts, with the host's own text. */
const TIERS = [
  {
    value: "readonly",
    label: "Read-only",
    description: "The agents can look at things but change nothing and spend nothing.",
  },
  {
    value: "supervised",
    label: "Supervised",
    description:
      "Conservative execution restrictions. Approval prompts are explicit through request_approval while policy HITL is disabled.",
  },
  {
    value: "auto",
    label: "Auto",
    description:
      "Balanced execution autonomy. Approval prompts are explicit through request_approval while policy HITL is disabled.",
  },
  {
    value: "full",
    label: "Full",
    description:
      "Broadest execution autonomy. Approval prompts are explicit through request_approval while policy HITL is disabled.",
  },
];

function policy(overrides: Partial<PolicyStatus> = {}): PolicyStatus {
  return {
    mode: "auto",
    alwaysApprove: [],
    autoApproveUnderUsd: 5,
    approvalTtlHours: 24,
    manifestMode: "auto",
    manifestAlwaysApprove: [],
    manifestAutoApproveUnderUsd: 5,
    manifestApprovalTtlHours: null,
    overridden: false,
    tiers: TIERS,
    takesEffect: "on the next turn",
    ...overrides,
  };
}

let host: HTMLDivElement;
let root: Root | null = null;

function render(node: Parameters<Root["render"]>[0]) {
  act(() => root!.render(node));
}

function pill(): HTMLElement | null {
  return host.querySelector("[data-testid=autonomy-pill]");
}

beforeEach(() => {
  (
    globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  toasts.success.mockClear();
  toasts.error.mockClear();
});

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  host.remove();
});

describe("reading the tier from the host", () => {
  it("names each of the four tiers with the host's own label", () => {
    // All four, not a spot check: the whole point of the pill is that
    // `readonly` and `full` are not the same state, and a test that only
    // exercised `auto` would pass against a component that hard-coded it.
    for (const tier of TIERS) {
      expect(tierLabel(policy({ mode: tier.value }))).toBe(tier.label);
    }
  });

  it("falls back to the mode word for a tier this console has no text for", () => {
    // A console built against a newer host must name the tier in force rather
    // than silently render an empty pill.
    const status = policy({ mode: "guarded", tiers: TIERS });
    expect(tierLabel(status)).toBe("guarded");
    expect(tierDescription(status)).toBeNull();
  });
});

describe("the lead sentence", () => {
  it("cuts the host's description at its first sentence, keeping the full stop", () => {
    expect(leadSentence(TIERS[2].description)).toBe("Balanced execution autonomy.");
  });

  it("uses a single-sentence description whole", () => {
    expect(leadSentence(TIERS[0].description)).toBe(
      "The agents can look at things but change nothing and spend nothing.",
    );
  });

  it("uses a description with no sentence break whole", () => {
    expect(leadSentence("No full stop here")).toBe("No full stop here");
  });
});

describe("the tier icons", () => {
  it("gives each mode word its own glyph, keyed by the word and not by position", () => {
    // Four distinct icons, one per tier. Position would re-point every icon at
    // a different meaning the moment a host filtered a tier out of the list.
    expect(tierIcon("readonly")).toBe(Eye);
    expect(tierIcon("supervised")).toBe(UserCheck);
    expect(tierIcon("auto")).toBe(Zap);
    expect(tierIcon("full")).toBe(InfinityIcon);
    const glyphs = new Set(TIERS.map((t) => tierIcon(t.value)));
    expect(glyphs.size).toBe(TIERS.length);
  });

  it("falls back to the neutral shield for a mode word it has never heard of", () => {
    // The tier list is the host's, so a newer runtime WILL present a mode with
    // no entry here. It has to draw that row, not a hole.
    expect(tierIcon("guarded")).toBe(ShieldCheck);
    expect(tierIcon("")).toBe(ShieldCheck);
  });

  it("renders an unknown tier's pill without crashing, glyph and all", () => {
    render(createElement(AutonomyPill, { status: policy({ mode: "guarded" }) }));
    expect(pill()!.textContent).toContain("guarded");
    expect(pill()!.querySelector("svg")).not.toBeNull();
  });
});

describe("the pill", () => {
  it("renders nothing at all when the tier is unknown", () => {
    // The rule this exists to hold: unknown is never folded into a confident
    // answer. An empty space is correct; a default tier would be a sentence
    // about what the agents may do, written when we do not know.
    render(createElement(AutonomyPill, { status: null }));
    expect(pill()).toBeNull();
    expect(host.textContent).toBe("");
  });

  it("shows the tier's name and the host's lead sentence", () => {
    render(createElement(AutonomyPill, { status: policy({ mode: "auto" }) }));
    expect(pill()!.textContent).toContain("Auto");
    expect(pill()!.textContent).toContain("Balanced execution autonomy.");
  });

  it("carries the host's FULL description in its tooltip, not the cut one", () => {
    render(createElement(AutonomyPill, { status: policy({ mode: "auto" }) }));
    // The row shows a sentence; the hover has to give back everything the host
    // said, including the half that says policy HITL is disabled.
    expect(pill()!.getAttribute("title")).toBe(TIERS[2].description);
    expect(pill()!.getAttribute("title")).toContain("policy HITL is disabled");
  });

  it("never states a spend threshold, whatever the policy says", () => {
    // `autoApproveUnderUsd` is stored but inactive in this build — the gate
    // returns Allow before it is ever consulted
    // (`src/policy/gate.rs:749`, `src/harness/built_in/policy.rs:1701`), and
    // the settings page labels the field "(inactive)". Rendering it here would
    // put a governing-looking number on permanent display that governs nothing.
    render(createElement(AutonomyPill, { status: policy({ autoApproveUnderUsd: 5 }) }));
    expect(pill()!.textContent).not.toContain("$");
    expect(pill()!.textContent).not.toContain("5");
    expect(pill()!.getAttribute("title")).not.toContain("$");
  });

  it("is a control, and gives its pointer events back to do it", () => {
    // Reversed from the original design on the operator's instruction. The
    // drag band survives because `data-tauri-drag-region` is opt-in per
    // element: `WindowTitleBar` carries a dedicated `flex-1 self-stretch`
    // spacer that opts in on its own, and that spacer — not the pill — is the
    // elastic part of the row.
    render(createElement(AutonomyPill, { status: policy() }));
    expect(pill()!.tagName).toBe("BUTTON");
    expect(pill()!.hasAttribute("data-tauri-drag-region")).toBe(false);
    // Nothing inside it may swallow the press before the trigger sees it.
    // `getAttribute` rather than `.className`: an SVG's `className` is an
    // `SVGAnimatedString` and never a string, so the obvious assertion
    // silently passes over the one child most likely to be under the pointer.
    for (const child of Array.from(pill()!.children)) {
      expect(child.getAttribute("class") ?? "").not.toContain("pointer-events-none");
    }
  });

  it("sits in the 52px row with real vertical padding", () => {
    // `py-0.5` read as cramped against `WINDOW_TITLE_BAR_HEIGHT` (52). The
    // class is asserted rather than a measured height because jsdom computes
    // no Tailwind.
    render(createElement(AutonomyPill, { status: policy() }));
    expect(pill()!.className).toContain("py-1.5");
    expect(pill()!.className).not.toContain("py-0.5");
  });

  it("drops the sentence below the lg breakpoint and keeps the tier's name", () => {
    // The degradation the 880px minimum window forces, made explicit: the
    // sentence is hidden, the tier is not. A pill that had silently dropped
    // the tier would look identical to a company with no policy at all.
    render(createElement(AutonomyPill, { status: policy({ mode: "auto" }) }));
    const sentence = pill()!.querySelector(
      "[data-testid=autonomy-consequence]",
    ) as HTMLElement;
    expect(sentence.className).toContain("hidden");
    expect(sentence.className).toContain("lg:inline");
    // The label carries no responsive visibility class of its own.
    const label = Array.from(pill()!.children).find(
      (c) => c.textContent === "Auto",
    ) as HTMLElement;
    expect(label).toBeDefined();
    expect(label.className).not.toContain("hidden");
  });

  it("renders no sentence element when the host ships no description", () => {
    render(createElement(AutonomyPill, { status: policy({ mode: "guarded" }) }));
    expect(pill()!.textContent).toContain("guarded");
    expect(pill()!.querySelector("[data-testid=autonomy-consequence]")).toBeNull();
    expect(pill()!.getAttribute("title")).toBeNull();
  });

  it("cannot be pressed with no authenticated client to write through", () => {
    // Rendered outside a `ConsoleProvider` — a styleguide page, or a console
    // whose session has gone. It still STATES the tier; it just cannot change
    // it. Disabled rather than hidden, so the capability stays discoverable.
    render(createElement(AutonomyPill, { status: policy() }));
    expect((pill() as HTMLButtonElement).disabled).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// The switcher: opening it, what it offers, and what a write does to the row.
// ---------------------------------------------------------------------------

/**
 * A fake host. `get` answers `GET {scope}/policy`, `put` the tier write.
 *
 * The whole `useAutonomy` → `AutonomyPill` → `setPolicy` → `applyAutonomy`
 * loop is exercised rather than the pill alone, because the property under
 * test — "the row never shows a tier the host did not return" — lives in that
 * loop and in no single piece of it.
 */
function client(over: {
  get?: () => Promise<PolicyStatus>;
  put?: (path: string, body: unknown) => Promise<PolicyStatus>;
}): OpenCompanyClient {
  return {
    scopeFor: () => "/api/v1/company/acme",
    get: vi.fn(over.get ?? (() => Promise.resolve(policy()))),
    put: vi.fn(over.put ?? ((_p: string, _b: unknown) => Promise.resolve(policy()))),
  } as unknown as OpenCompanyClient;
}

/** The shell's wiring, in miniature: the poll feeds the pill inside the scope. */
function Harness({ api }: { api: OpenCompanyClient }): ReactNode {
  const status = useAutonomy(api, "acme");
  return createElement(ConsoleProvider, {
    client: api,
    company: "acme",
    children: createElement(AutonomyPill, { status }),
  });
}

async function mount(api: OpenCompanyClient) {
  await act(async () => {
    root!.render(createElement(Harness, { api }));
  });
}

/** The menu renders through a portal onto `document.body`, not into `host`. */
function row(mode: string): HTMLElement | null {
  return document.querySelector(`[data-testid=autonomy-tier-${mode}]`);
}

async function openMenu() {
  await act(async () => {
    (pill() as HTMLButtonElement).click();
  });
}

describe("changing the tier from the title bar", () => {
  it("offers every tier the host returned, in the host's own words", async () => {
    const api = client({});
    await mount(api);
    await openMenu();
    for (const tier of TIERS) {
      const item = row(tier.value);
      expect(item, `no row for ${tier.value}`).not.toBeNull();
      expect(item!.textContent).toContain(tier.label);
      // The FULL description, not `leadSentence`: an open menu has the room,
      // and this is the moment the words actually matter.
      expect(item!.textContent).toContain(tier.description);
    }
  });

  it("offers a tier this console has no icon or text for", async () => {
    // The list is the host's. A console that showed only the modes it
    // recognizes would silently withhold a tier the host would accept.
    const api = client({
      get: () =>
        Promise.resolve(
          policy({
            tiers: [
              ...TIERS,
              { value: "guarded", label: "Guarded", description: "Something newer." },
            ],
          }),
        ),
    });
    await mount(api);
    await openMenu();
    expect(row("guarded")).not.toBeNull();
    expect(row("guarded")!.textContent).toContain("Guarded");
  });

  it("marks the tier actually in force", async () => {
    const api = client({ get: () => Promise.resolve(policy({ mode: "supervised" })) });
    await mount(api);
    await openMenu();
    expect(row("supervised")!.getAttribute("aria-current")).toBe("true");
    expect(row("full")!.getAttribute("aria-current")).toBe("false");
  });

  it("carries the host's timing sentence into the menu, before the choice", async () => {
    // An operator pulling the tier down mid-incident needs to know a running
    // turn finishes under the OLD tier before they pick, not after.
    const api = client({
      get: () => Promise.resolve(policy({ takesEffect: "on the next turn" })),
    });
    await mount(api);
    await openMenu();
    const note = document.querySelector("[data-testid=autonomy-takes-effect]");
    expect(note).not.toBeNull();
    expect(note!.textContent).toContain("Takes effect on the next turn.");
  });

  it("writes the chosen mode and shows the tier the host answered with", async () => {
    const api = client({
      get: () => Promise.resolve(policy({ mode: "auto" })),
      put: () => Promise.resolve(policy({ mode: "readonly" })),
    });
    await mount(api);
    expect(pill()!.textContent).toContain("Auto");
    await openMenu();
    await act(async () => {
      row("readonly")!.click();
    });
    // Only `mode` — an omitted field is left alone by the host, so picking a
    // tier here cannot silently discard the always-ask list.
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", {
      mode: "readonly",
    });
    // And the row says so NOW, not after the 30s poll.
    expect(pill()!.textContent).toContain("Read-only");
    expect(toasts.success).toHaveBeenCalled();
  });

  it("leaves the previous tier standing when the write fails", async () => {
    // THE case. A rejected write must never leave a tier on screen that the
    // company is not actually under.
    const api = client({
      get: () => Promise.resolve(policy({ mode: "auto" })),
      put: () => Promise.reject(new Error("Only an admin can change the policy.")),
    });
    await mount(api);
    await openMenu();
    // `full` is wider than `auto`, so the write is behind the confirmation.
    await act(async () => {
      row("full")!.click();
    });
    await act(async () => {
      confirmButton()!.click();
    });
    expect(pill()!.textContent).toContain("Auto");
    expect(pill()!.textContent).not.toContain("Full");
    // And it says so out loud rather than failing silently.
    expect(toasts.error).toHaveBeenCalledWith("Only an admin can change the policy.");
  });

  it("ignores a poll that was already in flight when the tier was written", async () => {
    // The 30s poll and the write race, and the poll can lose: a read ISSUED
    // before the change lands after it, still carrying the old tier. Without a
    // guard that response wins and the row states the previous tier for the
    // rest of the interval — which is exactly the "up to 30s of a wrong
    // answer" the direct hand-back exists to remove.
    vi.useFakeTimers();
    try {
      let releaseStale: ((next: PolicyStatus) => void) | null = null;
      let reads = 0;
      const api = client({
        get: () => {
          reads += 1;
          if (reads === 1) return Promise.resolve(policy({ mode: "auto" }));
          return new Promise<PolicyStatus>((resolve) => {
            releaseStale = resolve;
          });
        },
        put: () => Promise.resolve(policy({ mode: "readonly" })),
      });
      await mount(api);
      // Tick the poll so a second read is genuinely in flight.
      await act(async () => {
        vi.advanceTimersByTime(30000);
      });
      expect(reads).toBe(2);
      await openMenu();
      await act(async () => {
        row("readonly")!.click();
      });
      expect(pill()!.textContent).toContain("Read-only");
      // Now the stale read answers, with the tier as it was before the write.
      await act(async () => {
        releaseStale!(policy({ mode: "auto" }));
      });
      expect(pill()!.textContent).toContain("Read-only");
      expect(pill()!.textContent).not.toContain("Auto");
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not re-write the tier already in force", async () => {
    // The PUT is attributed and durable: it would record an operator
    // "changing" the policy to the value it already had.
    const api = client({ get: () => Promise.resolve(policy({ mode: "auto" })) });
    await mount(api);
    await openMenu();
    await act(async () => {
      row("auto")!.click();
    });
    expect(api.put).not.toHaveBeenCalled();
    expect(toasts.success).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// The gate. Widening the tier from here is the same act as widening it on the
// settings page, so it meets the same confirmation — and narrowing meets none.
// ---------------------------------------------------------------------------

/** The confirmation renders through a portal onto `document.body`, like the menu. */
function confirmButton(): HTMLButtonElement | null {
  return document.querySelector("[data-testid=autonomy-tier-confirm]");
}

function cancelButton(): HTMLButtonElement | null {
  return document.querySelector("[data-testid=autonomy-tier-cancel]");
}

/** Pick a row from the open menu. */
async function pick(mode: string) {
  await act(async () => {
    row(mode)!.click();
  });
}

describe("widening the tier from the title bar", () => {
  it("writes nothing on the click alone, and keeps stating the tier in force", async () => {
    // The regression this exists to stop: without the gate the title bar is a
    // strictly cheaper route to the broadest autonomy this runtime has than the
    // settings page, which was deliberately given a confirmation for the very
    // same act.
    const api = client({ get: () => Promise.resolve(policy({ mode: "supervised" })) });
    await mount(api);
    await openMenu();
    await pick("full");

    expect(api.put).not.toHaveBeenCalled();
    expect(confirmButton()).not.toBeNull();
    expect(pill()!.textContent).toContain("Supervised");
    expect(pill()!.textContent).not.toContain("Full");
  });

  it("asks in the settings page's own words, not a second set of them", async () => {
    // Every string here is asserted as a literal rather than against the
    // constant it comes from: importing the constant would pass however the two
    // dialogs were worded, and what is under test is that an operator meets the
    // SAME sentence wherever they widen the tier. `policy-tier-autonomy.test.ts`
    // pins the identical literals on the settings page.
    const api = client({ get: () => Promise.resolve(policy({ mode: "supervised" })) });
    await mount(api);
    await openMenu();
    await pick("full");

    const text = document.body.textContent ?? "";
    expect(text).toContain("Give teammates more autonomy?");
    // Both sides of the move, in the host's own prose.
    expect(text).toContain("Instead of: Conservative execution restrictions.");
    expect(text).toContain("With Full: Broadest execution autonomy.");
    expect(text).toContain("They will use the Full setting on their next turn.");
    // Still true in this build, and the reason the tier is not the only gate.
    expect(text).toContain("Approval prompts remain explicit through request_approval.");
    expect(cancelButton()!.textContent).toBe("Keep current setting");
    expect(confirmButton()!.textContent).toBe("Give more autonomy");
  });

  it("writes the wider tier once it is confirmed, and shows what the host answered", async () => {
    const api = client({
      get: () => Promise.resolve(policy({ mode: "supervised" })),
      put: () => Promise.resolve(policy({ mode: "full" })),
    });
    await mount(api);
    await openMenu();
    await pick("full");
    await act(async () => {
      confirmButton()!.click();
    });

    // Only `mode`, exactly as a narrowing sends — the confirmation gates the
    // decision, it does not change the request.
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", {
      mode: "full",
    });
    expect(pill()!.textContent).toContain("Full");
    expect(toasts.success).toHaveBeenCalled();
  });

  it("writes NOTHING and leaves the tier untouched when the confirmation is cancelled", async () => {
    // Cancelling has to be a true no-op. A dialog that dismissed but still wrote
    // would be worse than no dialog: the operator has been told they declined.
    const api = client({ get: () => Promise.resolve(policy({ mode: "supervised" })) });
    await mount(api);
    await openMenu();
    await pick("full");
    await act(async () => {
      cancelButton()!.click();
    });

    expect(api.put).not.toHaveBeenCalled();
    expect(pill()!.textContent).toContain("Supervised");
    expect(pill()!.textContent).not.toContain("Full");
    expect(toasts.success).not.toHaveBeenCalled();
    expect(toasts.error).not.toHaveBeenCalled();
      // Focus restoration is deliberately NOT asserted here, and the omission is
      // the finding rather than a gap.
      //
      // Cancelling used to leave focus on `<body>` — a keyboard operator
      // stranded mid-row — because the dialog named no `finalFocus`. The pill now
      // names its trigger explicitly, the way the settings page names its own
      // target, and that is a real fix worth having.
      //
      // What could not be built is an honest test for it. Base UI restores focus
      // asynchronously, outside the `act` that dispatched the click, so the
      // assertion needs a wait; and under jsdom's scheduling the result is noise
      // in BOTH directions — with a 50-tick budget it failed 6 runs in 12 before
      // the fix and still passed 5 of 6 with the fix reverted, and with a 3-tick
      // budget it passed 10 of 10 either way. A test that reports the same answer
      // whether or not the code is there proves nothing, and one that flakes
      // reddens CI for no signal. The behaviour is pinned by the `finalFocus`
      // prop and its comment in `autonomy-pill.tsx` instead.
  });

  it("narrows in one click, with no confirmation at all", async () => {
    // Deliberate asymmetry, and the same one the settings page draws. Reducing
    // what the agents may do is what an operator reaches for mid-incident;
    // friction there is friction pointed the wrong way.
    const api = client({
      get: () => Promise.resolve(policy({ mode: "full" })),
      put: () => Promise.resolve(policy({ mode: "readonly" })),
    });
    await mount(api);
    await openMenu();
    await pick("readonly");

    expect(confirmButton()).toBeNull();
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", {
      mode: "readonly",
    });
    expect(pill()!.textContent).toContain("Read-only");
  });

  it("confirms every step up the host's order and no step down it", async () => {
    // A spot check on one pair would pass against a component that hard-coded
    // "confirm `full`". The order is the host's, so the rule is about the order
    // and not about any particular tier word.
    for (const [from, to, gated] of [
      ["readonly", "supervised", true],
      ["readonly", "full", true],
      ["supervised", "auto", true],
      ["auto", "full", true],
      ["full", "auto", false],
      ["auto", "readonly", false],
      ["supervised", "readonly", false],
    ] as [string, string, boolean][]) {
      const api = client({
        get: () => Promise.resolve(policy({ mode: from })),
        put: () => Promise.resolve(policy({ mode: to })),
      });
      await mount(api);
      await openMenu();
      await pick(to);
      if (gated) {
        expect(api.put, `${from} -> ${to} should have been gated`).not.toHaveBeenCalled();
        await act(async () => {
          cancelButton()!.click();
        });
      } else {
        expect(api.put, `${from} -> ${to} should have written straight away`).toHaveBeenCalled();
      }
    }
  });

  it("does not call an unorderable current mode a widening", async () => {
    // A console against a newer host can be sitting on a mode absent from the
    // tier list; `widensAutonomy` answers false for it, and this asserts the
    // pill inherits that answer rather than inventing a stricter one the
    // settings page does not make.
    const api = client({
      get: () => Promise.resolve(policy({ mode: "guarded" })),
      put: () => Promise.resolve(policy({ mode: "full" })),
    });
    await mount(api);
    await openMenu();
    await pick("full");

    expect(confirmButton()).toBeNull();
    expect(api.put).toHaveBeenCalledWith("/api/v1/company/acme/policy", {
      mode: "full",
    });
  });

  it("keeps the confirmation up when the confirmed write is rejected", async () => {
    // The dialog closing on a failure would read as "done" for a change that
    // never landed. It stays, so the operator can retry or back out.
    const api = client({
      get: () => Promise.resolve(policy({ mode: "supervised" })),
      put: () => Promise.reject(new Error("Only an admin can change the policy.")),
    });
    await mount(api);
    await openMenu();
    await pick("full");
    await act(async () => {
      confirmButton()!.click();
    });

    expect(confirmButton()).not.toBeNull();
    expect(pill()!.textContent).toContain("Supervised");
    expect(toasts.error).toHaveBeenCalledWith("Only an admin can change the policy.");
  });
});
