// @vitest-environment jsdom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import { SendInvoiceDialog } from "@/views/finance/SendInvoiceDialog";

/**
 * The due-days guard in the invoice dialog.
 *
 * `due_days` is the one field that can silently become "no due date" (a `NaN`
 * serializes as `null`) or reach the wire in a shape the server rejects (a
 * decimal) — so the dialog has to parse it once and refuse to send until it is
 * either empty or a valid non-negative safe integer.
 */

vi.mock("sonner", () => ({ toast: { info: vi.fn(), success: vi.fn(), error: vi.fn() } }));

let container: HTMLDivElement;
let root: Root;

const sent = vi.fn();

const CLIENT = {
  scopeFor: () => "/api/v1/companies/acme",
} as unknown as OpenCompanyClient;

function fill(field: string, value: string) {
  const input = document.querySelector<HTMLInputElement>(`[data-testid="${field}"]`);
  expect(input).not.toBeNull();
  act(() => {
    input!.value = value;
    input!.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function at(testid: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
}

async function show(dueDays: string) {
  await act(async () => {
    root.render(
      createElement(SendInvoiceDialog, {
        client: CLIENT,
        company: "acme",
        site: "acme-test",
        open: true,
        onOpenChange: () => {},
        onSent: sent,
      }),
    );
  });
  fill("invoice-email", "alan@example.com");
  fill("invoice-description", "Consulting");
  fill("invoice-amount", "1250.00");
  fill("invoice-due-days", dueDays);
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe("the due-days field", () => {
  it("accepts an empty field", async () => {
    await show("");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(false);
  });

  it("accepts a whole non-negative number", async () => {
    await show("7");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(false);
  });

  it("disables send on non-numeric input", async () => {
    await show("abc");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(true);
  });

  it("disables send on a decimal", async () => {
    await show("1.5");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(true);
  });

  it("disables send on a negative number", async () => {
    await show("-1");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(true);
  });

  it("disables send on an out-of-safe-range number", async () => {
    await show("999999999999999999999");
    expect(at("invoice-send")?.hasAttribute("disabled")).toBe(true);
  });
});
