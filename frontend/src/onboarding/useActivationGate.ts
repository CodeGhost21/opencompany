import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { type ActivationStatus, getActivation } from "@/api/activation";
import { resolveActivationReadError } from "@/onboarding/gate-logic";
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

/**
 * How soon a `getActivation` failure that `resolveActivationReadError` does
 * NOT settle (a network error, a proxy 5xx — anything but the legacy-host
 * `404`) retries, rather than waiting for the next `POLL_MS` tick. Mirrors
 * `GATE_ADMIN_CHECK_RETRY_MS` (`app-shell.tsx`, PR #1875 review finding):
 * a real, non-activated company whose first read merely glitched should get
 * a fast second attempt, not the standard 5s cadence — the gap between "the
 * shell renders unblocked" and "the gate correctly locks it" is exactly the
 * window an operator could start clicking around in, and a fast retry keeps
 * that window small instead of leaving it as wide as a full poll interval.
 */
const ACTIVATION_READ_RETRY_MS = 3000;

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
  /** Pending fast retry from a read `resolveActivationReadError` did not settle. */
  const retryTimer = useRef<ReturnType<typeof setTimeout>>();

  const clearRetry = () => {
    if (retryTimer.current !== undefined) {
      clearTimeout(retryTimer.current);
      retryTimer.current = undefined;
    }
  };

  const load = useCallback(async () => {
    if (activated.current) return;
    clearRetry();
    const gen = ++generation.current;
    try {
      const next = await getActivation(client, company);
      if (gen !== generation.current) return;
      setStatus(next);
      setChecked(true);
      if (next.isActivated) activated.current = true;
    } catch (err) {
      if (gen !== generation.current) return;
      const outcome = resolveActivationReadError(err);
      if (outcome.settled) {
        // A host predating this route: definitively no such funnel, and
        // retrying will not change that. `status` stays `null` —
        // `shouldShowOnboardingGate`'s own `!status` guard keeps the gate off
        // permanently, same as before this fix.
        setChecked(true);
        return;
      }
      // A transient failure (network error, 5xx) — not an answer, so do not
      // settle `checked` on it. Retry sooner than the regular poll cadence
      // instead of waiting out the full `POLL_MS` tick — see
      // `ACTIVATION_READ_RETRY_MS`.
      retryTimer.current = setTimeout(() => {
        if (gen !== generation.current) return;
        void load();
      }, ACTIVATION_READ_RETRY_MS);
    }
  }, [client, company]);

  useEffect(() => {
    if (!enabled) return;
    activated.current = false;
    setChecked(false);
    setStatus(null);
    void load();
    const stopPolling = startVisiblePolling(() => void load(), POLL_MS);
    return () => {
      stopPolling();
      clearRetry();
    };
  }, [enabled, load]);

  return { checked, status, refresh: load };
}
