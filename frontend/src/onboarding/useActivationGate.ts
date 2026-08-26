import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { type ActivationStatus, getActivation } from "@/api/activation";
import { startVisiblePolling } from "@/lib/visible-poll";

/**
 * How often the gate re-reads the funnel while it is on screen.
 *
 * The gate has no SSE frame of its own to key a refresh off — the three steps
 * it watches live behind three different flows (a manifest write, an OAuth
 * round trip, a workflow run) and none of them tells this hook "something
 * changed" directly. Polling on the same cadence `useCompany` already uses for
 * the sidebar is cheap (the host's own read short-circuits to a record load
 * once the latch is set — see `compute_and_latch`) and buys "re-poll after
 * each [step]" (issue #1844's own words) without wiring a bespoke event for
 * three surfaces that do not otherwise need one.
 */
const POLL_MS = 5000;

export interface ActivationGate {
  /** Whether the first read has landed — before this, render nothing blocking. */
  checked: boolean;
  /** `null` only before the first read lands, or after a read that failed. */
  status: ActivationStatus | null;
  /** Re-reads the funnel immediately — called after an in-gate action. */
  refresh: () => Promise<void>;
}

/**
 * Polls `GET {scope}/activation` for as long as the caller wants it running,
 * re-subscribing whenever `company` changes.
 *
 * `enabled` lets the shell stop polling once the gate has nothing left to
 * decide (the company is activated, or the operator dismissed it) instead of
 * every open tab quietly reading a route it no longer renders anything from.
 */
export function useActivationGate(
  client: OpenCompanyClient,
  company: string | null,
  enabled: boolean,
): ActivationGate {
  const [checked, setChecked] = useState(false);
  const [status, setStatus] = useState<ActivationStatus | null>(null);
  const generation = useRef(0);
  /**
   * Set once a read reports the latch. `isActivated` is monotonic on the host
   * (`ActivationStatus::is_activated`'s own contract) — once true it can never
   * go false again — so this lets every later tick short-circuit before the
   * network call instead of polling a settled answer forever.
   */
  const activated = useRef(false);

  const load = useCallback(async () => {
    if (activated.current) return;
    const gen = ++generation.current;
    try {
      const next = await getActivation(client, company);
      if (gen !== generation.current) return;
      setStatus(next);
      setChecked(true);
      if (next.isActivated) activated.current = true;
    } catch {
      // A host predating this route, or a transient failure. Keep the last
      // good read (if any) rather than blank it — flipping a company that was
      // known-activated back to "unknown" over one dropped poll would be a
      // second, worse lie than a gate that is a beat late to open.
      if (gen !== generation.current) return;
      setChecked(true);
    }
  }, [client, company]);

  useEffect(() => {
    if (!enabled) return;
    activated.current = false;
    setChecked(false);
    setStatus(null);
    void load();
    return startVisiblePolling(() => void load(), POLL_MS);
  }, [enabled, load]);

  return { checked, status, refresh: load };
}
