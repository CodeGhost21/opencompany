// @vitest-environment jsdom

import { act, createElement, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { CapabilityStatusDto, CognitionState } from "@/api/types";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { ChatView } from "@/views/ChatView";

/**
 * Issues #1734 / #1735 — chat says so, before the first echo, when this company
 * has no model behind it.
 *
 * The bug is a silent degrade that reaches all the way to the transcript: with
 * no inference configured the runtime falls back to `EchoBrain`, whose replies
 * render under the teammate's own name, and no surface anywhere mentions it.
 * The remedy is one settings page away, so the banner has to *name* it — an
 * "unavailable" that does not say what to do sends the operator looking for a
 * new build.
 *
 * The two causes need different copy, which is why the host reports a
 * discriminated state rather than a boolean: only `unconfigured` is fixable in
 * the app. Both are pinned here, along with the two silences that must not
 * produce a banner at all.
 */

let container: HTMLDivElement;
let root: Root;

/**
 * A client that answers the capability read, and answers everything else with
 * an empty list.
 *
 * A `Proxy` rather than an enumerated stub on purpose. `ChatView` boots eight
 * unrelated reads — roster, viewer, people, desks, mentionables, history,
 * read-state, presence — and naming each one here would make this test a
 * standing record of that list, failing on the next read anyone adds for a
 * reason that has nothing to do with the banner. An empty answer is a state the
 * view already handles everywhere (it is what a company with no teammates and
 * no history looks like), so the fixture stays about the one read it is for.
 */
function clientWith(cognition: CognitionState | undefined | "reject"): OpenCompanyClient {
  const capabilityStatus = vi.fn(() =>
    cognition === "reject"
      ? Promise.reject(new Error("no capability surface on this host"))
      : Promise.resolve({ configured: false, cognition } as CapabilityStatusDto),
  );
  const named: Record<string, unknown> = {
    capabilityStatus,
    scopeFor: () => "/api/v1/company",
  };
  return new Proxy(named, {
    get: (target, prop: string) => target[prop] ?? (() => Promise.resolve([])),
  }) as unknown as OpenCompanyClient;
}

// `createElement` rather than JSX because the unit suite's vitest `include` is
// `*.test.ts` — a `.tsx` file is silently not collected, which reads as a
// passing suite.
async function render(cognition: CognitionState | undefined | "reject"): Promise<void> {
  const client = clientWith(cognition);
  const scopeRef = createRef<{
    connection: string;
    company: string | null;
    client: OpenCompanyClient;
  }>() as { current: { connection: string; company: string | null; client: OpenCompanyClient } };
  scopeRef.current = { connection: "c1", company: "acme", client };
  await act(async () => {
    root.render(
      createElement(ConnectionScopeProvider, {
        scope: { connection: "c1", company: "acme" },
        children: createElement(ChatView, {
          client,
          company: "acme",
          sub: "main",
          onNavigate: () => {},
          transcripts: {},
          setTranscripts: () => {},
          scopeRef,
        }),
      }),
    );
    // Two microtask drains: the capability read, then the state it sets.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

function banner(): HTMLElement | null {
  return container.querySelector('[data-testid="chat-cognition-banner"]');
}

/**
 * jsdom ships no `matchMedia`, and `useIsDesktop` reaches for it unguarded — so
 * without this the whole view fails to mount and every assertion below would be
 * about a blank container. Same stub as `working-indicator.test.ts`.
 */
function stubMatchMedia() {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: false,
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

beforeEach(() => {
  stubMatchMedia();
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("the chat cognition banner", () => {
  it("says what is wrong and where to fix it when no model is configured", async () => {
    await render("unconfigured");

    const notice = banner();
    expect(notice).not.toBeNull();
    // What is wrong, in the operator's terms rather than the runtime's.
    expect(notice!.textContent).toContain("Teammates can't think yet.");
    // Why the replies below are not what they look like.
    expect(notice!.textContent).toContain("offline echo brain");
    // And the remedy, as a link that actually goes there — the whole point of
    // the issue is that this is one settings page away and nothing said so.
    const link = notice!.querySelector("a");
    expect(link).not.toBeNull();
    expect(link!.getAttribute("href")).toBe("#/settings/inference");
    expect(link!.textContent).toContain("Settings → Inference");
  });

  it("names a rebuild, not a setting, when the harness is not in this build", async () => {
    await render("not-in-build");

    const notice = banner();
    expect(notice).not.toBeNull();
    // The host's own wording for this fact, reused rather than reworded.
    expect(notice!.textContent).toContain(
      "This build cannot reach a model — the agent harness is not compiled in.",
    );
    // No settings link here: offering one would be the switch-that-does-nothing
    // this whole surface exists to stop.
    expect(notice!.querySelector("a")).toBeNull();
  });

  it("stays down when the company has a model", async () => {
    await render("configured");

    expect(banner()).toBeNull();
  });

  it("stays down when the host does not report cognition", async () => {
    // An older host. Silence is not evidence of an echo, and a banner raised on
    // it would be the same unfounded claim in the other direction.
    await render(undefined);

    expect(banner()).toBeNull();
  });

  it("stays down when the capability read fails", async () => {
    await render("reject");

    expect(banner()).toBeNull();
  });
});
