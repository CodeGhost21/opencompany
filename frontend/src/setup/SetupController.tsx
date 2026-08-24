import { useCallback, useEffect, useRef, useState } from "react";

import type { OpenCompanyClient } from "@/api/client";
import type { TeamMemberDto } from "@/api/types";
import { useLocalScope } from "@/connections/ConnectionContext";
import { shouldOfferSetup, teamIsUnstaffed } from "@/lib/company-setup";
import { SetupDialog } from "./SetupDialog";
import {
  clearSetupRedesign,
  clearSetupResuming,
  clearSetupSkipped,
  markSetupRedesign,
  markSetupResuming,
  markSetupSkipped,
  setupRedesign,
  setupResuming,
  setupSkipped,
} from "./state";

/** Where "Set up a model" sends the operator. */
const MODEL_SETTINGS = "#/settings/connections";

/** Whether the operator is still on the page they left setup for. */
function onModelSettings(): boolean {
  return window.location.hash.startsWith(MODEL_SETTINGS);
}

/**
 * Decides whether first-run setup opens, and gets out of the way once it has
 * (docs/spec/runtime/company-setup.md).
 *
 * Mounted once inside `AppShell` beside `TourController`, so it overlays every
 * view. The two are sequenced rather than independent: setup runs **first** and
 * the tour waits, because a tour of an unstaffed company walks someone through
 * empty pages — the exact first impression this feature exists to fix. The
 * `onOpenChange` callback is how the shell tells the tour to hold.
 *
 * ## The gate
 *
 * Open when nobody has staffed this company and the operator has not skipped.
 * The test is the host's answer, not a stored flag, so it cannot drift from the
 * thing setup changes — see `shouldOfferSetup` for why a browser flag would be
 * unsafe for this and is fine for the skip.
 *
 * "Staffed" is narrower than "has a roster", and the difference is the whole of
 * issue #1404: the global baseline puts undeletable teammates on **every**
 * company, so an emptiness test never answered `true` anywhere and this dialog
 * could not open in the shipped product. `teamIsUnstaffed` reads the host's
 * per-row provenance instead.
 *
 * A company whose manifest names agents of its own therefore never sees the
 * offer. That is deliberate: it came with a team, so there is nothing to set up.
 * `force` is how the Team page's in-place prompt reopens it anyway.
 */
export function SetupController({
  client,
  company,
  force,
  deepLinked,
  onForceHandled,
  onOpenChange,
  onCompleted,
}: {
  client: OpenCompanyClient;
  company: string | null;
  /** Opened by hand from the Team page's prompt, regardless of the skip flag. */
  force?: boolean;
  /**
   * The operator arrived on a view they named, so do not open unprompted.
   *
   * A blocking dialog is an *offer* when someone lands on the console with
   * nowhere particular to be, and a *hijack* when they deep-linked to
   * `#/workflows/x`. They asked for that page; the Team page's in-place prompt
   * is the affordance a deliberate navigation should meet instead.
   *
   * This does not suppress `unstaffed` reporting — the tour still holds and the
   * Team prompt still shows.
   */
  deepLinked?: boolean;
  /** Clears the caller's force flag once the dialog has taken it. */
  onForceHandled?: () => void;
  /**
   * Fires whenever the tour should hold: while the dialog is open, and while the
   * company still has nobody on it.
   *
   * Emptiness and not just openness, because skipping setup would otherwise pop
   * the tour's welcome straight over an unstaffed company — a walkthrough of
   * empty pages, which is the first impression this whole feature exists to
   * replace. Held until there is a team to show.
   */
  onOpenChange?: (open: boolean) => void;
  /** Setup finished and created a team — the roster reads should refresh. */
  onCompleted?: () => void;
}) {
  const scope = useLocalScope();
  const [open, setOpen] = useState(false);
  /**
   * Whether the gate has been evaluated for this (connection, company).
   *
   * Without it a company switch would leave the previous company's answer in
   * place: the roster read is async, so the dialog would either linger over a
   * staffed company or fail to open over an empty one until the fetch landed.
   */
  const [checked, setChecked] = useState(false);
  /**
   * Whether the host says nobody has staffed this company, independent of the
   * skip flag. The global baseline does not count — see `teamIsUnstaffed`.
   */
  const [unstaffed, setUnstaffed] = useState(false);
  /**
   * Whether the dialog should open in **redesign** mode — the first pass shipped
   * a fallback team, the operator went to wire a model, and the next build-out
   * must replace that team rather than stack a second one on it.
   */
  const [redesigning, setRedesigning] = useState(false);
  /**
   * Whether the gate has already been evaluated once in this mount.
   *
   * Setup opens **unprompted only on the first evaluation**, never again on a
   * later company switch. A switch is navigation, not a first run: an operator
   * who deep-links into `#/workflows/x` on a company that happens to have no
   * team asked for that page, and a blocking modal over it is a hijack rather
   * than an offer. The Team page's in-place prompt still covers those companies,
   * which is the affordance a deliberate navigation should meet.
   *
   * A ref, not state: it must be true before the next evaluation's render, and
   * it must not itself trigger one.
   */
  const evaluatedOnce = useRef(false);

  // Report only once the roster read has landed.
  //
  // Reporting on mount would say "nothing to hold for" before we know, and the
  // tour would open its welcome in the gap — two dialogs stacked on the first
  // screen an operator ever sees. The shell therefore starts held and waits for
  // this, so the quiet case is a tour that opens a beat late rather than one
  // that flashes over setup.
  useEffect(() => {
    if (!checked) return;
    onOpenChange?.(open || unstaffed);
  }, [checked, open, unstaffed, onOpenChange]);

  // Re-evaluate the gate whenever the addressed company changes.
  useEffect(() => {
    let cancelled = false;
    setChecked(false);
    setOpen(false);

    (async () => {
      let roster: TeamMemberDto[] = [];
      try {
        roster = await client.listTeam(company);
      } catch {
        // A host with no roster surface, or one we cannot reach. Offer nothing:
        // a setup flow that cannot read the team cannot tell a fresh company
        // from a staffed one, and guessing risks a duplicate team.
        if (!cancelled) setChecked(true);
        return;
      }
      if (cancelled) return;
      const first = !evaluatedOnce.current;
      evaluatedOnce.current = true;
      const empty = teamIsUnstaffed(roster);
      setUnstaffed(empty);
      // A console reloaded after wiring a provider — a restart is a thing that
      // page asks for — lands here rather than on a `hashchange`, so both debts
      // are honoured on this path too. Not while still *on* that page: the
      // operator has not come back yet, and the listener below is what notices
      // when they do.
      const wasResuming = setupResuming(scope);
      const wasRedesigning = setupRedesign(scope);
      const returned = (wasResuming || wasRedesigning) && !onModelSettings();
      // Dropped on return whatever we do with it, so a debt cannot outlive the
      // trip that created it and resurface over some later unrelated
      // navigation. `empty` decides only whether a *resume* is worth acting on;
      // a redesign reopens over the staffed company the first pass created.
      if (wasResuming) clearSetupResuming(scope);
      if (wasRedesigning) clearSetupRedesign(scope);
      const resume = returned && (wasRedesigning || empty);
      setRedesigning(wasRedesigning && returned);
      // Only the first evaluation may open the dialog by itself; see
      // `evaluatedOnce`. Later switches still report `unstaffed`, so the tour
      // keeps holding and the Company page keeps prompting.
      setOpen(
        resume ||
          (first && !deepLinked && shouldOfferSetup({ roster, skipped: setupSkipped(scope) })),
      );
      setChecked(true);
    })();

    return () => {
      cancelled = true;
    };
  }, [client, company, scope, deepLinked]);

  // Bring the operator back when they return from wiring a model.
  //
  // The navigation away is a hash change and so is the navigation back, and this
  // controller sees neither through its own props — hence a listener rather than
  // an effect keyed on the route. The roster is re-read on arrival rather than
  // trusted from state captured before the navigation: another tab or colleague
  // may have staffed the company in the meantime, and a return must never open
  // setup over a team that already exists. The debt is dropped either way so it
  // cannot resurface on some later unrelated navigation.
  useEffect(() => {
    const arrive = () => {
      if (onModelSettings()) return;
      const wasResuming = setupResuming(scope);
      const wasRedesigning = setupRedesign(scope);
      if (!wasResuming && !wasRedesigning) return;
      if (wasResuming) clearSetupResuming(scope);
      if (wasRedesigning) clearSetupRedesign(scope);
      void client
        .listTeam(company)
        .then((roster) => {
          const empty = teamIsUnstaffed(roster);
          setUnstaffed(empty);
          if (wasRedesigning) {
            // The first pass shipped a fallback team and the operator went to
            // wire a model. Reopen in redesign mode so the next build-out
            // replaces that team instead of stacking a second one.
            setRedesigning(true);
            setOpen(true);
          } else if (empty) {
            setOpen(true);
          }
        })
        .catch(() => {
          // Cannot confirm what the roster looks like; opening setup over an
          // unknown team risks a duplicate, so stay closed.
        });
    };
    window.addEventListener("hashchange", arrive);
    return () => window.removeEventListener("hashchange", arrive);
  }, [scope, company, client]);

  // The Team page's prompt reopens setup after a skip.
  useEffect(() => {
    if (!force) return;
    setOpen(true);
    onForceHandled?.();
  }, [force, onForceHandled]);

  const skip = useCallback(() => {
    markSetupSkipped(scope);
    setOpen(false);
    setRedesigning(false);
  }, [scope]);

  /**
   * Close for a navigation that is *part of* setup, recording nothing.
   *
   * Following "Set up a model" is the operator starting this flow, not
   * declining it. Routing that through `skip` persisted an "I'll do this later"
   * they never expressed, so on return the company was still unstaffed and the
   * dialog no longer offered itself.
   *
   * Not recording the skip is only half of it, though, and the other half is
   * why this records something of its own. This controller stays mounted across
   * hash changes, its gate re-evaluates only on `(client, company, scope,
   * deepLinked)`, and `evaluatedOnce` bars a second unprompted open — so on the
   * operator's return nothing would reopen the dialog either way, and the flow
   * they had just gone to enable would still be reachable only through the Team
   * page's separate prompt. `markSetupResuming` is the debt; the effect below
   * pays it.
   */
  const leave = useCallback(() => {
    markSetupResuming(scope);
    setOpen(false);
    setRedesigning(false);
  }, [scope]);

  const done = useCallback(() => {
    // Clear the skip so it cannot outlive what it was suppressing: an operator
    // who skipped, later ran setup, then removed every agent should be offered
    // setup again rather than left on an empty company page.
    clearSetupSkipped(scope);
    setOpen(false);
    // The team exists now, so the tour has something to walk through.
    setUnstaffed(false);
    setRedesigning(false);
    onCompleted?.();
  }, [scope, onCompleted]);

  /**
   * The completion screen's "Add a model in Settings" action.
   *
   * Distinct from [`done`] even though both close the dialog: this one leaves
   * the fallback team in place and records a redesign debt, so when the
   * operator returns from wiring a model the dialog reopens in redesign mode
   * and that team is replaced rather than a second one stacked on it.
   */
  const redesign = useCallback(() => {
    markSetupRedesign(scope);
    setOpen(false);
  }, [scope]);

  if (!checked && !force) return null;
  if (!open) return null;

  return (
    <SetupDialog
      open={open}
      client={client}
      company={company}
      onSkip={skip}
      onLeave={leave}
      onDone={done}
    />
  );
}
