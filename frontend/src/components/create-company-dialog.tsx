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
import {
  buildManifestToml,
  describeProvisionError,
  resetReplacementId,
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
  /** Close the dialog (operator cancelled, or it finished). */
  onClose: () => void;
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
  }, [request]);

  if (!request) return null;

  async function submit() {
    if (!request) return;
    const trimmedName = name.trim();
    if (!trimmedName || busy) return;
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
        // Nothing was created, and the archive did not take — say so plainly
        // and leave the operator where they are to try again.
        setError(
          `Couldn't archive ${request.name}: ${describeProvisionError(err)} Nothing was changed.`,
        );
        setBusy(false);
        return;
      }
    }

    try {
      const manifest_toml = buildManifestToml({
        name: trimmedName,
        adminEmail: adminEmail.trim() || undefined,
        // Omitted at the default so the host records its own `auto`, rather
        // than pinning the tier in the manifest text.
        policyMode: policyMode !== DEFAULT_POLICY_MODE ? policyMode : undefined,
      });
      const body: { manifest_toml: string; id?: string } = { manifest_toml };
      // A reset always sends an explicit id, even if the operator cleared the
      // Advanced field back to empty: falling through to the unset-id default
      // would have the host re-derive the archived company's own id from the
      // (possibly untouched) name field above. See `resetReplacementId`.
      const id =
        explicitId.trim() ||
        (request.kind === "reset" ? resetReplacementId(request.company) : "");
      if (id) body.id = id;
      const status = await client.provisionCompany(body);
      onCreated(status);
    } catch (err) {
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
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
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
          <Label htmlFor="create-company-admin">Admin email (optional)</Label>
          <Input
            id="create-company-admin"
            type="email"
            value={adminEmail}
            onChange={(e) => setAdminEmail(e.target.value)}
            placeholder="who can sign in as an admin"
            disabled={busy}
          />
          <p className="text-2xs text-muted-foreground">
            Whoever provisioned this host can already sign in; add an address here
            to let someone else in without an invite first.
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
                  onChange={(e) => setExplicitId(e.target.value)}
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
          <Button variant="outline" onClick={onClose} disabled={busy}>
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
