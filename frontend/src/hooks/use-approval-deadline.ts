import { useEffect, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { getPolicy } from "@/api/policy";

/** The historical fallback when a host's policy omits the deadline. */
const DEFAULT_TTL_HOURS = 24;

/**
 * The queue's "Each one has a deadline" sentence, read from the company policy.
 *
 * An older host's `/policy` still returns 200 but omits `approvalTtlHours`, so
 * the value is normalized to the historical 24-hour default instead of leaking
 * `undefined` into the rendered sentence. A new scoped read restarts from that
 * default too: when the operator switches company and the next read fails, the
 * previous company's deadline must not carry into the new one's queue.
 */
export function useApprovalDeadline(
  client: OpenCompanyClient,
  company: string | null,
): number {
  const [hours, setHours] = useState(DEFAULT_TTL_HOURS);
  useEffect(() => {
    let live = true;
    setHours(DEFAULT_TTL_HOURS);
    void getPolicy(client, company)
      .then((policy) => {
        if (live) setHours(policy.approvalTtlHours ?? DEFAULT_TTL_HOURS);
      })
      .catch(() => {
        // A policy read is explanatory here. Keep the historical default if an
        // older or temporarily unreachable host cannot serve it.
      });
    return () => {
      live = false;
    };
  }, [client, company]);
  return hours;
}
