import { useCallback, useEffect, useState } from "react";

/** The two ways a ledger can render its entries. */
export type LedgerViewMode = "board" | "list";

/** Hash query key that keeps the selected ledger rendering in browser history. */
export const LEDGER_VIEW_PARAM = "view";

/** Reads a valid ledger view from a hash, defaulting unknown values to Board. */
export function readLedgerViewMode(hash = window.location.hash): LedgerViewMode {
  const [, query = ""] = hash.split("?");
  return new URLSearchParams(query).get(LEDGER_VIEW_PARAM) === "list"
    ? "list"
    : "board";
}

/**
 * Keeps the Board/List choice beside the ledger route (`?view=list`).
 *
 * This is navigation state, rather than component state: changing it pushes a
 * history entry and a browser Back returns to the rendering the operator left.
 */
export function useLedgerViewMode(): [
  LedgerViewMode,
  (mode: LedgerViewMode) => void,
] {
  const [mode, setMode] = useState(readLedgerViewMode);

  useEffect(() => {
    const onHashChange = () => setMode(readLedgerViewMode());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  const set = useCallback((next: LedgerViewMode) => {
    const [path, query = ""] = window.location.hash.replace(/^#/, "").split("?");
    const params = new URLSearchParams(query);
    if (next === "list") params.set(LEDGER_VIEW_PARAM, "list");
    else params.delete(LEDGER_VIEW_PARAM);
    const suffix = params.toString().replace(/=(?=&|$)/g, "");
    const nextHash = `#${path}${suffix ? `?${suffix}` : ""}`;
    if (window.location.hash !== nextHash) window.location.hash = nextHash;
    setMode(next);
  }, []);

  return [mode, set];
}
