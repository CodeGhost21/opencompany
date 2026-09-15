// The Composio half of the account-key reuse banner (keys rework, issue
// #2306, slice 4c; docs/key-reworks/phase-4c-reuse-banner.md §3.4). PURE — no
// React, no fetch — mirroring `@/composio/in-use`'s own reasoning: this
// project has no component-test harness, so anything worth a test has to be a
// function a component's state derives from, not markup a render would
// exercise. `ComposioSection.tsx` calls these; it does not reimplement the
// decision inline.
//
// The LLM page gets the analogous `showsInferenceReuseBanner` in its own
// module (out of scope here — a separate dispatch owns `frontend/src/inference/**`).
// The two are intentionally not unified into one shared function: the LLM
// banner's visibility also depends on `defaultChoice`/agent-pair references
// that have no Composio analogue (Composio has no default/agent-pair concept
// at all — `docs/key-reworks/in-use-guards.md` §1), so a single shared
// predicate would need a parameter neither caller could give an honest
// default for.

import type { ComposioMode } from "@/api/composio";

/**
 * Whether the Composio page should offer "use the same key for Composio?".
 *
 * `canManage` gates it exactly as every other Composio write is gated
 * (members never see it; the host would refuse the write anyway). The other
 * four conditions:
 *
 * - `accountConfigured` — the company has an account key at all
 *   (`GET …/credential`'s existing `configured` field). Nothing to reuse
 *   otherwise.
 * - `mode === "managed"` — the banner only ever offers to fill the *managed*
 *   TinyHumans slot; a BYOK company's own Composio account has nothing to do
 *   with the account key, and offering this there would read as though it
 *   could switch the route, which it cannot ({@link copyAccountKeyToComposio}
 *   is a single-slot copy, never a mode switch).
 * - `managedCredentialSource !== "static"` — `static` means the managed slot
 *   already has its own key (a token pasted directly, or a static instance
 *   key): nothing is missing, so there is nothing to offer. Every other
 *   tier — `none` (nothing resolves) and `company`/`attested`
 *   (already resolving through *something else*, i.e. still not this
 *   company's own pasted managed token) — is a state where filling the
 *   managed slot from the account key is a real, useful action.
 * - `!dismissed` — the operator already said "not now" this session/company.
 */
export function showsComposioReuseBanner(a: {
  canManage: boolean;
  accountConfigured: boolean;
  mode: ComposioMode | undefined;
  managedCredentialSource: string | undefined;
  dismissed: boolean;
}): boolean {
  return (
    a.canManage &&
    a.accountConfigured &&
    a.mode === "managed" &&
    a.managedCredentialSource !== "static" &&
    !a.dismissed
  );
}

/**
 * The `localStorage` key a "Not now" dismissal is recorded under, namespaced
 * per page and per company so dismissing the Composio banner for one company
 * never hides the (separate, out-of-scope-here) LLM banner or another
 * company's Composio banner.
 *
 * Only `"composio"` is a valid `page` from this module — the LLM half of this
 * key shape belongs to `frontend/src/inference/reuse-banner.ts`, a different
 * dispatch's file. The `page` parameter exists so the two halves agree on the
 * key shape without importing from each other.
 */
export function reuseDismissKey(
  page: "composio",
  company: string | null,
): string {
  return `oc.reuse-account-key.${page}.${company ?? "_"}`;
}

/**
 * Whether `key` was dismissed. `false` on any read failure — Safari private
 * mode and a "block all cookies" setting both make `localStorage` itself
 * throw rather than return a dud object, and a banner that cannot remember a
 * dismissal must still render correctly (shown, not stuck hidden).
 */
export function readDismissed(key: string): boolean {
  try {
    return window.localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

/**
 * Record a dismissal. A failure to write is not worth failing a render over
 * — the banner simply reappears next time, which is the safe direction.
 */
export function writeDismissed(key: string): void {
  try {
    window.localStorage.setItem(key, "1");
  } catch {
    // A full, blocked, or read-only store is not worth surfacing here.
  }
}
