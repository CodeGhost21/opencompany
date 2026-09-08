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
 * `connections/types.ts`.
 */

import { type LocalScope, scopedKey } from "@/connections/types";

function keyFor(scope: LocalScope): string {
  return scopedKey("oc.finance.invoice-force-new", scope);
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

/** A forced send that was attempted for one specific invoice and never resolved. */
export interface UnresolvedForceNew {
  /** The invoice's content-derived key, WITHOUT the nonce folded in. */
  invoiceKey: string;
  nonce: string;
}

/**
 * The unresolved forced send held for this scope, or `undefined`.
 *
 * Carries the invoice it belongs to, because a nonce is meaningful only for
 * the invoice it was minted against: restoring it onto a different invoice
 * would fold one invoice's nonce into another's key and have the host dedupe
 * a genuinely new invoice away as a replay of the old one.
 */
export function readUnresolvedForceNew(scope: LocalScope): UnresolvedForceNew | undefined {
  const raw = storage()?.getItem(keyFor(scope));
  if (!raw) return undefined;
  try {
    const held: unknown = JSON.parse(raw);
    if (
      typeof held === "object" &&
      held !== null &&
      typeof (held as UnresolvedForceNew).invoiceKey === "string" &&
      typeof (held as UnresolvedForceNew).nonce === "string"
    ) {
      return held as UnresolvedForceNew;
    }
  } catch {
    // A value this cannot read is a value it must not act on.
  }
  return undefined;
}

/** Marks a forced send of one invoice as attempted and unresolved. */
export function writeUnresolvedForceNew(scope: LocalScope, held: UnresolvedForceNew): void {
  const store = storage();
  if (!store) return;
  try {
    store.setItem(keyFor(scope), JSON.stringify(held));
  } catch {
    // A full or read-only quota is not worth failing a send over.
  }
}

/** Clears the latch: the forced send succeeded, or the operator started over. */
export function clearUnresolvedForceNew(scope: LocalScope): void {
  const store = storage();
  if (!store) return;
  try {
    store.removeItem(keyFor(scope));
  } catch {
    // Nothing to do about a store that will not let us clear it either.
  }
}
