// First-run setup's one piece of browser-local state: did this operator say
// "I'll do this later"?
//
// Keyed per (connection, company) exactly like `tour/state.ts`, so two hosts
// serving a company of the same name never share one operator's decision.
//
// ## Why a browser flag is safe here and would not be for "has setup run"
//
// `tour/state.ts` explains that first-run state lives in `localStorage` because
// `UserRecord` carries no per-user field. For the tour that is a small cost:
// cleared storage re-offers a walkthrough.
//
// Setup *creates things*, so the same trade would be unacceptable for the
// question "has setup already run?" — cleared storage would build a second team
// on top of the first. That question is therefore answered by the host instead:
// `shouldOfferSetup` asks whether the roster is empty (see
// `lib/company-setup.ts`).
//
// What lives here is only the *skip*, and skipping can do exactly one thing:
// hide an offer. Losing it re-offers setup to a company that still has nobody on
// it, which is the correct outcome anyway. So the fragile store holds the
// harmless half, and the durable store holds the half that matters.

import { type LocalScope, scopedKey } from "@/connections/types";

const KEY = (scope: LocalScope): string => scopedKey("oc-setup", scope);

interface SetupState {
  skipped?: boolean;
  at?: number;
}

function read(scope: LocalScope): SetupState {
  try {
    const raw = localStorage.getItem(KEY(scope));
    return raw ? (JSON.parse(raw) as SetupState) : {};
  } catch {
    return {};
  }
}

/** Has this operator dismissed the setup offer for this company? */
export function setupSkipped(scope: LocalScope): boolean {
  return Boolean(read(scope).skipped);
}

/** Record "I'll do this later", so the dialog stops opening by itself. */
export function markSetupSkipped(scope: LocalScope): void {
  try {
    localStorage.setItem(KEY(scope), JSON.stringify({ skipped: true, at: Date.now() }));
  } catch {
    /* private mode / quota — setup simply re-offers on the next load */
  }
}

/**
 * Forget the skip.
 *
 * Called when setup completes, so the flag cannot outlive the thing it was
 * suppressing: an operator who skips, later runs setup, and then removes every
 * agent should be offered setup again rather than silently left on an empty
 * team page.
 */
export function clearSetupSkipped(scope: LocalScope): void {
  try {
    localStorage.removeItem(KEY(scope));
  } catch {
    /* nothing to clear */
  }
}

// ---------------------------------------------------------------------------
// The sign-in hand-off marker
// ---------------------------------------------------------------------------

/**
 * The hash-query key a setup hand-off link carries, so a sign-in that
 * navigates the whole document away (setup's button sets `window.location.href`)
 * still lands knowing setup just finished: `…code=…#/company?from=setup`.
 *
 * `useHashView`'s segment parsing strips everything from `?` onward, so the
 * flag never reaches the router; AppShell consumes it on the landing mount to
 * apply the same welcome suppression a same-mount completion gets, then removes
 * it so a reload or a copied link cannot re-apply it.
 */
export const SETUP_HANDOFF_FLAG = "from";

/**
 * The landing fragment a setup hand-off link carries. The wizard hands this to
 * the host so a *mailed* link lands the same way the loopback link does.
 */
export const SETUP_HANDOFF_FRAGMENT = `#/company?${SETUP_HANDOFF_FLAG}=setup`;

/** A fragment marker scoped to one connection and company. */
export function setupHandoffFragment(scope: SetupHandoffScope): string {
  const company = scope.company ?? "single";
  return `#/company?${SETUP_HANDOFF_FLAG}=setup&connection=${encodeURIComponent(scope.connection)}&company=${encodeURIComponent(company)}`;
}

export interface SetupHandoffScope {
  connection: string;
  company: string | null;
}

/** Whether the current address arrived from setup's sign-in hand-off. */
export function arrivedViaSetupHandoff(scope?: SetupHandoffScope): boolean {
  const [, query = ""] = window.location.hash.split("?");
  const params = new URLSearchParams(query);
  if (params.get(SETUP_HANDOFF_FLAG) !== "setup") return false;
  if (!scope) return true;
  return (
    params.get("connection") === scope.connection &&
    params.get("company") === (scope.company ?? "single")
  );
}

/**
 * Whether the hand-off marker is scoped to a connection and company.
 *
 * `setupHandoffFragment` encodes the scope so a marker addressed to one company
 * cannot be consumed by another; the setup wizard and magic-link flow leave it
 * out because their scope may not survive the full-page hand-off. Telling the
 * two apart is what lets AppShell accept the unscoped form on whatever company
 * it lands on while still refusing a marker scoped somewhere else.
 */
export function setupHandoffHasScope(): boolean {
  const [, query = ""] = window.location.hash.split("?");
  const params = new URLSearchParams(query);
  return params.has("connection") || params.has("company");
}

/**
 * Whether the current address rode in on a hub sign-in that was asked to land
 * on setup's destination.
 *
 * The host puts the destination on the hub's return URI as a *query* parameter
 * (`?company=…&from=setup`), because a fragment cannot cross the OAuth round
 * trip — the hub appends its own `token=` to whatever it was given, and
 * anything after a `#` there would swallow it.
 */
export function arrivedViaHubSetupHandoff(scope?: SetupHandoffScope): boolean {
  const params = new URLSearchParams(window.location.search);
  if (params.get(SETUP_HANDOFF_FLAG) !== "setup") return false;
  if (!scope) return true;
  return (
    params.get("connection") === scope.connection &&
    params.get("company") === (scope.company ?? "single")
  );
}

/**
 * Consumes a hub-carried setup destination, translating it into the same
 * one-shot hash marker a setup hand-off link carries.
 *
 * An ecosystem sign-in returns to `/?company=…&from=setup`; the token
 * redemption strips the hub's own params but leaves `from`. This reads it,
 * takes it out of the query, and writes `#/company?from=setup` — the exact
 * landing a setup link would have produced — so the shell applies the same
 * welcome suppression and route, then clears the marker like any other
 * hand-off. A reload after the conversion has neither the query flag nor the
 * hash marker, so it cannot re-apply either.
 */
export function absorbHubSetupHandoff(scope?: SetupHandoffScope): void {
  if (!arrivedViaHubSetupHandoff(scope)) return;
  const params = new URLSearchParams(window.location.search);
  params.delete(SETUP_HANDOFF_FLAG);
  const qs = params.toString();
  window.history.replaceState(
    {},
    "",
    window.location.pathname + (qs ? `?${qs}` : "") + window.location.hash,
  );
  window.location.hash = scope ? setupHandoffFragment(scope) : SETUP_HANDOFF_FRAGMENT;
}

/**
 * Removes the hand-off flag from the address.
 *
 * One-shot: the suppression it enables belongs to the arrival it rode in on,
 * not to a later reload. Other hash-query keys (`?host=`, for instance) are
 * preserved.
 */
export function clearSetupHandoff(): void {
  const [path, query = ""] = window.location.hash.replace(/^#/, "").split("?");
  const params = new URLSearchParams(query);
  if (!params.has(SETUP_HANDOFF_FLAG)) return;
  params.delete(SETUP_HANDOFF_FLAG);
  const qs = params.toString().replace(/=(?=&|$)/g, "");
  const next = `#${path}${qs ? `?${qs}` : ""}`;
  if (next !== window.location.hash) window.history.replaceState(null, "", next);
}
