// Create a clean company, or reset a junk one, from the console (issue #1807).
//
// The host has provisioned a company over `POST /api/v1/companies` since #605,
// but nothing in the console reached it: an operator could archive or suspend a
// company (Settings → Lifecycle, #1401) yet had no way to make a fresh one, and
// no first-class "reset". This dialog is both.
//
// There is no purge route, so "reset" is not a wipe. The only honest reset the
// host can do is **archive the old company and provision a clean one**: archive
// retires it and removes it from the registry (its data is retained, not
// deleted — `server/provision.rs`), which also frees its id, name and quota slot
// for the replacement. So the reset path archives first, then creates.
//
// Every trigger for this dialog is gated on `client.carriesPlatformBearer`
// (see `canCreateCompanies`): provisioning, archiving and suspending are all
// `PlatformScope` routes a session cookie can never reach, so offering an
// enabled control to a magic-link operator would be the exact #1401
// dishonest-button bug one surface over. The gated call sites show a disabled
// control with an honest note instead.

import { useEffect, useState } from "react";
import { Loader2, TriangleAlert } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { CompanyStatus } from "@/api/types";
import { adminEmailProblem } from "@/lib/company-setup";
import {
  buildManifestToml,
  collidesWithArchived,
  describeProvisionError,
  resetReplacementId,
  wasAlreadyArchived,
  wasAmbiguousProvisionOutcome,
} from "@/lib/company-manifest";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

/**
 * What the operator asked to open the dialog for.
 *
 * `create` is a clean new company. `reset` carries the company being replaced —
 * its id (to archive) and name (to prefill and to name in the copy).
 */
export type CreateCompanyRequest =
  | { kind: "create" }
  | { kind: "reset"; company: string; name: string };

/** The host default the form starts on; the host injects this when omitted. */
const DEFAULT_POLICY_MODE = "auto";

/** The approval tiers the host accepts (`POLICY_MODES` in `company/types.rs`). */
const POLICY_MODES: { value: string; label: string }[] = [
  { value: "readonly", label: "Read-only — never acts on its own" },
  { value: "supervised", label: "Supervised — asks before acting" },
  { value: "auto", label: "Auto — acts, asks on the risky calls" },
  { value: "full", label: "Full — acts without asking" },
];

/** The honest line a gated trigger shows in place of an enabled control. */
export const CREATE_UNAVAILABLE_NOTE =
  "Creating a company needs a platform credential, which a person signed in here doesn't hold.";

/** Whether this client can reach the provisioning + archive routes at all. */
export function canCreateCompanies(client: OpenCompanyClient): boolean {
  return client.carriesPlatformBearer;
}

interface Props {
  client: OpenCompanyClient;
  /** The open request, or `null` when the dialog is closed. */
  request: CreateCompanyRequest | null;
  /**
   * Close the dialog (operator cancelled, or it finished).
   *
   * `archived` is true when a reset's archive leg already landed before the
   * dialog closed — cancelled after the archive, or after a create failure
   * the operator gave up retrying. The parent must refresh whatever roster
   * or company it is showing in that case: the company the picker or console
   * still displays is the one this reset just removed, and nothing else
   * tells it that (codex review on #1828, PR comment 3863028405).
   */
  onClose: (archived: boolean) => void;
  /**
   * A company was provisioned. The parent switches the console into it; on a
   * reset the old company has already been archived by the time this fires.
   */
  onCreated: (status: CompanyStatus) => void;
}

export function CreateCompanyDialog({ client, request, onClose, onCreated }: Props) {
  const [name, setName] = useState("");
  const [adminEmail, setAdminEmail] = useState("");
  const [policyMode, setPolicyMode] = useState(DEFAULT_POLICY_MODE);
  const [explicitId, setExplicitId] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Whether the reset's archive already landed, so a retry after a failed
  // *create* does not archive a second time (the id is already gone) and the
  // error copy can say the old company is already retired.
  const [archived, setArchived] = useState(false);
  // Whether the operator has directly edited the id field since the dialog
  // opened, as opposed to it merely holding the value `resetReplacementId`
  // pre-filled it with. Distinct from "is `explicitId` non-blank": a reset's
  // id field is *always* non-blank on open (see below), so blankness alone
  // cannot tell "the operator typed this" from "this is our own generated
  // default" — which `submit` needs to know before it may safely reconcile
  // an ambiguous provision response by looking the id up (an operator-typed
  // id could belong to someone else's company; a value we generated
  // ourselves cannot).
  const [idTouched, setIdTouched] = useState(false);

  const isReset = request?.kind === "reset";

  // Reset the form to the request each time the dialog opens. Keyed on the
  // request object so reopening for a different company re-seeds the name.
  useEffect(() => {
    if (!request) return;
    setName(request.kind === "reset" ? request.name : "");
    setAdminEmail("");
    setPolicyMode(DEFAULT_POLICY_MODE);
    // Reset pre-seeds a fresh id rather than leaving this blank: the name
    // field above is pre-filled with the archived company's own name, and an
    // unset id here would have the host derive that same name back into the
    // same id (`company_id_from_name`) — reprovisioning over the archived
    // company's own durable record instead of a clean one. See
    // `resetReplacementId`. The operator can still overwrite it from
    // Advanced; `create` leaves this blank as before, since there is no prior
    // company for a fresh name to collide with.
    setExplicitId(request.kind === "reset" ? resetReplacementId(request.company) : "");
    setAdvanced(false);
    setBusy(false);
    setError(null);
    setArchived(false);
    setIdTouched(false);
  }, [request]);

  if (!request) return null;

  async function submit() {
    if (!request) return;
    const trimmedName = name.trim();
    if (!trimmedName || busy) return;

    // Validate every replacement field before the destructive archive leg
    // runs, not just before provisioning. A malformed admin email used to
    // reach the host only after the old company was already gone: the
    // manifest validator has no way to see it until `provisionCompany` is
    // called, which — on a reset — is the second half of the request, after
    // `client.lifecycle("archive", …)` already ran. The operator would see
    // "Archived X, but couldn't create the new company" for a typo that a
    // pure check catches for free. Same rule `company-setup.ts` uses for the
    // same reason (codex review on #1828, PR comment 3862711345).
    //
    // `required: true` — this dialog cannot tell whether the host has a
    // deployment-wide `OPENCOMPANY_ADMIN_EMAIL` bootstrap grant (`serve`
    // without a manager injecting it is a documented no-op, AGENTS.md
    // "OPENCOMPANY_ADMIN_EMAIL"), so it must not assume one exists the way
    // the help text below used to. Leaving this blank on a host with no
    // bootstrap admin provisions — or, on a reset, *reprovisions* —  a
    // company whose manifest names nobody: `no_env_admin_leaves_a_provisioned
    // _company_refusing_everyone` confirms no address can then sign in. On a
    // reset that is destructive: the usable old company is archived before
    // its now-inaccessible replacement exists (codex review on #1828, PR
    // comment 3864885200).
    const emailProblem = adminEmailProblem(adminEmail, true);
    if (emailProblem) {
      setError(emailProblem);
      return;
    }

    // Reject a replacement id that is the same one about to be archived —
    // full id or, under shared-single-DB tenant namespacing, its bare form.
    // `resetReplacementId` seeds a fresh default, but the field stays
    // editable from Advanced, and typing the archived company's own id back
    // in — a likely move for an operator trying to keep the slug — recreates
    // the exact collision that default exists to avoid: `RuntimeBuilder::build`
    // reloads any existing durable record for an id before building over it,
    // so the "clean" replacement would come back carrying the archived
    // company's lifecycle, ledger and overlays. Caught before archiving, not
    // just before provisioning, so a bad id never leaves the operator with
    // the old company already gone and no way to retry cleanly (codex review
    // on #1828, PR comments 3861770475 and 3862711330).
    if (request.kind === "reset" && collidesWithArchived(explicitId.trim(), request.company)) {
      setError(
        `The replacement id can't be ${request.company} — that's the company being archived. Leave the field blank for an auto-generated id, or choose a different one.`,
      );
      return;
    }

    setBusy(true);
    setError(null);

    // Archive FIRST on a reset — it frees the id, name and quota slot the new
    // company would otherwise collide with. Done once: `archived` guards a
    // retry after the *create* half failed.
    let didArchive = archived;
    if (request.kind === "reset" && !archived) {
      try {
        await client.lifecycle("archive", request.company);
        setArchived(true);
        didArchive = true;
      } catch (err) {
        if (wasAlreadyArchived(err)) {
          // Our own earlier attempt already archived it; the response just
          // never arrived. Proceed to the create leg instead of reporting a
          // failure that didn't happen.
          setArchived(true);
          didArchive = true;
        } else {
          // Nothing was created, and the archive did not take — say so
          // plainly and leave the operator where they are to try again.
          setError(
            `Couldn't archive ${request.name}: ${describeProvisionError(err)} Nothing was changed.`,
          );
          setBusy(false);
          return;
        }
      }
    }

    const explicit = explicitId.trim();
    // A reset always sends an explicit id, even if the operator cleared the
    // Advanced field back to empty: falling through to the unset-id default
    // would have the host re-derive the archived company's own id from the
    // (possibly untouched) name field above. See `resetReplacementId`.
    const id = explicit || (request.kind === "reset" ? resetReplacementId(request.company) : "");
    // Whether `id` is one this client generated itself, rather than one the
    // operator typed — declared outside the try below so the catch block can
    // see it. Two cases count as ours: the field still holds exactly what
    // `resetReplacementId` seeded it with on open (`!idTouched`), or the
    // operator cleared it back to blank, which — per the fallback above —
    // still lands on a freshly generated id, never the unset-id default.
    // Anything else the operator has typed in is theirs; reconciling that by
    // looking it up could resolve to an unrelated, pre-existing company.
    const selfGenerated = request.kind === "reset" && (!idTouched || explicit === "");
    const autoId = selfGenerated ? id : "";

    try {
      const manifest_toml = buildManifestToml({
        name: trimmedName,
        adminEmail: adminEmail.trim() || undefined,
        // Omitted at the default so the host records its own `auto`, rather
        // than pinning the tier in the manifest text.
        policyMode: policyMode !== DEFAULT_POLICY_MODE ? policyMode : undefined,
      });
      const body: { manifest_toml: string; id?: string } = { manifest_toml };
      if (id) body.id = id;
      const status = await client.provisionCompany(body);
      onCreated(status);
    } catch (err) {
      // A dropped connection — or a retry that lands on the id this client
      // itself just asked for — is ambiguous by construction: the host may
      // have provisioned the company and only the reply, or the collision
      // check, makes it look like nothing happened. Reconcile with a status
      // lookup before reporting failure. Scoped to `autoId`, never an
      // operator-typed one: `resetReplacementId`'s random suffix can't
      // collide with a pre-existing company, so a hit there can only be this
      // request landing — an operator-typed id could genuinely belong to an
      // unrelated company, and switching the console into that would be
      // worse than the misleading error it replaces (codex review on #1828,
      // PR comment 3863028397).
      if (autoId && wasAmbiguousProvisionOutcome(err)) {
        try {
          onCreated(await client.status(autoId));
          return;
        } catch {
          // Genuinely not there — fall through to the ordinary error path.
        }
      }
      const reason = describeProvisionError(err);
      // The dangerous half-state: the old company is archived but its
      // replacement did not land. Never swallow it — name both facts so the
      // operator understands the picker no longer lists the old company and
      // can retry the create (the archive won't repeat).
      setError(
        didArchive && request.kind === "reset"
          ? `Archived ${request.name}, but couldn't create the new company: ${reason} Adjust and try again.`
          : reason,
      );
    } finally {
      setBusy(false);
    }
  }

  const title = isReset ? "Reset / start clean" : "New company";
  const description = isReset
    ? `This archives ${request.name} (its data is retained, not deleted) and creates a fresh, empty company in its place.`
    : "Provision a clean company on this host. You'll land in it once it's created.";
  const submitLabel = isReset ? "Archive & start clean" : "Create company";

  return (
    // Ignore a dismiss request while busy — Escape and the built-in ✕ close
    // button both fire the same `onOpenChange(false)` as Cancel (already
    // `disabled={busy}` below), and without this gate they would bypass that
    // guard: `onClose` clears `request`, this component then renders `null`,
    // but the in-flight `submit()` keeps running against a dialog the
    // operator believes they dismissed. On a reset that is the dangerous
    // half-state made invisible — a late archive-succeeded/create-failed
    // writes its warning into a hidden dialog, and a late success still calls
    // `onCreated`, navigating the operator into a company they thought they
    // had cancelled out of.
    <Dialog open onOpenChange={(open) => !open && !busy && onClose(archived)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>

        {error && (
          <Alert variant="destructive" data-testid="create-company-error">
            <TriangleAlert className="size-4" />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        )}

        <div className="grid gap-1.5">
          <Label htmlFor="create-company-name">Company name</Label>
          <Input
            id="create-company-name"
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. Acme Robotics"
            disabled={busy}
          />
        </div>

        <div className="grid gap-1.5">
          <Label htmlFor="create-company-admin">Admin email</Label>
          <Input
            id="create-company-admin"
            type="email"
            value={adminEmail}
            onChange={(e) => setAdminEmail(e.target.value)}
            placeholder="who can sign in as an admin"
            disabled={busy}
          />
          <p className="text-2xs text-muted-foreground">
            Required — a company provisioned with no admin here has nobody
            eligible to sign in unless this host has its own bootstrap admin
            configured.
          </p>
        </div>

        <div className="grid gap-1.5">
          <button
            type="button"
            className="w-fit text-xs font-medium text-muted-foreground underline underline-offset-2 hover:text-foreground"
            onClick={() => setAdvanced((v) => !v)}
            aria-expanded={advanced}
          >
            {advanced ? "Hide advanced" : "Advanced"}
          </button>
          {advanced && (
            <div className="grid gap-3 rounded-lg border p-3">
              <div className="grid gap-1.5">
                <Label htmlFor="create-company-policy">Approval tier</Label>
                <Select
                  value={policyMode}
                  onValueChange={(v) => setPolicyMode((v as string) ?? DEFAULT_POLICY_MODE)}
                  disabled={busy}
                >
                  <SelectTrigger id="create-company-policy">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {POLICY_MODES.map((mode) => (
                      <SelectItem key={mode.value} value={mode.value}>
                        {mode.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-2xs text-muted-foreground">
                  Leave on Auto to use the host default.
                </p>
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="create-company-id">Company id (optional)</Label>
                <Input
                  id="create-company-id"
                  value={explicitId}
                  onChange={(e) => {
                    setExplicitId(e.target.value);
                    setIdTouched(true);
                  }}
                  placeholder={
                    isReset
                      ? "auto-generated, distinct from the archived id"
                      : "derived from the name when left blank"
                  }
                  disabled={busy}
                  className="font-mono text-xs"
                />
              </div>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onClose(archived)} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant={isReset ? "destructive" : name.trim() ? "default" : "secondary"}
            onClick={() => void submit()}
            disabled={busy || !name.trim()}
          >
            {busy && <Loader2 className="mr-1.5 size-4 animate-spin" />}
            {submitLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
