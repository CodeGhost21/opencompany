// The key-grant result, in transit between the URL it lands on and the card
// that asked for it.
//
// The round trip is a top-level navigation: the console that started the grant
// is gone by the time the browser comes back, so the code arrives on a fresh
// boot with nothing on screen yet that knows what to do with it. `App` captures
// it off the URL before the first render (and strips it from the address bar
// immediately, because it is a live single-use credential), and the card that
// started the flow claims it once it mounts.
//
// A module-level box rather than `sessionStorage`: the value is single-use and
// wanted exactly once, in this document, milliseconds later. Persisting it would
// mean a redeemable code surviving in browser storage — which is the thing the
// address-bar strip exists to prevent, reintroduced one layer down.

/** A key grant that came back and has not been redeemed yet. */
export interface PendingKeyLink {
  state: string;
  code: string;
}

let pending: PendingKeyLink | null = null;
let failed = false;

/** Records the grant this page load landed with. Called once, from `App`. */
export function captureKeyLink(link: PendingKeyLink | null, wasRefused: boolean): void {
  pending = link;
  failed = wasRefused;
}

/**
 * Takes the pending grant, if any.
 *
 * Clears as it reads: the code is single-use, and two cards mounting at once
 * must not both try to spend it — the second would get the host's "expired"
 * refusal and report a failure the operator did not have.
 */
export function takeKeyLink(): PendingKeyLink | null {
  const held = pending;
  pending = null;
  return held;
}

/**
 * Whether the hub refused the grant, or the person chose Cancel on its consent
 * screen. Clears as it reads, for the same reason.
 */
export function takeKeyLinkRefusal(): boolean {
  const held = failed;
  failed = false;
  return held;
}
