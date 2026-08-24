import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, RotateCcw, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import {
  getPolicy,
  type PolicyStatus,
  resetPolicy,
  setPolicy,
} from "@/api/policy";
import { listWorkflowToolSlugs } from "@/api/workflows";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

/**
 * Tools worth naming as an *example* of something to always ask about, most
 * consequential first (issue #1226).
 *
 * A placeholder is a suggestion, so it should suggest a gate an operator might
 * actually want. Taking the host's list in its own order put
 * `read_workspace_state` — a read — in the worked example, which is a valid
 * entry and a pointless one. This orders the candidates; the datalist under the
 * field still carries the deployment's full set in the host's order, because
 * that is a lookup rather than a recommendation.
 *
 * Not a validator and not a filter: anything wired here is offered, and a tool
 * absent from this list simply sorts after the ones on it.
 */
const WORTH_GATING = [
  "publish_artifact",
  "shell",
  "http_request",
  "curl",
  "git_operations",
  "apply_patch",
  "web_fetch",
];

/**
 * Up to three worked examples, drawn from what this deployment wired.
 *
 * Falls back to real tool names when the host served nothing — a host predating
 * `…/workflows/tool-slugs` still deserves an example that would work, and the
 * one this field used to give (`payment.send, filing.submit, external.publish`)
 * is the one issue #684 deleted for gating nothing.
 */
export function alwaysAskPlaceholder(wired: string[]): string {
  if (wired.length === 0) return "shell, http_request, publish_artifact";
  const rank = (slug: string) => {
    const at = WORTH_GATING.indexOf(slug);
    return at === -1 ? WORTH_GATING.length : at;
  };
  return [...wired]
    .sort((a, b) => rank(a) - rank(b) || wired.indexOf(a) - wired.indexOf(b))
    .slice(0, 3)
    .join(", ");
}

/**
 * Whether moving through the host-provided tier order gives agents more
 * autonomy. `from`/`to` are host tier values; an unknown value is never "from"
 * (nothing is known about a move it starts from) and never "to" (there is no
 * ordering to move to).
 */
export function widensAutonomy(
  tiers: PolicyStatus["tiers"],
  from: string,
  to: string,
): boolean {
  const fromIndex = tiers.findIndex((tier) => tier.value === from);
  const toIndex = tiers.findIndex((tier) => tier.value === to);
  return fromIndex !== -1 && toIndex > fromIndex;
}

/**
 * Whether a tier change gives the company more freedom than it has now.
 *
 * Same order comparison as [`widensAutonomy`]; kept under the pre-#1423 name
 * because the always-ask vocabulary test pins it that way.
 */
export function isAutonomyEscalation(
  tiers: PolicyStatus["tiers"],
  currentMode: string,
  nextMode: string,
): boolean {
  return widensAutonomy(tiers, currentMode, nextMode);
}

/**
 * Whether an `always_approve` entry gates a target under the backend's matcher
 * (`src/policy/always_approve.rs`).
 *
 * The matcher accepts more than an exact tool name: the comparison is
 * ASCII-case-insensitive (a full-Unicode fold would accept a case confusable
 * the host's `eq_ignore_ascii_case` does not — `worKspace_write` lowercases
 * to `workspace_write` but never gates), and a leading dotted segment gates
 * the rest, so `SHELL` is the wired `shell` tool and `invoice` covers
 * `invoice.send`. The "is not a tool" warning under the field must not
 * contradict the gate it describes — an entry the backend would match is a
 * valid fence, not a mistake — so the same two rules decide whether an entry
 * counts as known.
 */
export function alwaysApproveGates(entry: string, target: string): boolean {
  const e = entry.trim();
  const t = target.trim();
  if (e === "") return false;
  if (asciiEqualsIgnoreCase(t, e)) return true;
  // Leading dotted segment: `invoice` gates `invoice.send`, but a bare prefix
  // (`pay` for `payroll.export`) does not — the segment boundary is load
  // bearing, exactly as it is in the backend.
  return (
    t.length > e.length &&
    t[e.length] === "." &&
    asciiEqualsIgnoreCase(t.slice(0, e.length), e)
  );
}

/**
 * ASCII-only case-insensitive equality, mirroring `str::eq_ignore_ascii_case`.
 *
 * `String.prototype.toLowerCase()` is NOT the same comparison: it folds
 * Unicode case, so `"Ä".toLowerCase() === "ä"` while the host treats the two
 * as different effect kinds. The confirmation must agree with the gate itself,
 * so only ASCII letters fold here and every other code unit must match exactly.
 */
function asciiEqualsIgnoreCase(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const ca = a.charCodeAt(i);
    const cb = b.charCodeAt(i);
    if (ca === cb) continue;
    // Folding an ASCII letter is OR-ing in bit 0x20. Anything that does not
    // land in 'a'..'z' after the fold is not an ASCII letter, so it cannot be
    // a case pair.
    const lowerA = ca | 0x20;
    const lowerB = cb | 0x20;
    if (lowerA !== lowerB || lowerA < 0x61 || lowerA > 0x7a) return false;
  }
  return true;
}

/**
 * Whether `list` still gates `target`, mirroring the host matcher
 * (`src/policy/always_approve.rs::matches`): exact or a leading dotted segment,
 * ASCII-case-insensitive, on a segment boundary.
 *
 * A reset drops the whole override, always-ask list included, so an effective
 * entry the manifest's list does not gate is a fence a reset would silently
 * take down. This is the "would the reset let something through that used to
 * ask" test, and it must agree with the gate itself or the confirmation would
 * contradict the behaviour it describes.
 */
export function gatedBy(list: string[], target: string): boolean {
  const t = target.trim();
  return list.some((entry) => {
    const e = entry.trim();
    if (e === "") return false;
    if (asciiEqualsIgnoreCase(t, e)) return true;
    return (
      t.length > e.length &&
      t[e.length] === "." &&
      asciiEqualsIgnoreCase(t.slice(0, e.length), e)
    );
  });
}

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

/**
 * The autonomy tier and the always-ask list (issue #562).
 *
 * An operator drowning in approval cards previously had no way to stop it: the
 * tier lives in the company manifest, and nothing in the console read or wrote
 * it — so changing it meant editing a version-controlled file and redeploying,
 * or on a hosted tenant (where the manifest is a read-only boot snapshot) it
 * meant nothing at all.
 *
 * Two things this deliberately renders rather than hides:
 *
 * - **The tiers are described by consequence, not by name.** "Supervised" and
 *   "full" mean nothing to someone deciding between them; "asks before every
 *   change, including its own scratch files" does. The prose comes from the
 *   host, because it describes what that host's approval gate actually does.
 * - **When a change bites.** A tier change lands on the company's *next* turn,
 *   so a turn already running finishes under the old one. Since stopping the
 *   flood *now* is exactly why an operator is here, that gap is stated instead
 *   of being left to discover.
 * - **That version control outranks it.** The override is durable between seed
 *   edits, but editing `[policy]` in `company.toml` clears it. An operator who
 *   cannot see that would be surprised by a redeploy.
 */
export function PolicySettings({ client, company }: Props) {
  const [status, setStatus] = useState<PolicyStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  // Distinguishes "still loading" from "load finished and failed". Without it,
  // `loading || !status` renders the spinner forever on a failed load and the
  // operator has no way to retry.
  const [loadError, setLoadError] = useState<string | null>(null);
  // The always-ask list is edited as text and only committed on Save, so a
  // half-typed effect kind never reaches the gate.
  const [draftAlways, setDraftAlways] = useState("");
  const [dirty, setDirty] = useState(false);
  // A looser tier changes what teammates can do without stopping for approval.
  // Keep the target, rather than a boolean, so the dialog can compare the
  // host-provided consequences that actually apply to this deployment.
  const [tierAwaitingConfirmation, setTierAwaitingConfirmation] =
    useState<PolicyStatus["tiers"][number] | null>(null);
  // A reset restores the manifest's tier AND always-ask list, so the widening
  // check must run on it too — otherwise "Use the manifest's policy" is a
  // one-click way around the confirmation the tier buttons get, and the same
  // for always-ask gates the manifest does not carry. Kept separate from the
  // tier state so the dialog knows which action to perform on confirm.
  const [resetAwaitingConfirmation, setResetAwaitingConfirmation] =
    useState(false);
  /**
   * The tool names this deployment can actually gate (issue #1226).
   *
   * An `always_approve` entry IS a tool name on the harness path — see
   * `src/policy/always_approve.rs`, which explains that the two were never
   * separate namespaces. So the honest set of worked examples is the set of
   * tools wired here, and this is the same read the workflow copilot grounds on
   * (issues #783 / #874) for the same reason: so nothing suggests a tool this
   * deployment does not have.
   *
   * Empty on a host predating the route, which degrades to the plain field the
   * operator had before — the suggestions are help, never a constraint. The
   * namespace stays open on purpose (a hosted brain may emit a kind this
   * repository has never seen), so nothing here validates what is typed.
   */
  const [wiredTools, setWiredTools] = useState<string[]>([]);
  // Whether the wired-tool set above was actually served. The array starts
  // empty while the request is pending and stays empty on a host predating the
  // route; an empty set is not proof that every configured entry is unwired, so
  // only a successful load lets the "is not a tool" warning speak.
  const [wiredToolsLoaded, setWiredToolsLoaded] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const next = await getPolicy(client, company);
      setStatus(next);
      setDraftAlways(next.alwaysApprove.join(", "));
      setDirty(false);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not load the policy.";
      setLoadError(message);
      toast.error(message);
    } finally {
      setLoading(false);
    }
  }, [client, company]);

  useEffect(() => {
    void load();
  }, [load]);

  // The confirmation dialog holds a choice reviewed against ONE company's
  // policy. If the scope changes while it is open, that pending action no
  // longer describes what the operator looked at — confirming would loosen or
  // reset the NEW company under a dialog about the old one. Drop it on scope
  // change rather than bind it to the originating company.
  useEffect(() => {
    setTierAwaitingConfirmation(null);
    setResetAwaitingConfirmation(false);
  }, [client, company]);

  // Deliberately silent about its own failure, and deliberately not part of
  // `load`: these are suggestions under a free-text box. A host that cannot
  // serve them costs the operator a datalist, not the setting, and a second
  // error banner would report the policy card as broken when it is merely
  // plainer — the same reasoning `LedgersView.refreshTasks` gives.
  useEffect(() => {
    let live = true;
    setWiredTools([]);
    setWiredToolsLoaded(false);
    void listWorkflowToolSlugs(client, company)
      .then((r) => {
        if (live) {
          setWiredTools(r.slugs);
          setWiredToolsLoaded(true);
        }
      })
      .catch(() => {
        if (live) setWiredTools([]);
      });
    return () => {
      live = false;
    };
  }, [client, company]);

  /**
   * Applies a server response.
   *
   * `resyncDraft` is false when the operator has unsaved always-ask edits: the
   * server's list is authoritative for what the gate is enforcing, but
   * overwriting the box would silently discard what they were part-way through
   * typing. The tier request does not touch the list, so leaving the draft
   * alone keeps the two independent — the same separation the `PUT` body has.
   */
  const apply = (next: PolicyStatus, message: string, resyncDraft = true) => {
    setStatus(next);
    if (resyncDraft) {
      setDraftAlways(next.alwaysApprove.join(", "));
      setDirty(false);
    }
    toast.success(message, { description: next.takesEffect });
  };

  const saveTier = async (mode: string) => {
    if (!status || saving || mode === status.mode) return false;
    setSaving(true);
    try {
      // Only `mode` is sent: an omitted field leaves the always-ask list where
      // it is, so picking a tier cannot silently discard a list the operator
      // edited earlier.
      // `dirty` means the operator has unsaved list edits; keep them.
      apply(
        await setPolicy(client, company, { mode }),
        "Autonomy tier updated",
        !dirty,
      );
      return true;
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not change the tier.",
      );
      return false;
    } finally {
      setSaving(false);
    }
  };

  const chooseTier = (tier: PolicyStatus["tiers"][number]) => {
    if (!status || saving || tier.value === status.mode) return;
    if (widensAutonomy(status.tiers, status.mode, tier.value)) {
      confirmSource.current = "tier";
      setTierAwaitingConfirmation(tier);
      return;
    }
    void saveTier(tier.value);
  };

  // Only a successfully loaded tool set may flag an entry: while the request is
  // pending, and on hosts predating the route, the empty array is "unknown", not
  // "none of these are wired".
  //
  // The best set to compare against is the policy response's `knownTools` — the
  // complete gateable registry, which is broader than the workflow tool set
  // served by `/workflows/tool-slugs`: an agent may be wired a tool that cannot
  // be a workflow node (`hosting_launch_site`, `publish_artifact`), and the
  // gate matches it by name. Comparing against the workflow subset alone would
  // call such a fence a mistake. So when the host serves the complete registry
  // the note is confident; a host predating it falls back to the workflow set,
  // and the note scopes itself to what that set can prove.
  //
  // An entry counts as matching when it would gate a known tool under the
  // backend's own matcher (`SHELL` for the `shell` tool, `invoice` for a
  // `invoice.send` kind), so a fence the gate accepts is never called a mistake
  // outright.
  const knownTools = status?.knownTools ?? null;
  const gateableSet = knownTools ?? (wiredToolsLoaded ? wiredTools : null);
  const unmatchedWiredTools = gateableSet
    ? draftAlways
        .split(",")
        .map((kind) => kind.trim())
        .filter(
          (kind) =>
            kind && !gateableSet.some((tool) => alwaysApproveGates(kind, tool)),
        )
    : [];

  /**
   * The note's wording, scoped to what `gateableSet` can prove. With the
   * complete registry the claim is confident — no tool the gate recognizes —
   * and the hedge only needs the open effect namespace. On a host predating
   * the field, the note names the workflow set it actually compared against
   * and hedges that a wired agent tool outside it may still exist.
   */
  const unmatchedNote = unmatchedWiredTools.length
    ? `${unmatchedWiredTools.join(", ")} ${
        unmatchedWiredTools.length === 1 ? "doesn't" : "don't"
      } match any ${
        knownTools
          ? "tool the approval gate recognizes"
          : "of the workflow tools wired here"
      }. ${
        unmatchedWiredTools.length === 1 ? "It may" : "They may"
      } still be ${
        knownTools
          ? unmatchedWiredTools.length === 1
            ? "a hosted effect kind"
            : "hosted effect kinds"
          : unmatchedWiredTools.length === 1
            ? "a wired agent tool or a hosted effect kind"
            : "wired agent tools or hosted effect kinds"
      }.`
    : null;

  const saveAlways = async () => {
    if (!status || saving) return;
    setSaving(true);
    try {
      // An empty box means an empty list, not "leave it alone" — the host keeps
      // those apart and so must this.
      const kinds = draftAlways
        .split(",")
        .map((kind) => kind.trim())
        .filter(Boolean);
      apply(
        await setPolicy(client, company, { alwaysApprove: kinds }),
        "Always-ask list updated",
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not save the list.",
      );
    } finally {
      setSaving(false);
    }
  };

  /**
   * The tier buttons, in `status.tiers` order, so the radio group's arrow keys
   * can move focus between them (a roving-tabindex group: only the checked tier
   * is in the Tab order, and arrows move and select in one step).
   */
  const tierButtons = useRef<Array<HTMLButtonElement | null>>([]);
  /**
   * Which control launched the confirmation dialog, so closing it can return
   * focus somewhere sensible. A tier escalation is opened from the radio the
   * operator pressed — which may not be the tier that ends up selected — so
   * closing re-syncs focus to the checked tier; the reset flow's trigger is a
   * plain button whose own focus restore is right. A ref, not state, because
   * the dialog's close handler reads it after `onOpenChange` has cleared the
   * confirmation state.
   */
  const confirmSource = useRef<"tier" | "reset">("tier");
  /**
   * The "Use the manifest's policy" button, so a cancelled reset-driven
   * confirmation can return focus to it (the controlled `AlertDialog` has no
   * trigger of its own for Base UI to restore).
   */
  const resetButtonRef = useRef<HTMLButtonElement | null>(null);

  const reset = async () => {
    if (!status || saving) return false;
    setSaving(true);
    try {
      apply(
        await resetPolicy(client, company),
        "Reverted to the manifest's policy",
      );
      return true;
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not reset the policy.",
      );
      return false;
    } finally {
      setSaving(false);
    }
  };

  // Always-ask gates an operator added that a reset would drop — entries the
  // manifest's list does not gate. The tier-widening test misses these: the
  // tiers can agree while the lists disagree, and restoring the manifest then
  // still widens what gets through, so it earns the same confirmation and the
  // dialog names it.
  const removedAlwaysAsk =
    status?.alwaysApprove.filter(
      (entry) => !gatedBy(status.manifestAlwaysApprove, entry),
    ) ?? [];

  /**
   * The "Use the manifest's policy" button. A reset that gives the company
   * *more* autonomy than the override it replaces is an escalation like any
   * other tier change, so it gets the same confirmation; so does a reset that
   * drops always-ask gates the manifest does not carry. A reset that tightens
   * or holds the tier lands immediately, the way a downgrade does.
   */
  const requestReset = () => {
    if (!status || saving) return;
    // The manifest's tier can be MORE autonomous than the override an operator
    // set — resetting would restore that looser tier, so it earns the same
    // widening confirmation as picking the tier directly. So does dropping
    // always-ask gates the manifest does not carry: a reset removes the whole
    // override, and an effective entry the manifest list does not gate is a
    // fence that silently comes down even when the tiers agree.
    const manifestTier = status.tiers.find(
      (tier) => tier.value === status.manifestMode,
    );
    if (
      manifestTier &&
      (widensAutonomy(status.tiers, status.mode, status.manifestMode) ||
        removedAlwaysAsk.length > 0)
    ) {
      confirmSource.current = "reset";
      setResetAwaitingConfirmation(true);
      return;
    }
    void reset();
  };

  /**
   * Radio-group arrow keys: move focus to the neighbour and select it in the
   * same step, the way native radios behave. Without this, every tier stays in
   * the Tab order and no Arrow key moves between them — a screen reader
   * announces radio-group controls whose keyboard behavior does not exist.
   */
  const handleTierKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!status || saving) return;
    let step = 0;
    switch (event.key) {
      case "ArrowDown":
      case "ArrowRight":
        step = 1;
        break;
      case "ArrowUp":
      case "ArrowLeft":
        step = -1;
        break;
      default:
        return;
    }
    // Navigate from the radio that has focus, not the tier that happens to be
    // selected — the two can differ. Pressing ArrowRight on Auto focuses Full
    // and, because that is an escalation, parks the choice in a confirmation
    // dialog; when the operator cancels, focus is back on Full while Auto is
    // still selected, and the next arrow must compute from Full or it skips a
    // tier. The keydown bubbles from the focused button to this container, so
    // `event.target` is that button.
    const focused = tierButtons.current.indexOf(
      event.target as HTMLButtonElement,
    );
    if (focused === -1) return;
    // Wrap at both ends, like a radio group: ArrowUp on the first tier lands
    // on the last and ArrowDown on the last lands on the first. A bare
    // `focused + step` bounds check would dead-end the group at its edges
    // instead of looping it.
    const next = (focused + step + status.tiers.length) % status.tiers.length;
    const tier = status.tiers[next];
    if (!tier) return;
    event.preventDefault();
    tierButtons.current[next]?.focus();
    chooseTier(tier);
  };

  const manifestTier = status?.tiers.find(
    (tier) => tier.value === status.manifestMode,
  );

  return (
    <Card data-testid="policy-settings">
      <CardHeader>
        <CardTitle id="approvals-heading" className="flex items-center gap-2 text-base">
          <ShieldCheck className="h-4 w-4" />
          Approvals
        </CardTitle>
        <CardDescription>
          How much the teammates do on their own, and what they always ask about
          first.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        {loading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading the current policy…
          </div>
        ) : !status ? (
          <div className="space-y-3">
            <p className="text-sm text-muted-foreground">
              {loadError ?? "Could not load the policy."}
            </p>
            <Button size="sm" variant="outline" onClick={() => void load()}>
              Try again
            </Button>
          </div>
        ) : (
          <>
            <div
              className="space-y-2"
              role="radiogroup"
              aria-labelledby="approvals-heading"
              onKeyDown={handleTierKeyDown}
            >
              <div className="flex justify-between px-1 text-xs text-muted-foreground">
                <span>More oversight</span>
                <span>More autonomy</span>
              </div>
              {status.tiers.map((tier, index) => {
                const active = tier.value === status.mode;
                const looser = tier.value === "auto" || tier.value === "full";
                return (
                  <button
                    key={tier.value}
                    ref={(el) => {
                      tierButtons.current[index] = el;
                    }}
                    type="button"
                    disabled={saving}
                    onClick={() => chooseTier(tier)}
                    role="radio"
                    aria-checked={active}
                    tabIndex={active ? 0 : -1}
                    data-testid={`policy-tier-${tier.value}`}
                    className={cn(
                      "w-full rounded-md border p-3 text-left transition-colors",
                      "disabled:cursor-not-allowed disabled:opacity-60",
                      looser &&
                        "border-status-blocked/40 bg-status-blocked-soft hover:bg-status-blocked-soft",
                      active
                        ? looser
                          ? "ring-1 ring-status-blocked/30"
                          : "border-primary bg-primary/5"
                        : "hover:bg-muted/50",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">{tier.label}</span>
                      {active && (
                        <Badge variant="secondary" className="text-xs">
                          Current
                        </Badge>
                      )}
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {tier.description}
                    </p>
                  </button>
                );
              })}
              <p className="text-xs text-muted-foreground">
                Takes effect {status.takesEffect}.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="always-approve">Always ask first</Label>
              {/* Issue #1226: what an entry IS, said here rather than left to
                  the placeholder. `payment.send, filing.submit,
                  external.publish` used to be the only worked example this
                  field offered — the exact three strings issue #684 deleted
                  from the shipped default because, on the harness path, none of
                  them names a tool and so none of them gated anything. An
                  operator following the suggestion got a fence that was not
                  there, confirmed by a "list updated" toast.

                  A tool name and an effect kind were never two namespaces (see
                  `src/policy/always_approve.rs`), so naming the tool case first
                  is naming the case that applies to every company running the
                  openhuman toolbelt. The prefix rule is stated because it is
                  what `always_approve::matches` implements and nothing in the
                  console said it. */}
              <p className="text-xs text-muted-foreground">
                What the teammates always park for approval, whatever the tier —
                these win even on Full. Comma-separated. An entry is a tool name
                (<code>shell</code>, <code>http_request</code>), or a dotted
                effect kind a hosted brain emits; a leading segment matches the
                rest, so <code>invoice</code> covers{" "}
                <code>invoice.send</code>.
              </p>
              <Input
                id="always-approve"
                value={draftAlways}
                disabled={saving}
                list={wiredTools.length > 0 ? "always-approve-tools" : undefined}
                placeholder={alwaysAskPlaceholder(wiredTools)}
                onChange={(event) => {
                  setDraftAlways(event.target.value);
                  setDirty(true);
                }}
              />
              {/* Suggestions, never a constraint: the effect namespace is open
                  on purpose, because a hosted brain may emit a kind this
                  repository has never seen, and a `datalist` leaves free text
                  free. Rendered only when the host served the set, so a host
                  predating the route degrades to the plain box. */}
              {wiredTools.length > 0 && (
                <datalist id="always-approve-tools">
                  {wiredTools.map((slug) => (
                    <option key={slug} value={slug} />
                  ))}
                </datalist>
              )}
              {unmatchedNote && (
                <p className="text-xs text-muted-foreground">{unmatchedNote}</p>
              )}
              {dirty && (
                <Button
                  size="sm"
                  disabled={saving}
                  onClick={() => void saveAlways()}
                >
                  Save list
                </Button>
              )}
            </div>

            {status.overridden && (
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-dashed p-3">
                <p className="text-xs text-muted-foreground">
                  Set here{status.setBy ? ` by ${status.setBy}` : ""}, overriding
                  the manifest ({status.manifestMode}). Editing{" "}
                  <code>[policy]</code> in <code>company.toml</code> clears it —
                  version control wins when it speaks.
                </p>
                <Button
                  ref={resetButtonRef}
                  size="sm"
                  variant="outline"
                  disabled={saving}
                  onClick={() => requestReset()}
                >
                  <RotateCcw className="mr-1 h-3 w-3" />
                  Use the manifest's policy
                </Button>
              </div>
            )}
            <AlertDialog
              open={
                tierAwaitingConfirmation !== null || resetAwaitingConfirmation
              }
              onOpenChange={(open) => {
                if (!open) {
                  // A PUT/DELETE is in flight — keep the dialog up. The confirm
                  // action already stops the primitive's own Close, but Escape
                  // and outside-click still reach here; dismissing now would
                  // let the request finish (or fail) under a cancelled dialog
                  // instead of the promised retry UI. The close after a save
                  // is a state change from the `.then`, not a close request,
                  // so it is unaffected.
                  if (saving) return;
                  setTierAwaitingConfirmation(null);
                  setResetAwaitingConfirmation(false);
                }
              }}
            >
              <AlertDialogContent
                // A tier escalation is opened from the radio the operator
                // pressed, which may not be the one that ends up selected —
                // cancelling leaves the old tier checked with focus on the new
                // one. Return focus to the checked tier so the roving-tabindex
                // group's next arrow key computes from the right radio. The
                // reset flow returns focus to the button that opened it — this
                // controlled dialog has no trigger of its own, so without an
                // explicit target Base UI would leave focus nowhere. A reset
                // that succeeds clears the override, so that button unmounts
                // before the dialog closes; the checked tier radio is the
                // fallback then, instead of letting focus fall out.
                finalFocus={() => {
                  const checkedIndex = status.tiers.findIndex(
                    (tier) => tier.value === status.mode,
                  );
                  const checked =
                    checkedIndex === -1
                      ? null
                      : tierButtons.current[checkedIndex] ?? null;
                  if (confirmSource.current === "reset") {
                    return resetButtonRef.current ?? checked;
                  }
                  return checked;
                }}
              >
                <AlertDialogHeader>
                  <AlertDialogTitle>
                    Give teammates more autonomy?
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    {resetAwaitingConfirmation ? (
                      <>
                        Reverting clears the override set here and returns to
                        the manifest's{" "}
                        {manifestTier?.label ?? status.manifestMode} setting
                        {manifestTier ? ` — ${manifestTier.description}` : ""}.
                        They will use that setting on their next turn.
                        {manifestTier && (
                          <>
                            {" "}
                            {manifestTier.value !== status.mode
                              ? "This also"
                              : "This"}{" "}
                            replaces the current always-ask list with the
                            manifest's list:{" "}
                            {status.manifestAlwaysApprove.length > 0
                              ? status.manifestAlwaysApprove.join(", ")
                              : "none"}
                            {removedAlwaysAsk.length > 0 &&
                              `; ${removedAlwaysAsk.join(", ")} ${
                                removedAlwaysAsk.length === 1
                                  ? "stops"
                                  : "stop"
                              } always asking for approval`}
                            .
                          </>
                        )}
                      </>
                    ) : (
                      <>
                        Instead of:{" "}
                        {
                          status.tiers.find(
                            (tier) => tier.value === status.mode,
                          )?.description
                        }{" "}
                        With {tierAwaitingConfirmation?.label}:{" "}
                        {tierAwaitingConfirmation?.description} They will use
                        the {tierAwaitingConfirmation?.label} setting on their
                        next turn.
                      </>
                    )}
                  </AlertDialogDescription>
                  <p className="text-sm text-muted-foreground">
                    {resetAwaitingConfirmation
                      ? "Reset replaces the whole policy override, including the always-ask list."
                      : dirty
                        ? "Your saved always-ask list still wins, even on Full — save the list to enforce new gates."
                        : "Your always-ask list still wins, even on Full."}
                  </p>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel disabled={saving}>
                    Keep current setting
                  </AlertDialogCancel>
                  <AlertDialogAction
                    data-testid="policy-tier-confirm"
                    disabled={saving}
                    onClick={(event) => {
                      // The primitive's `Close` would dismiss the dialog
                      // before the PUT resolves, so prevent it and close
                      // explicitly only after a successful save — a failed
                      // persistence keeps the dialog open for a retry.
                      event.preventBaseUIHandler();
                      if (tierAwaitingConfirmation) {
                        void saveTier(tierAwaitingConfirmation.value).then(
                          (saved) => {
                            if (saved) setTierAwaitingConfirmation(null);
                          },
                        );
                      } else if (resetAwaitingConfirmation) {
                        void reset().then((saved) => {
                          if (saved) setResetAwaitingConfirmation(false);
                        });
                      }
                    }}
                  >
                    {resetAwaitingConfirmation
                      ? "Revert and give more autonomy"
                      : "Give more autonomy"}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </>
        )}
      </CardContent>
    </Card>
  );
}
