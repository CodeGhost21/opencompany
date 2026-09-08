/**
 * Persists the nonce of an unresolved deliberate-duplicate ("force new")
 * invoice send, so it survives whatever unmounts `SendInvoiceDialog` —
 * navigating away from Finance and back, or a page reload.
 *
 * The nonce is component state (`SendInvoiceDialog`'s own `forceNewNonce`)
 * for as long as the component stays mounted, which is enough for a
 * same-mount close/reopen retry. It is not enough for a genuine remount: an
 * ambiguous forced-send failure (timeout, dropped connection) followed by
 * leaving the page loses both the nonce and the fact that a forced send is
 * outstanding, so a retry after coming back checks the box again, mints a
 * fresh nonce, and can raise a second real invoice — the exact failure this
 * dialog exists to prevent, one layer further out.
 *
 * Scoped by `LocalScope` (connection + company), not company alone, for the
 * same reason every other browser-local key in the console is — see
 * `connections/types.ts` — and further scoped by the invoice's own
 * content-derived key, because a nonce is meaningful only for the invoice it
 * was minted against. A single slot per connection/company let an unrelated
 * invoice's write or successful resolve clobber a still-unresolved one: start
 * (or finish) a forced send for invoice B while invoice A's forced send from
 * earlier is still unresolved, and A's entry was silently overwritten or
 * erased, so a later retry of A minted a fresh nonce and could bill it twice.
 * One key per invoice means resolving B never touches A's entry.
 */

import { type LocalScope, scopedKey } from "@/connections/types";

function keyFor(scope: LocalScope, invoiceKey: string): string {
  return scopedKey(`oc.finance.invoice-force-new.${invoiceKey}`, scope);
}

/**
 * `localStorage`, or `null` where it isn't usable.
 *
 * Access itself can throw — Safari's private mode and a "block all cookies"
 * setting both make the property itself raise rather than return a dud
 * object. Losing the latch is the safe direction: the dialog degrades to
 * exactly its pre-fix behavior, not a crash.
 */
function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

/**
 * The nonce of a forced send that has not yet resolved for this scope and
 * invoice, or `undefined` if none is outstanding.
 */
export function readUnresolvedForceNew(scope: LocalScope, invoiceKey: string): string | undefined {
  const raw = storage()?.getItem(keyFor(scope, invoiceKey));
  return raw && raw.length > 0 ? raw : undefined;
}

/** Marks a forced send of one invoice as attempted and unresolved. */
export function writeUnresolvedForceNew(
  scope: LocalScope,
  invoiceKey: string,
  nonce: string,
): void {
  const store = storage();
  if (!store) return;
  try {
    store.setItem(keyFor(scope, invoiceKey), nonce);
  } catch {
    // A full or read-only quota is not worth failing a send over.
  }
}

/** Clears the latch: this invoice's forced send succeeded, or the operator started over. */
export function clearUnresolvedForceNew(scope: LocalScope, invoiceKey: string): void {
  const store = storage();
  if (!store) return;
  try {
    store.removeItem(keyFor(scope, invoiceKey));
  } catch {
    // Nothing to do about a store that will not let us clear it either.
  }
}
