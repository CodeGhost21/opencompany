// What a provider's detail view is allowed to claim (issue #404).
//
// The issue's rule — "do not invent a number … say so on screen rather than
// rendering a plausible-looking zero" — is not only about usage. Two of the
// three facts the view states are unavailable for most connections, and each
// has a wrong answer that looks right:
//
//  - **when it was connected**: only Composio records it, so a blank cell reads
//    as "never" for the two systems that never had one to record;
//  - **what has gone through it**: metered per *provider*, never per account,
//    and an unmetered host is not a host where nothing happened.
//
// These are the two decisions, kept out of the component so they can be pinned
// without a DOM.

import type { ProviderCallsDto } from "@/api/types";

/**
 * "connected 9 Aug 2026", or a statement that no date exists.
 *
 * Composio is the only one of the three connection systems that records one.
 * The native `oauth/{provider}` store keeps `{token, account}` and journals
 * nothing on connect; MCP has no such concept. So for those the date is not a
 * gap waiting on a timestamp column — it is a fact about the store, and saying
 * it plainly beats a blank the operator reads as "never".
 */
export function connectedOn(createdAt: string | undefined): string {
  if (createdAt === undefined || createdAt.trim() === "") {
    return "connection date not recorded";
  }
  const at = new Date(createdAt);
  // Composio's own field, forwarded verbatim by the host — which is exactly why
  // it is parsed defensively here. An unparseable string is not evidence of a
  // date, and `Invalid Date` on screen is worse than admitting there isn't one.
  if (Number.isNaN(at.getTime())) return "connection date not recorded";
  return `connected ${at.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  })}`;
}

/**
 * Successful calls counted against one Composio toolkit over the usage window.
 *
 * Matched on the bare slug. A remote MCP server's calls are recorded under
 * `mcp:<server>` (issue #698) precisely so the two namespaces cannot merge: a
 * company with a Composio `gmail` and an MCP server its operator also called
 * `gmail` would otherwise read one total as the other's. Prefix-matching here
 * would re-create by hand the collision that naming was chosen to prevent.
 *
 * Zero is a real answer, not a missing one — `composio_execute` meters every
 * successful call (`src/metering/oauth.rs`), so a provider with no row has had
 * no successful call in the window. The *unmetered* case is a failed usage read,
 * which the caller reports separately rather than flattening to zero here.
 */
export function callsForProvider(rows: readonly ProviderCallsDto[], slug: string): number {
  const want = slug.trim().toLowerCase();
  return rows.find((r) => r.provider.trim().toLowerCase() === want)?.calls ?? 0;
}
