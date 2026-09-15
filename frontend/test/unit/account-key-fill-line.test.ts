// Pure coverage for the account-key dialog's one conditional line (Q9) and
// its "which slots would this save fill" decision — keys rework, issue
// #2306, slice 4b. No React, no host: `docs/key-reworks/phase-4b-account-dialog.md`
// §3.3, with the wording override (decision "X5", 2026-09-15) that the LLM
// branch never claims the key "connects" or "is connected" for LLM — the
// fan-out (4a) never creates a `tinyhumans` row without a model, so a key
// with no row behind it has done nothing for LLM yet.

import { describe, expect, it } from "vitest";

import type { CompanyCredentialStatus } from "@/api/credential";
import {
  COMPOSIO_PAGE_HREF,
  LLM_PAGE_HREF,
  accountFillLine,
  accountFills,
  modelStepTitle,
} from "@/views/connections/account-fill";

function status(overrides: Partial<CompanyCredentialStatus> = {}): CompanyCredentialStatus {
  return {
    configured: true,
    source: "company",
    notice: "notice",
    hubLink: false,
    ...overrides,
  };
}

describe("accountFills", () => {
  it("reads false HasOwnKey as a slot this save would fill", () => {
    expect(
      accountFills(status({ inferenceHasOwnKey: false, composioHasOwnKey: false })),
    ).toEqual({ llm: true, composio: true });
  });

  it("reads true HasOwnKey as a slot this save would leave alone", () => {
    expect(accountFills(status({ inferenceHasOwnKey: true, composioHasOwnKey: true }))).toEqual({
      llm: false,
      composio: false,
    });
  });

  it("mixes the two independently", () => {
    expect(accountFills(status({ inferenceHasOwnKey: true, composioHasOwnKey: false }))).toEqual({
      llm: false,
      composio: true,
    });
  });

  it("is null when either field is missing (an older host)", () => {
    expect(accountFills(status({ inferenceHasOwnKey: true, composioHasOwnKey: undefined }))).toBe(
      null,
    );
    expect(accountFills(status({ inferenceHasOwnKey: undefined, composioHasOwnKey: true }))).toBe(
      null,
    );
    expect(accountFills(null)).toBe(null);
  });
});

describe("accountFillLine", () => {
  it("names both slots without claiming LLM is connected", () => {
    const line = accountFillLine({ llm: true, composio: true });
    expect(line).toBe(
      "Saving also adds this key to TinyHumans on the LLM page — choose a model there to finish — and connects it for Composio.",
    );
    expect(line?.toLowerCase()).not.toContain("connects tinyhumans for llm");
    expect(line?.toLowerCase()).not.toContain("llm is connected");
  });

  it("names only the LLM slot, and says a model is still needed", () => {
    const line = accountFillLine({ llm: true, composio: false });
    expect(line).toBe(
      "Saving also adds this key to TinyHumans on the LLM page — choose a model there to finish.",
    );
    expect(line?.toLowerCase()).not.toContain("connects");
  });

  it("names only the Composio slot — unaffected by the LLM wording override", () => {
    expect(accountFillLine({ llm: false, composio: true })).toBe(
      "Saving also connects TinyHumans for Composio.",
    );
  });

  it("is null when saving would fill neither slot", () => {
    expect(accountFillLine({ llm: false, composio: false })).toBe(null);
  });

  it("is null when the host did not say (accountFills returned null)", () => {
    expect(accountFillLine(null)).toBe(null);
  });
});

describe("modelStepTitle", () => {
  it("names the default when this model would also become one", () => {
    expect(modelStepTitle(true)).toBe("Choose the model new work uses");
  });

  it("names TinyHumans plainly otherwise", () => {
    expect(modelStepTitle(false)).toBe("Choose the model TinyHumans uses");
  });
});

describe("the dialog's two links", () => {
  it("point at the LLM and Composio connection pages", () => {
    expect(LLM_PAGE_HREF).toBe("#/connections/inference");
    expect(COMPOSIO_PAGE_HREF).toBe("#/connections/composio");
  });
});
