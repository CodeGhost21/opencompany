import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import type { ApprovalSummary, CompanyStatus } from "@/api/types";
import { startVisiblePolling } from "@/lib/visible-poll";

const POLL_MS = 5000;

export interface CompanyFeed {
  status: CompanyStatus;
  approvals: ApprovalSummary[];
  /** Wall-clock at the last successful refresh, for relative timestamps. */
  now: number;
  refresh: () => Promise<void>;
}

/**
 * `status` with its approval count reconciled to the queue fetched beside it
 * (issue #932).
 *
 * The host has one source of truth — the journal's parked set — but the console
 * reads it over **two** requests: `GET …/{id}` carries `pending_approvals` as a
 * number, `GET …/{id}/approvals` carries the rows themselves. Nothing makes the
 * two samples the same instant, and they systematically are not: the status
 * handler awaits a store load for the company's name and lifecycle *before* it
 * counts, so its number is taken later than the queue's. While a workflow run
 * parks gates, every poll therefore lands a count that is ahead of the list it
 * arrived with — the sidebar badge said 18 beside a page that said 14.
 *
 * Reconciling here rather than at the badge keeps it to one rule for one feed:
 * every consumer of this hook sees a count and a queue that were the same
 * response. The queue wins because it is the actionable one — it is what the
 * Approvals page can list and the operator can decide, so a badge promising
 * more than the page can show is the half that is wrong.
 *
 * This does **not** make `CompanyStatus.pending_approvals` redundant. The
 * company picker renders one count per company without fetching any of their
 * queues, and there the number is the only thing there is.
 *
 * Applied only where a status and a queue arrived **together**. Reconciling a
 * status against a queue this console has not read yet is a different claim
 * — see the reset effect in [`useCompany`] for what that costs.
 */
export function withApprovalCount(
  status: CompanyStatus,
  approvals: ApprovalSummary[],
): CompanyStatus {
  // Returned as-is when they already agree, which is the common case: a new
  // object every poll would re-render every consumer of `status` five times a
  // minute for a value that did not change.
  return status.pending_approvals === approvals.length
    ? status
    : { ...status, pending_approvals: approvals.length };
}

/**
 * Polls a single company's status and approvals on an interval, keeping the
 * last good view on transient errors. Re-subscribes when the company changes.
 */
export function useCompany(
  client: OpenCompanyClient,
  company: string | null,
  initialStatus: CompanyStatus,
): CompanyFeed {
  const [status, setStatus] = useState<CompanyStatus>(initialStatus);
  const [approvals, setApprovals] = useState<ApprovalSummary[]>([]);
  const [now, setNow] = useState(() => Date.now());
  const mounted = useRef(true);

  // Reset to the freshly-picked company's status when switching.
  //
  // Deliberately NOT reconciled against the empty queue below (#932). There is
  // no approvals response to reconcile *against* here — the queue is empty
  // because this console has not read the new company's yet, which is an
  // absence, not a count of zero. Forcing the badge to 0 and letting the first
  // poll raise it makes every load and every company switch a rising edge, and
  // the rising edge is what fires the "needs a sign-off" push in `useEvents` —
  // whose detector is seeded precisely so that the first read never toasts.
  useEffect(() => {
    setStatus(initialStatus);
    setApprovals([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [company]);

  const refresh = useCallback(async () => {
    try {
      const [s, a] = await Promise.all([client.status(company), client.approvals(company)]);
      if (!mounted.current) return;
      setStatus(withApprovalCount(s, a));
      setApprovals(a);
      setNow(Date.now());
    } catch {
      /* transient; keep the last good view */
    }
  }, [client, company]);

  // Issue #581: gated on visibility. This hook has one instance per open
  // console tab, and its refresh is two requests — so an ungated interval meant
  // every tab left in the background kept costing the host 24 requests a minute
  // for a company nobody was looking at. The mount fetch stays here rather than
  // moving into the poller: the poller deliberately does not load on start, and
  // the hidden → visible catch-up read it does perform covers the staleness of
  // a tab coming back.
  useEffect(() => {
    mounted.current = true;
    void refresh();
    const dispose = startVisiblePolling(() => void refresh(), POLL_MS);
    return () => {
      mounted.current = false;
      dispose();
    };
  }, [refresh]);

  return { status, approvals, now, refresh };
}
