// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { InferenceStatus } from "@/api/inference";
import type { TeamMemberDto } from "@/api/types";
import { SetupDialog } from "@/setup/SetupDialog";

/**
 * What first-run setup promises about the model, and what it does with the
 * answer (docs/spec/runtime/company-setup.md).
 *
 * Every case here is a way the disclosure can be *wrong*, which is worse than
 * absent: the whole point of saying something before the questions is that the
 * operator can trust it.
 *
 * - It must ask the **addressed company**, not the host. Readiness read off the
 *   host's managed credential warns a BYOK company about a model it is about to
 *   use, and refuses multi-company hosts outright.
 * - It must not be able to **hold the dialog shut**. Nothing dismisses this
 *   dialog — no Esc, no backdrop, no close button — so an unanswered readiness
 *   check is a locked console.
 * - "Set up a model" must not record a **skip**. It is the operator starting
 *   this flow, and suppressing the offer on their return strands them.
 * - The credential CTA must be gated on the host's **reason**. A model that
 *   answered and could not be designed from already had a working key.
 */

const HARNESS: InferenceStatus = {
  provider: "openrouter",
  slug: "openrouter",
  baseUrl: "https://example.invalid/v1",
  models: {},
  source: "manifest",
  keyConfigured: true,
  cognition: "harness",
  usageMetering: "perTurn",
} as InferenceStatus;

const ECHO: InferenceStatus = { ...HARNESS, cognition: "echo" } as InferenceStatus;

/** One proposed agent is enough: the build-out's reveal is paced per agent. */
const AGENT = { name: "Ada", role: "Operations", description: "Runs the desk." };

function clientWith(
  over: {
    status?: () => Promise<InferenceStatus>;
    source?: "model" | "fallback";
    reason?: string | null;
    roster?: TeamMemberDto[];
  } = {},
): OpenCompanyClient & { removed: string[] } {
  const removed: string[] = [];
  return {
    scopeFor: (company: string | null) => `/api/v1/companies/${company}`,
    get: async () => (over.status ? await over.status() : ECHO),
    post: async () => ({
      agents: [AGENT],
      template: "ecommerce",
      source: over.source ?? "fallback",
      ...(over.reason === null ? {} : { reason: over.reason ?? "no_model" }),
    }),
    addTeamMember: async () => ({}),
    listTeam: async () => over.roster ?? [],
    removeTeamMember: async (agentId: string) => {
      removed.push(agentId);
    },
    removed,
  } as unknown as OpenCompanyClient & { removed: string[] };
}

let container: HTMLDivElement;
let root: Root;
let skipped: number;
let left: number;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  skipped = 0;
  left = 0;
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function show(
  client: OpenCompanyClient,
  over: { redesign?: boolean; fallbackIds?: string[]; onRedesign?: () => void } = {},
) {
  await act(async () => {
    root.render(
      createElement(SetupDialog, {
        open: true,
        client,
        company: "acme",
        redesign: over.redesign,
        fallbackIds: over.fallbackIds,
        onSkip: () => {
          skipped += 1;
        },
        onLeave: () => {
          left += 1;
        },
        onDone: () => {},
        onRedesign: over.onRedesign ?? (() => {}),
      }),
    );
  });
}

/** Base UI portals the dialog, so query the document rather than the container. */
const find = (testId: string) => document.querySelector(`[data-testid="${testId}"]`);

const linkNamed = (text: string) =>
  Array.from(document.querySelectorAll("a")).find((a) => a.textContent?.trim() === text);

async function click(el: Element) {
  await act(async () => {
    (el as HTMLElement).click();
  });
}

async function answer(testId: string, value: string) {
  const field = find(testId) as HTMLInputElement | HTMLTextAreaElement | null;
  expect(field, `no field ${testId}`).toBeTruthy();
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      field instanceof HTMLTextAreaElement
        ? HTMLTextAreaElement.prototype
        : HTMLInputElement.prototype,
      "value",
    )!.set!;
    setter.call(field, value);
    field!.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await click(find("setup-next")!);
}

/** Answer all three questions and let the build-out finish. */
async function runFlow() {
  await answer("setup-field-industry", "E-commerce — homeware");
  await answer("setup-field-teamHint", "");
  await answer("setup-field-automate", "");
  // The build-out reveals one agent per REVEAL_MS and lingers before finishing.
  for (let i = 0; i < 40 && !find("setup-finish"); i++) {
    await act(async () => {
      await new Promise((r) => setTimeout(r, 60));
    });
  }
  expect(find("setup-finish"), "build-out never finished").toBeTruthy();
}

describe("readiness is the addressed company's, not the host's", () => {
  it("says nothing when this company's own cognition can design a roster", async () => {
    await show(clientWith({ status: async () => HARNESS }));
    expect(find("setup-question")).toBeTruthy();
    // The BYOK case: this company has a model even where the host holds no
    // managed credential, so warning about a missing one would be a lie.
    expect(find("setup-inference-notice")).toBeNull();
  });

  it("warns when this company booted onto a path with no design pass", async () => {
    await show(clientWith({ status: async () => ECHO }));
    expect(find("setup-inference-notice")?.textContent).toContain("can't reach a model");
  });

  it("does not promise a tailored team when readiness could not be read", async () => {
    await show(clientWith({ status: async () => Promise.reject(new Error("nope")) }));
    expect(find("setup-inference-notice")?.textContent).toContain("couldn't check");
  });
});

describe("a stalled readiness check cannot lock the operator in", () => {
  it("gives up on the check and asks the questions anyway", async () => {
    vi.useFakeTimers();
    // A host that never answers: `fetch` has no timeout of its own, and this
    // dialog ignores Esc, the backdrop and the close button.
    await show(clientWith({ status: () => new Promise<InferenceStatus>(() => {}) }));
    expect(find("setup-inference-check"), "should be waiting at first").toBeTruthy();
    expect(find("setup-question")).toBeNull();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(find("setup-inference-check")).toBeNull();
    expect(find("setup-question"), "questions withheld indefinitely").toBeTruthy();
    expect(find("setup-inference-notice")?.textContent).toContain("couldn't check");
  });
});

describe('"Set up a model" is starting setup, not declining it', () => {
  it("closes without recording the skip that would suppress the offer", async () => {
    await show(clientWith());
    const link = linkNamed("Set up a model");
    expect(link, "no model link on the notice").toBeTruthy();
    expect(link).toHaveProperty("hash", "#/settings/connections");

    await click(link!);

    expect(left, "should close for the navigation").toBe(1);
    expect(skipped, 'must not persist "I\'ll do this later"').toBe(0);
  });

  it('still records a skip for the explicit "I\'ll do this later"', async () => {
    await show(clientWith());
    await click(find("setup-skip")!);
    expect(skipped).toBe(1);
    expect(left).toBe(0);
  });
});

describe("the finished build-out points at what would actually help", () => {
  it("offers the credential CTA when no model was reachable", async () => {
    await show(clientWith({ source: "fallback", reason: "no_model" }));
    await runFlow();
    expect(find("setup-add-model")).toBeTruthy();
    expect(find("setup-buildout-title")?.parentElement?.textContent).toContain(
      "couldn't reach a model",
    );
  });

  it("asks for more detail instead when a model answered but was not designable", async () => {
    await show(clientWith({ source: "fallback", reason: "not_designable" }));
    await runFlow();
    // The key already worked. Sending this operator to Settings is an
    // instruction that cannot help them.
    expect(find("setup-add-model"), "sent to fix a credential that worked").toBeNull();
    expect(find("setup-buildout-title")?.parentElement?.textContent).toContain(
      "enough in your answers",
    );
  });

  it("withholds the credential CTA when the host did not say why", async () => {
    await show(clientWith({ source: "fallback", reason: null }));
    await runFlow();
    expect(find("setup-add-model")).toBeNull();
  });

  it("says nothing about fallbacks when the model designed the team", async () => {
    await show(clientWith({ source: "model", reason: null }));
    await runFlow();
    expect(find("setup-add-model")).toBeNull();
    expect(find("setup-buildout-title")?.textContent).toContain("Your starting team is ready");
  });

  it("offers a retry and a connection check when a wired model was unreachable", async () => {
    await show(clientWith({ source: "fallback", reason: "model_unreachable" }));
    await runFlow();
    // A credential is already wired, so the CTA that fixes a *missing* model
    // cannot help — but the failure could be the credential itself (rejected
    // rather than merely busy), so the route that checks it is offered beside
    // the retry that fixes a transient blip.
    expect(find("setup-add-model")).toBeNull();
    expect(find("setup-try-redesign")).toBeTruthy();
    expect(find("setup-check-connection")).toBeTruthy();
    expect(find("setup-buildout-title")?.parentElement?.textContent).toContain(
      "couldn't reach it just now",
    );
  });

  it('"Check connection in Settings" records the redesign debt like the wiring CTA', async () => {
    const redesigned: string[][] = [];
    await show(clientWith({ source: "fallback", reason: "model_unreachable" }), {
      onRedesign: (ids) => redesigned.push(ids),
    });
    await runFlow();
    await click(find("setup-check-connection")!);
    // The fallback team is to be replaced when the operator returns from
    // checking the connection — the same debt the wiring CTA records, because
    // the next build-out must replace the shipped team, not stack on it.
    expect(redesigned).toEqual([[]]);
  });

  it('"Try again" returns to the questions in replacing mode', async () => {
    await show(clientWith({ source: "fallback", reason: "model_unreachable" }));
    await runFlow();
    await click(find("setup-try-redesign")!);
    expect(find("setup-question")).toBeTruthy();
    // The company now carries the standard team, so the next build-out must
    // replace it — the notice makes that consequence visible before answering.
    expect(find("setup-redesign-notice")).toBeTruthy();
  });
});

describe("a replacing build-out clears the team it replaces", () => {
  it("removes operator-staffed teammates before creating the new roster", async () => {
    const client = clientWith({
      source: "fallback",
      reason: "model_unreachable",
      roster: [
        { id: "a1", name: "Old Ada", role: "Operations", global: false } as TeamMemberDto,
        { id: "b2", name: "Baseline", role: "Founder", global: true } as TeamMemberDto,
      ],
    });
    // `fallbackIds` is the fallback team the previous pass created, captured
    // when the operator left for model settings — in this direct render it is
    // named by hand rather than read from the debt.
    await show(client, { redesign: true, fallbackIds: ["a1"] });
    await runFlow();
    // The baseline survives — it is not on the operator's roster and cannot be
    // deleted anyway. Only the fallback team the last pass created goes.
    expect(client.removed).toEqual(["a1"]);
  });

  it("removes nothing on a first-run build-out", async () => {
    const client = clientWith({ source: "fallback", reason: "model_unreachable" });
    await show(client);
    await runFlow();
    expect(client.removed).toEqual([]);
  });
});
