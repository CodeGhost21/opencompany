import { useCallback, useEffect, useState } from "react";
import { Check, Copy, CreditCard, Loader2, TriangleAlert } from "lucide-react";
import { toast } from "sonner";

import {
  clearBilling,
  getBilling,
  saveBilling,
  type BillingStatus,
} from "@/api/billing";
import type { OpenCompanyClient } from "@/api/client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

/**
 * Settings → Billing: the company's Chargebee connection (issue #788, #527).
 *
 * # Why four states and not a "Connected ✓" badge
 *
 * Four separate things can each be missing, and three of them are invisible
 * from this form's own fields:
 *
 *   - no key or site  — the agent has no billing tools at all
 *   - no webhook      — the tools work, but nobody is told when a customer pays
 *   - not granted     — both credentials stored and STILL nothing reaches an
 *                       agent, because the manifest does not grant `chargebee`.
 *                       The fix is `company.toml`, not this page.
 *   - not in build    — the running host was compiled without the feature, so
 *                       no amount of configuring will do anything.
 *
 * A single "Connected" badge would be green for the last three and send an
 * operator hunting through this form for a problem that is not in it. So each
 * is reported on its own terms, and the two that this page cannot fix say where
 * the fix actually is.
 *
 * # Credentials are write-only
 *
 * The API key and the webhook credential are never returned by the host, so
 * they are never rendered. A stored key shows as "Configured", and the input
 * stays empty with a placeholder saying that typing replaces it — an input
 * pre-filled with dots invites an operator to "correct" a value they cannot
 * see, and submitting the dots would store the dots.
 */
export function BillingView({ client, company }: Props) {
  const [status, setStatus] = useState<BillingStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [apiKey, setApiKey] = useState("");
  const [site, setSite] = useState("");
  const [webhookSecret, setWebhookSecret] = useState("");

  const load = useCallback(async () => {
    try {
      const next = await getBilling(client, company);
      setStatus(next);
      setLoadError(null);
      // Seed the site box with what is stored — it is the one non-secret field,
      // and an operator correcting a typo should not have to retype it.
      setSite(next.site ?? "");
    } catch (err) {
      setLoadError(err instanceof Error ? err.message : String(err));
    }
  }, [client, company]);

  useEffect(() => {
    void load();
  }, [load]);

  async function onSave() {
    // Send only what was actually entered. The host treats this as a patch, so
    // an untouched key keeps its stored value rather than being cleared.
    const body: Record<string, string> = {};
    if (apiKey.trim()) body.apiKey = apiKey.trim();
    if (site.trim() && site.trim() !== (status?.site ?? "")) body.site = site.trim();
    if (webhookSecret.trim()) body.webhookSecret = webhookSecret.trim();

    if (Object.keys(body).length === 0) {
      toast.info("Nothing to save — fill in a field first.");
      return;
    }

    setBusy(true);
    try {
      const next = await saveBilling(client, company, body);
      setStatus(next);
      // Clear the secret inputs on success: leaving a key sitting in a form
      // field after it has been stored is one stray screen-share from a leak.
      setApiKey("");
      setWebhookSecret("");
      toast.success("Chargebee settings saved.");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not save Chargebee settings.");
    } finally {
      setBusy(false);
    }
  }

  async function onClear() {
    setBusy(true);
    try {
      const next = await clearBilling(client, company);
      setStatus(next);
      setApiKey("");
      setWebhookSecret("");
      setSite("");
      toast.success("Chargebee credentials cleared.");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Could not clear the credentials.");
    } finally {
      setBusy(false);
    }
  }

  if (loadError) {
    return (
      <Alert variant="destructive" data-testid="billing-load-error">
        <TriangleAlert className="size-4" />
        <AlertDescription>Could not load billing settings: {loadError}</AlertDescription>
      </Alert>
    );
  }

  if (!status) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        <Loader2 className="mr-2 size-4 animate-spin" /> Loading billing…
      </div>
    );
  }

  const connected = status.apiKeyConfigured && !!status.site;

  return (
    <div className="space-y-6" data-testid="billing-view">
      <div>
        <h2 className="flex items-center gap-2 text-lg font-medium">
          <CreditCard className="size-5" /> Billing
        </h2>
        <p className="text-sm text-muted-foreground">
          Connect Chargebee so your teammates can raise invoices and answer questions about who has
          paid, without leaving this app.
        </p>
      </div>

      {/* The two problems this form cannot fix, said before the form so an
          operator does not fill it in and wonder why nothing happened. */}
      {!status.inBuild ? (
        <Alert data-testid="billing-not-in-build">
          <TriangleAlert className="size-4" />
          <AlertDescription>
            This host was built without Chargebee support, so these settings will be stored and
            have no effect. Rebuild with the <code>chargebee</code> feature.
          </AlertDescription>
        </Alert>
      ) : null}

      {status.inBuild && !status.granted ? (
        <Alert data-testid="billing-not-granted">
          <TriangleAlert className="size-4" />
          <AlertDescription>
            This company does not grant <code>chargebee</code>, so billing tools will not reach any
            teammate even once these credentials are saved. Add <code>chargebee</code> to{" "}
            <code>[tools].allow</code> in the company&rsquo;s manifest — it cannot be fixed from
            this page.
          </AlertDescription>
        </Alert>
      ) : null}

      <Card>
        <CardContent className="space-y-4 pt-6">
          <div className="flex items-center justify-between">
            <div className="space-y-1">
              <p className="text-sm font-medium">Chargebee</p>
              <p className="text-xs text-muted-foreground">
                {connected
                  ? `Connected to ${status.site}.chargebee.com`
                  : "Not connected yet."}
              </p>
            </div>
            {connected ? (
              <Badge variant="secondary" data-testid="billing-connected">
                <Check className="mr-1 size-3" /> Connected
              </Badge>
            ) : null}
          </div>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="cb-site">Site</Label>
              <Input
                id="cb-site"
                data-testid="billing-site"
                placeholder="acme-test"
                value={site}
                onChange={(e) => setSite(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                The part before <code>.chargebee.com</code>. Pasting the full URL is fine.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="cb-key">API key</Label>
              <Input
                id="cb-key"
                data-testid="billing-api-key"
                type="password"
                autoComplete="off"
                placeholder={
                  status.apiKeyConfigured ? "Configured — type to replace" : "From Chargebee → API Keys"
                }
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">
                {status.apiKeyConfigured
                  ? "Stored. It is never shown again."
                  : "Stored write-only; it is never shown again."}
              </p>
            </div>
          </div>

          <div className="flex gap-2">
            <Button onClick={onSave} disabled={busy} data-testid="billing-save">
              {busy ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
              Save
            </Button>
            {status.apiKeyConfigured || status.webhookConfigured ? (
              <Button
                variant="ghost"
                onClick={onClear}
                disabled={busy}
                data-testid="billing-clear"
              >
                Disconnect
              </Button>
            ) : null}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="space-y-4 pt-6">
          <div className="space-y-1">
            <p className="text-sm font-medium">Payment notifications</p>
            <p className="text-xs text-muted-foreground">
              Optional. Without this, invoicing still works — nobody is just told when a customer
              pays.
            </p>
          </div>

          <div className="space-y-2">
            <Label>Webhook URL</Label>
            {status.webhookUrl ? (
              <div className="flex gap-2">
                <Input readOnly value={status.webhookUrl} data-testid="billing-webhook-url" />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => {
                    void navigator.clipboard.writeText(status.webhookUrl ?? "");
                    toast.success("Webhook URL copied.");
                  }}
                >
                  <Copy className="size-4" />
                </Button>
              </div>
            ) : (
              // Deliberately not a disabled box showing a loopback address:
              // Chargebee cannot deliver to one, and showing it would send an
              // operator to configure a webhook that silently never arrives.
              <p className="text-xs text-muted-foreground" data-testid="billing-no-webhook-url">
                This host has no public URL, so Chargebee cannot reach it. Set{" "}
                <code>OPENCOMPANY_PUBLIC_URL</code> to an https address to get a webhook URL.
              </p>
            )}
          </div>

          <div className="space-y-2">
            <Label htmlFor="cb-hook">Webhook credential</Label>
            <Input
              id="cb-hook"
              data-testid="billing-webhook-secret"
              type="password"
              autoComplete="off"
              placeholder={
                status.webhookConfigured ? "Configured — type to replace" : "username:password"
              }
              value={webhookSecret}
              onChange={(e) => setWebhookSecret(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              Invent a username and password, save them here as{" "}
              <code>username:password</code>, then set the same pair in Chargebee under{" "}
              <em>Protect webhook URL with basic authentication</em>. Subscribe to{" "}
              <code>payment_succeeded</code> and <code>payment_failed</code>.
            </p>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
