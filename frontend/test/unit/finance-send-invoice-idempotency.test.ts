// @vitest-environment jsdom

// CONSOLE-ADMIN-015: the idempotency key `SendInvoiceDialog` sends to the
// host must be derived from the invoice's own content, not from when the
// dialog was opened. Red-prove target: before the fix, the key was
// `console-${crypto.randomUUID()}`, minted once per dialog-open — so an
// operator who saw an ambiguous result (timeout, dropped connection, 5xx),
// closed the dialog and reopened it to resend the *same* invoice got an
// unrelated key, and Chargebee billed the customer twice.

import { act, createElement, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import type { Invoice, SendInvoice } from "@/api/finance";

const sendInvoice = vi.fn();

vi.mock("@/api/finance", async (importActual) => {
  const actual = await importActual<typeof import("@/api/finance")>();
  return { ...actual, sendInvoice };
});

vi.mock("sonner", () => ({
  toast: { error: vi.fn(), success: vi.fn(), info: vi.fn() },
}));

const { SendInvoiceDialog } = await import("@/views/finance/SendInvoiceDialog");
const { deriveInvoiceIdempotencyKey } = await import("@/views/finance/invoiceKey");

const CLIENT = { scopeFor: () => "/api/v1/companies/acme" } as unknown as OpenCompanyClient;

function invoiceReply(overrides: Partial<Invoice> = {}): Invoice {
  return {
    id: "inv_1",
    customer_id: "cus_1",
    status: "payment_due",
    currency_code: "USD",
    total_in_minor_units: 125_000,
    amount_due_in_minor_units: 125_000,
    amount_paid_in_minor_units: 0,
    due_date: null,
    line_items: ["Consulting"],
    payment_url: null,
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: Root;

function at(testid: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`[data-testid="${testid}"]`);
}

async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
}

/** Fills every required field the way an operator does, so React state updates. */
async function fillInvoiceFields(fields: {
  email: string;
  description: string;
  amount: string;
  dueDays?: string;
}): Promise<void> {
  await fill("invoice-email", fields.email);
  await fill("invoice-description", fields.description);
  await fill("invoice-amount", fields.amount);
  await fill("invoice-due-days", fields.dueDays ?? "");
}

async function fill(testid: string, value: string): Promise<void> {
  const input = at(testid) as HTMLInputElement | null;
  if (!input) throw new Error(`no input ${testid}`);
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/**
 * A real `.click()` rather than forcing `.checked` + a synthetic `change`
 * event: a click is what actually toggles a checkbox's property natively in
 * the DOM and is what React's controlled-input machinery expects to observe,
 * so it is the only way here that reliably reaches the component's `onChange`
 * (and therefore its `forceNew` state) rather than just repainting the input.
 */
async function toggleForceNew(checked: boolean): Promise<void> {
  const box = at("invoice-force-new") as HTMLInputElement | null;
  if (!box) throw new Error("no invoice-force-new checkbox");
  if (box.checked === checked) return;
  await act(async () => {
    box.click();
  });
}

async function clickSend(): Promise<void> {
  act(() => {
    at("invoice-send")?.click();
  });
  await settle();
}

function lastSentKey(): string {
  const calls = sendInvoice.mock.calls;
  expect(calls.length).toBeGreaterThan(0);
  const body = calls[calls.length - 1][2] as SendInvoice;
  expect(body.idempotency_key).toBeTruthy();
  return body.idempotency_key as string;
}

beforeEach(() => {
  (globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  act(() => {
    root = createRoot(container);
  });
  sendInvoice.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("SendInvoiceDialog idempotency key", () => {
  it("matches the pure derivation for the fields on the form", async () => {
    sendInvoice.mockResolvedValue(invoiceReply());
    act(() => {
      root.render(
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open: true,
          onOpenChange: () => {},
          onSent: () => {},
        }),
      );
    });
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00", dueDays: "7" });
    await clickSend();

    const expected = deriveInvoiceIdempotencyKey({
      customerEmail: "alan@example.com",
      currencyCode: "USD",
      dueDays: 7,
      lineItems: [{ description: "Consulting", amountInMinorUnits: 125_000 }],
    });
    expect(lastSentKey()).toBe(expected);
  });

  it("carries the same key when the same invoice is retried after a failed send", async () => {
    sendInvoice.mockRejectedValueOnce(new Error("timeout"));
    sendInvoice.mockResolvedValueOnce(invoiceReply());

    act(() => {
      root.render(
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open: true,
          onOpenChange: () => {},
          onSent: () => {},
        }),
      );
    });
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });

    await clickSend();
    const firstKey = lastSentKey();

    // The failed send leaves the dialog open with the same field values still
    // in it (the reset block only runs on success) — an operator clicking
    // Send again on the same invoice, no reopen involved.
    await clickSend();
    const secondKey = lastSentKey();

    expect(secondKey).toBe(firstKey);
  });

  /**
   * Mirrors `InvoicingView`: one `SendInvoiceDialog` instance stays mounted
   * for the life of the view, with `open` toggled by the parent's own
   * `sending` state — the dialog is never torn down and remounted just to
   * close it. `harness-reopen` stands in for whatever UI action
   * (re-clicking "Send invoice" on the finance page) sets that state back to
   * `true`.
   */
  function renderReopenableHarness() {
    function Harness() {
      const [open, setOpen] = useState(true);
      return createElement(
        "div",
        null,
        createElement(
          "button",
          { "data-testid": "harness-reopen", onClick: () => setOpen(true) },
          "reopen",
        ),
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open,
          onOpenChange: setOpen,
          onSent: () => {},
        }),
      );
    }
    act(() => {
      root.render(createElement(Harness));
    });
  }

  async function closeViaCancel(): Promise<void> {
    const cancel = Array.from(document.querySelectorAll("button")).find(
      (b) => b.textContent === "Cancel",
    );
    act(() => {
      cancel?.click();
    });
    await settle();
  }

  async function reopen(): Promise<void> {
    act(() => {
      at("harness-reopen")?.click();
    });
    await settle();
  }

  it("carries the same key across a genuine close and reopen of the same invoice", async () => {
    sendInvoice.mockRejectedValueOnce(new Error("timeout"));
    sendInvoice.mockResolvedValueOnce(invoiceReply());

    renderReopenableHarness();
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await clickSend();
    const firstKey = lastSentKey();

    // Operator sees the ambiguous failure, closes the dialog, and reopens it.
    await closeViaCancel();
    await reopen();

    // Field state persisted on the same component instance the whole time —
    // matching what an operator actually sees: the dialog remembers what
    // they typed rather than presenting a blank form.
    expect((at("invoice-email") as HTMLInputElement).value).toBe("alan@example.com");

    await clickSend();
    const secondKey = lastSentKey();

    expect(secondKey).toBe(firstKey);
  });

  it("gives a different invoice a different key", async () => {
    sendInvoice.mockResolvedValue(invoiceReply());
    act(() => {
      root.render(
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open: true,
          onOpenChange: () => {},
          onSent: () => {},
        }),
      );
    });
    await settle();

    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await clickSend();
    const firstKey = lastSentKey();

    await fillInvoiceFields({ email: "bob@example.com", description: "Consulting", amount: "1250.00" });
    await clickSend();
    const secondKey = lastSentKey();

    expect(secondKey).not.toBe(firstKey);
  });

  it("survives a full component remount for the same invoice", async () => {
    sendInvoice.mockResolvedValue(invoiceReply());

    function Mount({ instance }: { instance: string }) {
      return createElement(SendInvoiceDialog, {
        key: instance,
        client: CLIENT,
        company: "acme",
        site: "acme-test",
        open: true,
        onOpenChange: () => {},
        onSent: () => {},
      });
    }

    act(() => {
      root.render(createElement(Mount, { instance: "first" }));
    });
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await clickSend();
    const firstKey = lastSentKey();

    // A different component instance entirely — same identity as far as
    // React is concerned as a full unmount + fresh mount, e.g. navigating
    // away from Finance and back.
    act(() => {
      root.render(createElement(Mount, { instance: "second" }));
    });
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await clickSend();
    const secondKey = lastSentKey();

    expect(secondKey).toBe(firstKey);
  });

  it("a deliberate resend forces a different key from the default derivation", async () => {
    sendInvoice.mockResolvedValue(invoiceReply());
    act(() => {
      root.render(
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open: true,
          onOpenChange: () => {},
          onSent: () => {},
        }),
      );
    });
    await settle();
    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });

    await clickSend();
    const ordinaryKey = lastSentKey();

    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await toggleForceNew(true);
    await clickSend();
    const forcedKey = lastSentKey();

    expect(forcedKey).not.toBe(ordinaryKey);
  });

  it("two separate deliberate resends of the same invoice do not collide with each other", async () => {
    sendInvoice.mockResolvedValue(invoiceReply());
    act(() => {
      root.render(
        createElement(SendInvoiceDialog, {
          client: CLIENT,
          company: "acme",
          site: "acme-test",
          open: true,
          onOpenChange: () => {},
          onSent: () => {},
        }),
      );
    });
    await settle();

    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await toggleForceNew(true);
    await clickSend();
    const firstForced = lastSentKey();

    await fillInvoiceFields({ email: "alan@example.com", description: "Consulting", amount: "1250.00" });
    await toggleForceNew(true);
    await clickSend();
    const secondForced = lastSentKey();

    expect(secondForced).not.toBe(firstForced);
  });

  it("resets the deliberate-resend toggle on every open, so it cannot leak into the next send", async () => {
    renderReopenableHarness();
    await settle();

    await toggleForceNew(true);
    expect((at("invoice-force-new") as HTMLInputElement).checked).toBe(true);

    await closeViaCancel();
    await reopen();

    expect((at("invoice-force-new") as HTMLInputElement).checked).toBe(false);
  });
});
