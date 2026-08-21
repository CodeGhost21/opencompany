// The company's lists, as a hook — the single source of truth the sidebar
// (one row per list) and a list's own screen both read, replacing the private
// `listLedgers()` call `LedgersView` used to make on its own.
//
// Modeled on `use-board-columns.ts`: one read per company, on mount and on
// company change. Unlike that hook, this one is also mutated from elsewhere —
// declaring or retiring a list happens on Manage Lists, not here — so callers
// need a way to say "read it again", which is `refresh`. There is no SSE event
// for a declare or a retire (`use-events.ts` carries nothing ledger-shaped), so
// without an explicit `refresh` the sidebar would only ever learn about a new
// list on the next full reload.

import { useCallback, useEffect, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { listLedgers, type LedgerSummary } from "@/api/ledgers";

export interface LedgerNav {
  /** Every list this company holds, or `[]` before the first read lands. */
  ledgers: LedgerSummary[];
  /** Declarations that could not be loaded, and why (see `LedgerList.faults`). */
  faults: string[];
  /** How many more this company may declare. */
  remaining: number;
  /**
   * Whether the first read for the current company is still in flight.
   *
   * `[]` is the honest answer both while loading and for a company with no
   * lists at all — the same "not yet vs. genuinely empty" distinction
   * `useBoardColumns`'s own doc comment draws. A caller that must tell the two
   * apart (the sidebar, so it does not render an empty state as if it were
   * final) reads this rather than inferring it from an empty array.
   */
  loading: boolean;
  /** Re-reads the list. Called after a declare or a retire. */
  refresh: () => Promise<void>;
}

export function useLedgerNav(
  client: OpenCompanyClient,
  company: string | null,
): LedgerNav {
  const [ledgers, setLedgers] = useState<LedgerSummary[]>([]);
  const [faults, setFaults] = useState<string[]>([]);
  const [remaining, setRemaining] = useState(0);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    if (!company) {
      setLedgers([]);
      setFaults([]);
      setRemaining(0);
      setLoading(false);
      return;
    }
    try {
      const list = await listLedgers(client, company);
      setLedgers(list.ledgers);
      setFaults(list.faults ?? []);
      setRemaining(list.remaining);
    } catch {
      // Swallowed on purpose, matching `useBoardColumns`: a failed read here
      // must not surface as an error banner over an otherwise-working shell.
      // The sidebar degrades to showing no list rows rather than reporting an
      // error nobody asked this hook for; a screen that reads a single list
      // directly (`LedgersView`) has its own error path for its own read.
      setLedgers([]);
      setFaults([]);
      setRemaining(0);
    } finally {
      setLoading(false);
    }
  }, [client, company]);

  useEffect(() => {
    setLoading(true);
    void load();
  }, [load]);

  return { ledgers, faults, remaining, loading, refresh: load };
}
