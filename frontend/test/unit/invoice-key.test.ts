import { describe, expect, it } from "vitest";

import { deriveInvoiceIdempotencyKey, type InvoiceIdentity } from "@/views/finance/invoiceKey";

/**
 * `deriveInvoiceIdempotencyKey` is the console's half of CONSOLE-ADMIN-015:
 * the key sent to Chargebee must be a function of *what is being invoiced*,
 * not of when the dialog happened to be open, so an operator who closes a
 * dialog after an ambiguous result and reopens it to resend does not bill the
 * customer a second time.
 */

const BASE: InvoiceIdentity = {
  customerEmail: "alan@example.com",
  currencyCode: "USD",
  dueDays: 7,
  lineItems: [{ description: "Consulting", amountInMinorUnits: 125_000 }],
};

describe("deriveInvoiceIdempotencyKey", () => {
  it("is a fixed value, not merely self-consistent", () => {
    // Pinned as a literal, mirroring the host's own
    // `a_derived_key_is_a_fixed_value_not_merely_self_consistent` test
    // (src/chargebee/api.rs): a hash stable only within one JS engine still
    // bills a customer twice if a retry is served by a different browser
    // build. "Same input, same key" has to hold across engines, and the only
    // way to assert that is to write the value down.
    expect(deriveInvoiceIdempotencyKey(BASE)).toBe("console-invoice-059233e750f98a22");
  });

  it("derives the same key for the same invoice, called twice independently", () => {
    const a = deriveInvoiceIdempotencyKey({ ...BASE });
    const b = deriveInvoiceIdempotencyKey({ ...BASE, lineItems: [...BASE.lineItems] });
    expect(a).toBe(b);
  });

  it("is unaffected by how the operator capitalized or spaced the email and currency", () => {
    const tidy = deriveInvoiceIdempotencyKey(BASE);
    const messy = deriveInvoiceIdempotencyKey({
      ...BASE,
      customerEmail: "  Alan@Example.com  ",
      currencyCode: " usd ",
    });
    expect(messy).toBe(tidy);
  });

  it("differs when the customer email differs", () => {
    expect(deriveInvoiceIdempotencyKey({ ...BASE, customerEmail: "bob@example.com" })).not.toBe(
      deriveInvoiceIdempotencyKey(BASE),
    );
  });

  it("differs when the amount differs", () => {
    const other: InvoiceIdentity = {
      ...BASE,
      lineItems: [{ description: "Consulting", amountInMinorUnits: 125_001 }],
    };
    expect(deriveInvoiceIdempotencyKey(other)).not.toBe(deriveInvoiceIdempotencyKey(BASE));
  });

  it("differs when the currency differs", () => {
    expect(deriveInvoiceIdempotencyKey({ ...BASE, currencyCode: "EUR" })).not.toBe(
      deriveInvoiceIdempotencyKey(BASE),
    );
  });

  it("differs when the due-days term differs, including present vs. absent", () => {
    const noDueDays = deriveInvoiceIdempotencyKey({ ...BASE, dueDays: undefined });
    const sevenDays = deriveInvoiceIdempotencyKey({ ...BASE, dueDays: 7 });
    const fourteenDays = deriveInvoiceIdempotencyKey({ ...BASE, dueDays: 14 });
    expect(new Set([noDueDays, sevenDays, fourteenDays]).size).toBe(3);
  });

  it("differs when the line item description differs", () => {
    const other: InvoiceIdentity = {
      ...BASE,
      lineItems: [{ description: "Consulting, April", amountInMinorUnits: 125_000 }],
    };
    expect(deriveInvoiceIdempotencyKey(other)).not.toBe(deriveInvoiceIdempotencyKey(BASE));
  });

  it("does not let adjacent fields bleed into one another", () => {
    // Without a separator between a description and the amount that follows
    // it, description "1" + amount 23 and description "12" + amount 3 would
    // both concatenate to the same bytes. Named keys already guard most of
    // this, but the field terminator is what actually closes the hole — the
    // same property the host's own test asserts for `derived_idempotency_key`.
    const a = deriveInvoiceIdempotencyKey({
      ...BASE,
      lineItems: [{ description: "1", amountInMinorUnits: 23 }],
    });
    const b = deriveInvoiceIdempotencyKey({
      ...BASE,
      lineItems: [{ description: "12", amountInMinorUnits: 3 }],
    });
    expect(a).not.toBe(b);
  });

  it("is unaffected by the nonce argument when none is supplied", () => {
    expect(deriveInvoiceIdempotencyKey(BASE, undefined)).toBe(deriveInvoiceIdempotencyKey(BASE));
    expect(deriveInvoiceIdempotencyKey(BASE, "")).toBe(deriveInvoiceIdempotencyKey(BASE));
  });

  it("a supplied nonce deliberately forces a different key for an identical invoice", () => {
    const withoutNonce = deriveInvoiceIdempotencyKey(BASE);
    const withNonce = deriveInvoiceIdempotencyKey(BASE, "one-off-abc");
    expect(withNonce).not.toBe(withoutNonce);
  });

  it("two different deliberate resends of the same invoice get two different keys", () => {
    const first = deriveInvoiceIdempotencyKey(BASE, "nonce-1");
    const second = deriveInvoiceIdempotencyKey(BASE, "nonce-2");
    expect(first).not.toBe(second);
  });
});
