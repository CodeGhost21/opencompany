// A hibernating cloud tenant is not a host that is gone.
//
// The hosted control plane runs a wake-on-request proxy: a tenant container
// that has been idle is not running, and the proxy starts it when a request
// arrives, blocking on `/healthz` until it answers. The first request after an
// idle period therefore takes seconds, which is the whole point — a tenant
// costs nearly nothing while nobody is using it.
//
// The console's probe assumes a host that is either listening or gone, and
// applied to a cold tenant it concludes `down`. That is wrong in the way that
// matters most: it tells an operator their company is broken when it is asleep,
// and the row stays red because nothing re-probes on its own.
//
// So the patience belongs to the connector, not to the prober. A `cloud`
// connection that cannot be reached keeps trying, and stays `connecting` while
// it does — labelled "Waking…", because "Connecting…" for ninety seconds reads
// as a hang.
//
// See `docs/spec/runtime/connectors.md`.

import type { ConnectionStatus, Connector } from "./types";

/**
 * How long a tenant is given to wake before it is called unreachable.
 *
 * A ceiling this client imposes, not one the manager reports: there is no
 * surface that says what its startup timeout is. Generous on purpose — the
 * cost of being too patient is a row that says "Waking…" for a while longer
 * than it needed to, and the cost of being too impatient is telling somebody
 * their company is gone.
 */
export const CLOUD_WAKE_WINDOW_MS = 90_000;

/** The longest gap between attempts, once the backoff has climbed to it. */
export const WAKE_RETRY_CEILING_MS = 8_000;

/**
 * Whether a failed probe is worth another attempt.
 *
 * Three conditions, and each excludes a case that would otherwise retry
 * forever over something retrying cannot fix:
 *
 * - **only `cloud`.** No other connector has anything waking it up. A local
 *   host that is not listening is not listening; an `ssh` tunnel that is down
 *   is a tunnel to report, not a host to wait for.
 * - **only `down`.** `unauthenticated` is an *answer* — the tenant is awake
 *   and refusing this credential — and retrying it would hide the sign-in the
 *   operator has to do behind a spinner.
 * - **only inside the window**, after which the honest reading is that
 *   something is actually wrong.
 */
export function keepWaking(
  connector: Connector,
  status: ConnectionStatus,
  elapsedMs: number,
): boolean {
  if (connector.kind !== "cloud") return false;
  if (status !== "down") return false;
  return elapsedMs < CLOUD_WAKE_WINDOW_MS;
}

/**
 * How long to wait before attempt `attempt` (zero-based).
 *
 * Exponential and capped. A tenant takes seconds rather than milliseconds to
 * boot, so hammering it adds load to the thing being waited for; and the cap
 * keeps the last attempt near the end of the window rather than well past it.
 */
export function wakeRetryDelay(attempt: number): number {
  return Math.min(1_000 * 2 ** attempt, WAKE_RETRY_CEILING_MS);
}
