// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "@/api/types";
import type { OpenCompanyClient } from "@/api/client";
import type { CompanyStatus } from "@/api/types";
import { CreateCompanyDialog } from "@/components/create-company-dialog";

/**
 * Codex review on #1828 (PR comment 3863028405): after a reset's archive
 * leg lands but the create leg then fails, `busy` goes back to false and
 * Cancel closes the dialog without telling the parent that the company on
 * screen is gone. `ConnectionConsole`'s `onClose` used to take no argument
 * at all, so there was nothing it *could* check — the picker kept the
 * archived card in its roster, and an active console stayed scoped to a
 * runtime `useCompany` has no way to notice is gone (it only reconciles
 * against a later poll that *answers*, and a dead company's poll never
 * does).
 *
 * `onClose` now carries whether the archive landed, and `ConnectionConsole`
 * refreshes the roster when it did (see the callsite wiring in
 * `ConnectionConsole.tsx`, exercised by reading rather than a full mount —
 * same reasoning as `connection-retarget-default-company.test.ts`). These
 * tests are the testable half: that `CreateCompanyDialog` reports the flag
 * correctly for every way the dialog can close.
 */

type ProvisionBody = { manifest_toml: string; id?: string };

function stubClient(opts: {
  lifecycle?: ReturnType<typeof vi.fn>;
  provisionCompany?: ReturnType<typeof vi.fn<(body: ProvisionBody) => Promise<CompanyStatus>>>;
}) {
  return {
    carriesPlatformBearer: true,
    lifecycle: opts.lifecycle ?? vi.fn(() => Promise.resolve()),
    provisionCompany:
      opts.provisionCompany ??
      vi.fn(() => Promise.resolve({ id: "whatever" } as unknown as CompanyStatus)),
  } as unknown as OpenCompanyClient;
}

let container: HTMLDivElement;
let root: Root;
let onClose: ReturnType<typeof vi.fn>;
let onCreated: ReturnType<typeof vi.fn>;

function submitButton(): HTMLButtonElement {
  const match = Array.from(
    document.querySelectorAll<HTMLButtonElement>('[data-slot="dialog-content"] button'),
  ).find(
    (b) =>
      b.textContent?.trim().startsWith("Archive & start clean") ||
      b.textContent?.trim() === "Create company",
  );
  expect(match, 'no submit button found').toBeTruthy();
  return match as HTMLButtonElement;
}

function cancelButton(): HTMLButtonElement {
  const match = Array.from(
    document.querySelectorAll<HTMLButtonElement>('[data-slot="dialog-content"] button'),
  ).find((b) => b.textContent?.trim() === "Cancel");
  expect(match, 'no button labeled "Cancel"').toBeTruthy();
  return match as HTMLButtonElement;
}

async function open(
  client: OpenCompanyClient,
  request: { kind: "reset"; company: string; name: string } | { kind: "create" },
) {
  await act(async () => {
    root.render(createElement(CreateCompanyDialog, { client, request, onClose, onCreated }));
  });
}

async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT =
    true;
  onClose = vi.fn();
  onCreated = vi.fn();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("cancelling a reset after the archive already landed", () => {
  it("reports archived=true when the create leg then fails", async () => {
    const provisionCompany = vi.fn((_body: ProvisionBody) =>
      Promise.reject(new ApiError(500, "internal_error", "internal error", true)),
    );
    await open(stubClient({ provisionCompany }), {
      kind: "reset",
      company: "acme",
      name: "Acme Robotics",
    });

    await act(async () => {
      submitButton().click();
    });
    await settle();

    // The archive landed and the create failed — the dialog is back to idle
    // with an error shown, not the create having succeeded.
    expect(document.querySelector('[data-testid="create-company-error"]')).toBeTruthy();
    expect(onCreated).not.toHaveBeenCalled();

    await act(async () => {
      cancelButton().click();
    });

    expect(onClose).toHaveBeenCalledExactlyOnceWith(true);
  });

  it("reports archived=false when Cancel is pressed before ever submitting", async () => {
    await open(stubClient({}), { kind: "reset", company: "acme", name: "Acme Robotics" });

    await act(async () => {
      cancelButton().click();
    });

    expect(onClose).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("reports archived=false for a plain create — there is nothing to have archived", async () => {
    const provisionCompany = vi.fn((_body: ProvisionBody) =>
      Promise.reject(new ApiError(500, "internal_error", "internal error", true)),
    );
    await open(stubClient({ provisionCompany }), { kind: "create" });

    await act(async () => {
      submitButton().click();
    });
    await settle();
    await act(async () => {
      cancelButton().click();
    });

    expect(onClose).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("reports archived=true on the same successful-archive-then-failed-create path via Escape, not just Cancel", async () => {
    const provisionCompany = vi.fn((_body: ProvisionBody) =>
      Promise.reject(new ApiError(500, "internal_error", "internal error", true)),
    );
    await open(stubClient({ provisionCompany }), {
      kind: "reset",
      company: "acme",
      name: "Acme Robotics",
    });

    await act(async () => {
      submitButton().click();
    });
    await settle();

    const popup = document.querySelector<HTMLElement>('[data-slot="dialog-content"]');
    expect(popup, "dialog popup did not render").toBeTruthy();
    await act(async () => {
      popup!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
      );
    });

    expect(onClose).toHaveBeenCalledExactlyOnceWith(true);
  });
});
