/**
 * Derives the Chargebee idempotency key for a `SendInvoiceDialog` send.
 *
 * A plain module with no React in it, matching `money.ts` — the derivation is
 * pure and worth testing without mounting a component.
 *
 * # Why derived from the invoice, not minted per dialog-open
 *
 * The host itself (`chargebee::api::derived_idempotency_key`) hashes the
 * outgoing form when the caller supplies no key, so that a transport retry of
 * the *same* request collapses to one invoice. The console used to defeat that
 * by always supplying a key, minted once per dialog-open — which protects a
 * same-session double-click and nothing else. An operator who sees an
 * ambiguous result (timeout, dropped connection, 5xx), closes the dialog and
 * reopens it to try again gets a brand-new random key on reopen, so the retry
 * looks like an unrelated invoice to Chargebee and bills the customer twice.
 *
 * The fix is the same idea the host already uses: hash the fields that
 * identify *what is being invoiced* — the customer, the currency, the due
 * term, and the line items — so the key is a function of the invoice's
 * content, not of when the dialog happened to be open. Two sends of the same
 * invoice hash the same however many times the dialog is closed, reopened, or
 * the tab reloaded; changing any of those fields changes the key.
 *
 * `customer_name` and `auto_collection` are left out on purpose, mirroring the
 * host: the host's own hash is taken over the resolved `customer_id` (not the
 * name, which only affects customer creation) plus currency, due term and line
 * items — the console does not know `customer_id` yet at derive-time, so email
 * stands in for customer identity here.
 *
 * # FNV-1a, not a JS hash builtin
 *
 * Same reasoning as the host: the key has to be stable as a *value*, across
 * browser sessions and app versions, not merely within one runtime's hash
 * implementation. The field separators (`=` and `&` after every field) keep
 * `["ab", "c"]` from hashing the same as `["a", "bc"]`.
 *
 * # Deliberate resend
 *
 * An operator who wants to send the same invoice twice on purpose — the first
 * attempt genuinely failed and they want a second, distinct one, not a
 * dedup — needs an escape valve, the same way the host's tool schema lets a
 * caller pass a distinct `idempotency_key` to get a second invoice. `nonce`
 * exists for that: pass a freshly generated value only when the operator has
 * explicitly said "this is a new invoice, not a retry" (`invoice-force-new` in
 * the dialog), and it is folded into the hash so the key diverges. Omitted
 * (the default), the key is fully determined by the invoice's content.
 */

const FNV_OFFSET_BASIS = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const MASK_64 = 0xffffffffffffffffn;

const encoder = new TextEncoder();

function fnv1a64(fields: readonly (readonly [string, string])[]): bigint {
  let hash = FNV_OFFSET_BASIS;
  const eat = (text: string) => {
    for (const byte of encoder.encode(text)) {
      hash ^= BigInt(byte);
      hash = (hash * FNV_PRIME) & MASK_64;
    }
  };
  for (const [key, value] of fields) {
    eat(key);
    eat("=");
    eat(value);
    eat("&");
  }
  return hash;
}

/** One line item, as identified for the key — same shape as `ChargeLine`. */
export interface InvoiceKeyLineItem {
  description: string;
  amountInMinorUnits: number;
}

/** The fields of a `SendInvoiceDialog` send that identify "the same invoice". */
export interface InvoiceIdentity {
  customerEmail: string;
  currencyCode: string;
  /** `undefined` means "no due date supplied" (Chargebee's site default). */
  dueDays: number | undefined;
  lineItems: readonly InvoiceKeyLineItem[];
}

/**
 * Derives the idempotency key for an invoice send.
 *
 * Deterministic in `identity` alone. Pass `nonce` only for a deliberate
 * resend — any non-empty, distinct value forces a different key.
 */
export function deriveInvoiceIdempotencyKey(identity: InvoiceIdentity, nonce?: string): string {
  const fields: [string, string][] = [
    ["customer_email", identity.customerEmail.trim().toLowerCase()],
    ["currency_code", identity.currencyCode.trim().toUpperCase()],
    ["due_days", identity.dueDays === undefined ? "" : String(identity.dueDays)],
  ];
  identity.lineItems.forEach((item, index) => {
    fields.push([`line_item[${index}].description`, item.description.trim()]);
    fields.push([`line_item[${index}].amount`, String(item.amountInMinorUnits)]);
  });
  if (nonce) {
    fields.push(["force_new_nonce", nonce]);
  }
  return `console-invoice-${fnv1a64(fields).toString(16).padStart(16, "0")}`;
}
