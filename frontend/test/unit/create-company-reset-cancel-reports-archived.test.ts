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
  status?: ReturnType<typeof vi.fn<(company?: string | null) => Promise<CompanyStatus>>>;
  provisionCompany?: ReturnType<typeof vi.fn<(body: ProvisionBody) => Promise<CompanyStatus>>>;
}) {
  return {
    carriesPlatformBearer: true,
    lifecycle: opts.lifecycle ?? vi.fn(() => Promise.resolve()),
    status: opts.status ?? vi.fn(() => Promise.resolve({ id: "acme" } as unknown as CompanyStatus)),
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

// Required after codex review comment 3864885200 — unrelated to what this
// file pins (whether `onClose` reports the archive as landed), so every
// submit needs one filled in first or the earlier email check blocks the
// archive leg these tests are about before it ever runs.
async function fillAdminEmail(value = "ceo@acme.test") {
  const input = document.querySelector<HTMLInputElement>("#create-company-admin");
  expect(input, "no admin-email field").toBeTruthy();
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  await act(async () => {
    setter.call(input, value);
    input!.dispatchEvent(new Event("input", { bubbles: true }));
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

    await fillAdminEmail();
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

    await fillAdminEmail();
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

    await fillAdminEmail();
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

  // Codex review on #1828, PR comment 3873186315, fresh evidence beyond the
  // reconciliation fix `create-company-reset-archive-network-error-
  // ambiguous.test.ts` already covers: when BOTH the archive attempt and its
  // reconciliation status lookup fail at the transport layer, the archive
  // may in fact have landed, but `archived` deliberately stays false (a
  // retry must remain possible). `onClose(archived)` used to report that
  // `false` at face value — an operator who closed here got no roster
  // refresh for a company that might already be archived.
  it("reports true, not false, when both the archive attempt and the reconciliation lookup are ambiguous", async () => {
    const lifecycle = vi.fn(() =>
      Promise.reject(new ApiError(0, "network_error", "network error", true)),
    );
    const status = vi.fn(() =>
      Promise.reject(new ApiError(0, "network_error", "network error", true)),
    );
    await open(stubClient({ lifecycle, status }), {
      kind: "reset",
      company: "acme",
      name: "Acme Robotics",
    });

    await fillAdminEmail();
    await act(async () => {
      submitButton().click();
    });
    await settle();

    // Confirms this test actually reached the double-failure branch, not
    // some other error path.
    const error = document.querySelector('[data-testid="create-company-error"]');
    expect(error?.textContent).toContain("Couldn't confirm");

    await act(async () => {
      cancelButton().click();
    });

    expect(onClose).toHaveBeenCalledExactlyOnceWith(true);
  });
});
