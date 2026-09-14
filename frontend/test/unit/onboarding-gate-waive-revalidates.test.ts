// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { ComposioCredentialSource, ComposioStatus } from "@/api/composio";
import { IntegrationStep } from "@/onboarding/IntegrationStep";

/**
 * Codex review, PR #2046, round 3. The mount read that decides whether to
 * offer a durable waiver depends on `client` and `company`, and neither
 * changes when a credential is added — so a card left mounted while another
 * tab pastes a Composio key kept offering to permanently skip a step that had
 * become ordinarily completable. The waive click now re-reads first.
 */

function status(credentialSource: ComposioCredentialSource): ComposioStatus {
  return {
    inBuild: true,
    granted: true,
    credentialSource,
    backendUrl: "",
    toolkits: [],
    openMode: true,
    effectiveToolkits: [],
    effectiveCatalog: [],
    catalogSource: "backend",
    catalogNotice: null,
  };
}

/** Answers each `/composio` read from `sequence`, repeating its last entry. */
function scriptedClient(sequence: (ComposioCredentialSource | "fail")[]) {
  let call = 0;
  const client = {
    scopeFor: () => "/api/v1/company",
    get: (path: string) => {
      if (!path.includes("/composio")) throw new Error(`unexpected path: ${path}`);
      const answer = sequence[Math.min(call, sequence.length - 1)];
      call += 1;
      if (answer === "fail") return Promise.reject(new Error("network blip"));
      return Promise.resolve(status(answer));
    },
  } as unknown as OpenCompanyClient;
  return { client, reads: () => call };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function mount(client: OpenCompanyClient, onWaive: () => void) {
  await act(async () => {
    root.render(
      createElement(IntegrationStep, {
        client,
        company: null,
        onOpenApps: () => {},
        onOpenCredential: () => {},
        onWaive,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();
  });
}

function waiveButton() {
  return container.querySelector('[data-testid="gate-integration-waive"]') as HTMLButtonElement | null;
}

describe("the durable waiver is revalidated against a fresh credential read", () => {
  it("persists the waiver when the credential is still genuinely absent", async () => {
    const onWaive = vi.fn();
    const { client, reads } = scriptedClient(["none"]);
    await mount(client, onWaive);

    const button = waiveButton();
    expect(button, "a confirmed 'none' still offers the waiver").toBeTruthy();
    await act(async () => {
      button!.click();
      await Promise.resolve();
    });
    await settle();

    expect(onWaive).toHaveBeenCalledTimes(1);
    expect(reads(), "the click asked again rather than trusting the mount read").toBe(2);
  });

  it("does not persist a waiver when a credential arrived after the mount read", async () => {
    const onWaive = vi.fn();
    // Mount sees "none" — another tab then pastes a key, so the re-read differs.
    const { client } = scriptedClient(["none", "static"]);
    await mount(client, onWaive);
    expect(waiveButton()).toBeTruthy();

    await act(async () => {
      waiveButton()!.click();
      await Promise.resolve();
    });
    await settle();

    expect(
      onWaive,
      "the step became ordinarily completable — waiving it durably would be the harm",
    ).not.toHaveBeenCalled();
    expect(
      container.querySelector('[data-testid="gate-integration-has-credential"]'),
      "the card should tell the founder the step is now completable, not drop the click silently",
    ).toBeTruthy();
    expect(waiveButton(), "and withdraw the waiver it can no longer honestly offer").toBeNull();
  });

  it("recovers when the revalidating read hangs instead of answering", async () => {
    // Codex review, PR #2046: `revalidating` disables the waive button and is
    // only cleared by a SETTLED promise, and `OpenCompanyClient` has no
    // timeout anywhere in its request path (`lib/read-timeout.ts`). A request
    // accepted and never answered therefore left the one control a
    // credential-less founder has disabled for the rest of the mount — the
    // same trap this component exists to remove, reached through a stalled
    // read.
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const onWaive = vi.fn();
      let call = 0;
      const client = {
        scopeFor: () => "/api/v1/company",
        get: (path: string) => {
          if (!path.includes("/composio")) throw new Error(`unexpected path: ${path}`);
          call += 1;
          // The mount read answers; the revalidation never does.
          if (call === 1) return Promise.resolve(status("none"));
          return new Promise(() => {});
        },
      } as unknown as OpenCompanyClient;

      await act(async () => {
        root.render(
          createElement(IntegrationStep, {
            client,
            company: null,
            onOpenApps: () => {},
            onOpenCredential: () => {},
            onWaive,
          }),
        );
        await vi.advanceTimersByTimeAsync(0);
      });

      await act(async () => {
        waiveButton()!.click();
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(waiveButton()!.disabled, "the button is disabled while the re-read is out").toBe(true);

      // Past the read timeout the hang becomes an ordinary failure.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(20_000);
      });

      expect(onWaive, "a hung read is not a confirmed 'no credential'").not.toHaveBeenCalled();
      expect(container.querySelector('[data-testid="gate-integration-waive-failed"]')).toBeTruthy();
      expect(waiveButton()!.disabled, "and the founder can try again").toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("persists nothing and says so when the revalidating read fails", async () => {
    const onWaive = vi.fn();
    const { client } = scriptedClient(["none", "fail"]);
    await mount(client, onWaive);

    await act(async () => {
      waiveButton()!.click();
      await Promise.resolve();
    });
    await settle();

    expect(onWaive, "an unconfirmed answer is not grounds for a durable waiver").not.toHaveBeenCalled();
    expect(container.querySelector('[data-testid="gate-integration-waive-failed"]')).toBeTruthy();
    expect(waiveButton(), "the founder can retry rather than being stranded").toBeTruthy();
    expect(waiveButton()!.disabled).toBe(false);
  });
});
