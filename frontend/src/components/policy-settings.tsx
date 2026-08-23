import { useCallback, useEffect, useState } from "react";
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
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

/** Whether moving through the host-provided tier order gives agents more autonomy. */
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
  const [pendingTier, setPendingTier] = useState<PolicyStatus["tiers"][number] | null>(
    null,
  );
  // A reset restores the manifest's tier AND always-ask list, so the widening
  // check must run on it too — otherwise "Use the manifest's policy" is a
  // one-click way around the confirmation the tier buttons get, and the same
  // for always-ask gates the manifest does not carry. Remember which action
  // the dialog is confirming so the confirm button performs the same one the
  // operator asked for.
  const [pendingReset, setPendingReset] = useState(false);
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

  // Deliberately silent about its own failure, and deliberately not part of
  // `load`: these are suggestions under a free-text box. A host that cannot
  // serve them costs the operator a datalist, not the setting, and a second
  // error banner would report the policy card as broken when it is merely
  // plainer — the same reasoning `LedgersView.refreshTasks` gives.
  useEffect(() => {
    let live = true;
    void listWorkflowToolSlugs(client, company)
      .then((r) => {
        if (live) setWiredTools(r.slugs);
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

  const chooseTier = async (mode: string) => {
    if (!status || saving || mode === status.mode) return;
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
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not change the tier.",
      );
    } finally {
      setSaving(false);
    }
  };

  const requestTier = (tier: PolicyStatus["tiers"][number]) => {
    if (!status || saving || tier.value === status.mode) return;
    if (widensAutonomy(status.tiers, status.mode, tier.value)) {
      setPendingReset(false);
      setPendingTier(tier);
      return;
    }
    void chooseTier(tier.value);
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
      setPendingReset(true);
      setPendingTier(manifestTier);
      return;
    }
    void reset();
  };

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

  const reset = async () => {
    if (!status || saving) return;
    setSaving(true);
    try {
      apply(
        await resetPolicy(client, company),
        "Reverted to the manifest's policy",
      );
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not reset the policy.",
      );
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card data-testid="policy-settings">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
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
            <div className="space-y-2">
              <p className="text-xs font-medium text-muted-foreground">
                More freedom to act ↓
              </p>
              {status.tiers.map((tier) => {
                const active = tier.value === status.mode;
                const looser = tier.value === "auto" || tier.value === "full";
                return (
                  <button
                    key={tier.value}
                    type="button"
                    disabled={saving}
                    onClick={() => requestTier(tier)}
                    aria-pressed={active}
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

            <AlertDialog
              open={pendingTier !== null}
              onOpenChange={(open) => {
                if (!open) {
                  setPendingTier(null);
                  setPendingReset(false);
                }
              }}
            >
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>
                    Let teammates do more on their own?
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    {pendingTier && (
                      <>
                        {pendingTier.value !== status.mode && (
                          <>
                            Instead of:{" "}
                            {
                              status.tiers.find(
                                (tier) => tier.value === status.mode,
                              )?.description
                            }{" "}
                            With {pendingTier.label}: {pendingTier.description}
                            {" "}
                          </>
                        )}
                        {pendingReset && (
                          <>
                            {pendingTier.value !== status.mode
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
                    )}
                  </AlertDialogDescription>
                  <p className="text-sm text-muted-foreground">
                    {pendingReset
                      ? "Reset replaces the whole policy override, including the always-ask list."
                      : "Your always-ask list still wins, even on Full."}
                  </p>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Keep current setting</AlertDialogCancel>
                  <AlertDialogAction
                    onClick={() => {
                      if (pendingReset) {
                        void reset();
                      } else if (pendingTier) {
                        void chooseTier(pendingTier.value);
                      }
                    }}
                    data-testid="policy-tier-confirm"
                  >
                    {pendingReset
                      ? "Revert to the manifest's policy"
                      : `Use ${pendingTier?.label}`}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>

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
          </>
        )}
      </CardContent>
    </Card>
  );
}
