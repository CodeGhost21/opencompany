import { useEffect, useState } from "react";
import { AlertTriangle, Loader2, LogIn, Unplug } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { ComposioConnectedAccount } from "@/api/composio";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { callsForProvider, connectedOn } from "@/lib/connection-detail";
import { toolkitSlug } from "@/lib/connections";
import type { GridProvider } from "@/lib/provider-grid";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The provider to show, or `null` when nothing is open. */
  provider: GridProvider | null;
  /** Whether this viewer may change what the company connects through (#403). */
  canManage: boolean;
  /** A connect or disconnect is in flight somewhere on the page. */
  busy: boolean;
  onClose: () => void;
  onConnectAnother: (provider: GridProvider) => void;
  onDisconnectAccount: (provider: GridProvider, account: ComposioConnectedAccount) => void;
}

/** The window the usage figure covers. Matches the Usage view's own default. */
const USAGE_RANGE = "30d";

/**
 * A connected provider as an object you can open (issue #404).
 *
 * ## What this is honest about, and why each line is worded the way it is
 *
 * The issue's requirement is not "show more fields" — it is that every claim on
 * this panel is one the system can actually back. Three of them could not be
 * made naively:
 *
 *  1. **Which account an agent uses.** OpenHuman marks the first of several as
 *     the default, and inheriting that was the plan. It is not true here:
 *     `composio_execute` posts `{tool, arguments}` and **no connection id**
 *     (`src/harness/composio.rs`, and `execute_tool` in the shared client), so
 *     nothing on this side selects an account — Composio resolves it for the
 *     entity. A "Default" chip would name a decision this product does not
 *     make. The panel says that instead.
 *  2. **When it was connected.** Only Composio records it. The native
 *     `oauth/{provider}` store keeps `{token, account}` and journals nothing on
 *     connect, and MCP has no such concept — so for those the date is not
 *     merely missing, it is unrecoverable. An empty cell would read as
 *     "never"; the panel says "not recorded".
 *  3. **What has gone through it.** Composio meters a successful
 *     `composio_execute` per toolkit (`src/metering/oauth.rs`), so the number
 *     here is real — but it is per *provider*, not per *account*, and both
 *     accounts of a two-Gmail company land on the same total. Rendering it
 *     against one account would be a number that means something else.
 *
 * ## Composio only
 *
 * The native OAuth catalog gets no detail view. Its credential is written and
 * read by nothing (#396), and a page devoted to how healthy a connection is,
 * is exactly where an inert one would look most alive. A provider that also
 * holds a native credential says so as a caveat, not as a second connection.
 */
export function ProviderDetail({
  client,
  company,
  provider,
  canManage,
  busy,
  onClose,
  onConnectAnother,
  onDisconnectAccount,
}: Props) {
  const slug = provider ? toolkitSlug(provider.slug) : null;
  const [calls, setCalls] = useState<number | null>(null);
  const [usageLoad, setUsageLoad] = useState<"loading" | "ready" | "unavailable">("loading");

  useEffect(() => {
    if (slug === null) return;
    let alive = true;
    setUsageLoad("loading");
    setCalls(null);
    client
      .usage(USAGE_RANGE, company)
      .then((usage) => {
        if (!alive) return;
        setCalls(callsForProvider(usage.byProvider, slug));
        setUsageLoad("ready");
      })
      // A host without the usage route (older build) 404s. "Not recorded here"
      // is the honest render — not a zero, which claims the calls were counted
      // and there were none.
      .catch(() => {
        if (alive) setUsageLoad("unavailable");
      });
    return () => {
      alive = false;
    };
  }, [client, company, slug]);

  const accounts = provider?.accounts ?? [];
  const live = accounts.filter((a) => a.connected);

  return (
    <Sheet open={provider !== null} onOpenChange={(next) => !next && onClose()}>
      <SheetContent side="right" className="w-full overflow-y-auto sm:max-w-md">
        {provider && (
          <>
            <SheetHeader className="border-b">
              <SheetTitle className="flex items-center gap-2">
                <span className="truncate">{provider.label}</span>
              </SheetTitle>
              <SheetDescription className="flex flex-wrap items-center gap-1.5 text-xs">
                <Badge variant="outline" className="font-normal">
                  {/* Which of the three systems backs this connection. The
                      panel exists for providers Composio holds; the badge is
                      still stated rather than assumed, because "through which
                      system" is one of the questions the issue asks it to
                      answer. */}
                  Composio
                </Badge>
                <span>
                  {live.length === 0
                    ? "no active account"
                    : live.length === 1
                      ? "1 account connected"
                      : `${live.length} accounts connected`}
                </span>
              </SheetDescription>
            </SheetHeader>

            <div className="space-y-4 px-4 pb-6">
              <section className="space-y-2" aria-label="Connected accounts">
                {accounts.map((account) => (
                  <AccountRow
                    key={account.id}
                    account={account}
                    canManage={canManage}
                    busy={busy}
                    onDisconnect={() => onDisconnectAccount(provider, account)}
                  />
                ))}
                {accounts.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    No Composio account is connected for {provider.label}.
                  </p>
                )}
              </section>

              {live.length > 1 && (
                <p className="flex items-start gap-2 rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                  <AlertTriangle className="mt-px size-3 shrink-0" />
                  <span>
                    Holding several accounts is fine — they are the company&apos;s, and every member
                    works through them. Which one an agent acts as is not set here:{" "}
                    <span className="font-mono">composio_execute</span> sends no connection id, so
                    Composio resolves it. Disconnect the one you do not want an agent to use.
                  </span>
                </p>
              )}

              {provider.via.includes("native") && (
                <p className="flex items-start gap-2 rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                  <AlertTriangle className="mt-px size-3 shrink-0" />
                  <span>
                    This company also stores a self-hosted OAuth credential for {provider.label}. No
                    agent reads it (issue #396), it is not what the accounts above are, and
                    disconnecting here does not touch it.
                  </span>
                </p>
              )}

              <Separator />

              <section className="space-y-1" aria-label="Usage">
                <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                  Usage
                </h4>
                {usageLoad === "loading" && (
                  <p className="text-xs text-muted-foreground">Reading usage…</p>
                )}
                {usageLoad === "unavailable" && (
                  <p className="text-xs text-muted-foreground">
                    This host does not report usage, so what has gone through this connection is not
                    recorded here.
                  </p>
                )}
                {usageLoad === "ready" && (
                  <>
                    <p className="text-sm">
                      <span className="font-medium">{calls}</span>{" "}
                      {calls === 1 ? "call" : "calls"} in the last 30 days
                    </p>
                    <p className="text-xs text-muted-foreground">
                      Successful tool calls through {provider.label}, counted per provider rather
                      than per account — a call through any account above lands on this one total.
                    </p>
                  </>
                )}
              </section>

              {canManage && (
                <>
                  <Separator />
                  <section className="space-y-2" aria-label="Manage this connection">
                    <Button
                      variant="outline"
                      className="w-full"
                      disabled={busy}
                      onClick={() => onConnectAnother(provider)}
                      data-testid="provider-detail-connect-another"
                    >
                      <LogIn className="size-4" />
                      {accounts.length === 0 ? "Connect an account" : "Connect another account"}
                    </Button>
                    <p className="text-xs text-muted-foreground">
                      Disconnecting removes the connection at Composio, so agents lose these tools on
                      their next turn. It does not sign the company out of {provider.label}, and it
                      does not delete anything there.
                    </p>
                  </section>
                </>
              )}

              {!canManage && accounts.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  Only an admin can connect or disconnect an account here.
                </p>
              )}
            </div>
          </>
        )}
      </SheetContent>
    </Sheet>
  );
}

/** One connected account: what it is, since when, and how to release it. */
function AccountRow({
  account,
  canManage,
  busy,
  onDisconnect,
}: {
  account: ComposioConnectedAccount;
  canManage: boolean;
  busy: boolean;
  onDisconnect: () => void;
}) {
  return (
    <div
      className="flex items-start justify-between gap-2 rounded-lg border border-border px-3 py-2"
      data-testid={`provider-account-${account.id}`}
    >
      <div className="min-w-0 space-y-0.5">
        <p className="truncate text-sm font-medium">
          {/* Composio publishes no label for some providers, and guessing one
              from the toolkit makes two accounts indistinguishable at the one
              moment an operator has to tell them apart. */}
          {account.account ?? "Account name not published"}
        </p>
        <p className="text-xs text-muted-foreground">
          <Badge variant="outline" className="mr-1.5 font-mono font-normal">
            {/* Composio's own status string, not a re-spelling of it: "set up
                and since expired" and "never finished setting up" are
                different sentences and both flatten to "not connected". */}
            {account.status}
          </Badge>
          {connectedOn(account.createdAt)}
        </p>
      </div>
      {canManage && (
        <Button
          variant="destructive"
          size="sm"
          disabled={busy}
          onClick={onDisconnect}
          aria-label={`Disconnect ${account.account ?? account.id}`}
        >
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Unplug className="size-3.5" />}
          Disconnect
        </Button>
      )}
    </div>
  );
}
