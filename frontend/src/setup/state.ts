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
  /**
   * The operator left this flow to wire a model and has not been brought back.
   *
   * The exact opposite of [`skipped`] despite sharing a record: skipping hides
   * the offer, this one owes the operator a re-offer. It is here rather than in
   * a ref because wiring a provider can ask for a restart, and a console
   * reloaded on the Connections page would otherwise lose the debt — which is
   * the whole failure this flag exists to prevent.
   *
   * Losing it anyway (private mode, cleared storage) costs nothing worse than
   * the operator reaching setup through the Company page's prompt, exactly as
   * they did before. It can only ever *offer* something.
   */
  resuming?: boolean;
  /**
   * The operator finished a fallback team, was sent to Settings to wire a
   * model, and is owed a redesign on return.
   *
   * Set by the completion screen's "Add a model in Settings" action. Distinct
   * from [`resuming`] because the two returns differ: a resume reopens the
   * questions over an **unstaffed** company, while a redesign reopens over the
   * standard team the first pass just created — and must replace it rather than
   * stack a second one.
   */
  redesign?: boolean;
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

/**
 * Record "I'll do this later", so the dialog stops opening by itself.
 *
 * Writes the record whole, which drops any pending resume: an operator who went
 * to wire a model and then said "later" has said "later", and honouring the
 * older debt would reopen the dialog they just dismissed.
 */
export function markSetupSkipped(scope: LocalScope): void {
  try {
    localStorage.setItem(KEY(scope), JSON.stringify({ skipped: true, at: Date.now() }));
  } catch {
    /* private mode / quota — setup simply re-offers on the next load */
  }
}

/** Is setup owed a re-offer because the operator left to wire a model? */
export function setupResuming(scope: LocalScope): boolean {
  return Boolean(read(scope).resuming);
}

/**
 * Record that the operator left for model settings mid-setup.
 *
 * Merged onto whatever is stored rather than written whole: a company that had
 * been skipped before can be forced open again from the Team page, and clearing
 * the skip here would silently re-enable the unprompted offer.
 */
export function markSetupResuming(scope: LocalScope): void {
  try {
    const next: SetupState = { ...read(scope), resuming: true, at: Date.now() };
    localStorage.setItem(KEY(scope), JSON.stringify(next));
  } catch {
    /* private mode / quota — the Team page's prompt is still the way back */
  }
}

/** Forget the debt, once it has been paid by reopening the dialog. */
export function clearSetupResuming(scope: LocalScope): void {
  try {
    const { resuming: _dropped, ...rest } = read(scope);
    localStorage.setItem(KEY(scope), JSON.stringify(rest));
  } catch {
    /* nothing to clear */
  }
}

/** Is a redesign owed because the operator left the completion screen to wire a model? */
export function setupRedesign(scope: LocalScope): boolean {
  return Boolean(read(scope).redesign);
}

/**
 * Record that the operator left the completion screen to wire a model and
 * wants the standard team redesigned on their return.
 *
 * Merged rather than written whole, for the same reason as
 * [`markSetupResuming`]: a company that had been skipped before can be forced
 * open again, and clearing the skip here would silently re-enable the
 * unprompted offer.
 */
export function markSetupRedesign(scope: LocalScope): void {
  try {
    const next: SetupState = { ...read(scope), redesign: true, at: Date.now() };
    localStorage.setItem(KEY(scope), JSON.stringify(next));
  } catch {
    /* private mode / quota — the Company page's prompt is still the way back */
  }
}

/** Forget the redesign debt, once it has been paid by reopening the dialog. */
export function clearSetupRedesign(scope: LocalScope): void {
  try {
    const { redesign: _dropped, ...rest } = read(scope);
    localStorage.setItem(KEY(scope), JSON.stringify(rest));
  } catch {
    /* nothing to clear */
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
