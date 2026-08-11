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
  useEffect(() => {
    setStatus(initialStatus);
    setApprovals([]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [company]);

  const refresh = useCallback(async () => {
    try {
      const [s, a] = await Promise.all([client.status(company), client.approvals(company)]);
      if (!mounted.current) return;
      setStatus(s);
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
