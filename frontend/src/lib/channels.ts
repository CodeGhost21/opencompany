// What a saved Telegram channel is actually doing, kept as one decision so the
// badge and the "can't collect or reply" notice cannot contradict each other
// (issue #1467).

/**
 * Whether a configured Telegram channel is collecting and replying.
 *
 * - `delivering` — a token is stored AND the host reports it is polling. The
 *   green "configured" badge is honest only here.
 * - `stored-not-delivering` — a token is stored but the host explicitly reports
 *   `polling: false`: it has no outbound transport, so it can neither collect
 *   nor reply. This is the state the rebuild notice is for.
 * - `unknown` — a token is stored but the host did not say whether it polls.
 *   `getTelegramChannel` is an unvalidated cast, so a host predating the
 *   `polling` field sends nothing; treating that silence as `false` told the
 *   operator to rebuild their host on the strength of a field that was never
 *   sent. Claim neither delivery nor its absence.
 * - `unconfigured` — no token stored.
 *
 * `configured` alone must never drive the badge: a stored token is not delivery,
 * and `configured && !polling` (which the notice fired on) is satisfiable at the
 * same time as a `configured`-only badge — the two rendered together by
 * construction.
 */
export type TelegramDelivery =
  | "delivering"
  | "stored-not-delivering"
  | "unknown"
  | "unconfigured";

export function telegramDelivery(
  status: { configured: boolean; polling?: boolean } | null | undefined,
): TelegramDelivery {
  if (!status || !status.configured) return "unconfigured";
  // `polling` is typed `boolean`, but the unvalidated wire read makes `undefined`
  // reachable at runtime — hence the explicit true/false checks with an
  // `unknown` fall-through rather than a truthiness test.
  if (status.polling === true) return "delivering";
  if (status.polling === false) return "stored-not-delivering";
  return "unknown";
}
