import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, Info, Loader2, Plug, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

import { me as fetchMe } from "@/api/auth";
import type { OpenCompanyClient } from "@/api/client";
import {
  getComposioStatus,
  listComposioConnections,
  startComposioAuthorize,
} from "@/api/composio";
import type { ConnectionState } from "@/api/types";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  CONNECTION_CATEGORY_ORDER,
  CONNECTION_PROVIDERS,
  connectionStateFor,
  connectRoute,
  type ComposioReach,
  type ConnectionProvider,
  type ConnectRoute,
} from "@/lib/connections";
import { cn } from "@/lib/utils";
import { armTourResume } from "@/tour/state";
import { InferenceSection } from "@/views/connections/InferenceSection";
import { McpServersSection } from "@/views/connections/McpServersSection";
import { CompanyCredentialCard } from "@/views/connections/CompanyCredentialCard";
import { ComposioSection } from "@/views/connections/ComposioSection";
import { ChannelsSection } from "./connections/ChannelsSection";
import { useLocalScope } from "@/connections/ConnectionContext";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
}

type Load = "loading" | "ready" | "unavailable";

/**
 * How long the Composio status probe may take before the grid stops waiting on
 * it. Mirrors the host's own `COMPOSIO_PROBE_TIMEOUT` on the connections read
 * path (`src/server/ops/connections_read.rs`) — the same argument applies on
 * this side of the wire: the answer it contributes is which route a tile takes,
 * and a page that paints honestly beats a page that never paints.
 */
const COMPOSIO_PROBE_TIMEOUT_MS = 5_000;

/** Wire the third-party accounts your company can act through. */
export function ConnectionsView({ client, company }: Props) {
  // Which (connection, company) this subtree's browser-local state belongs to.
  const scope = useLocalScope();
  const [load, setLoad] = useState<Load>("loading");
  const [states, setStates] = useState<Record<string, ConnectionState>>({});
  const [busy, setBusy] = useState<string | null>(null);
  // Bumped when the company credential changes, to remount the sections whose
  // reported tier is downstream of it (issue #586).
  const [credentialGeneration, setCredentialGeneration] = useState(0);
  // Whether this viewer may change what the company connects through (issue
  // #403). A connection belongs to the company, so changing one is an admin's
  // call; reading the page is everyone's.
  //
  // **Courtesy, not enforcement.** The host refuses every write on this page
  // with a 403 whatever this says. All hiding the controls prevents is offering
  // an operator a button that cannot work — which on this page would be a
  // particularly poor greeting, since the failure arrives only after they have
  // pasted a live credential into a form that could never submit it.
  const [canManage, setCanManage] = useState(false);
  // What Composio offers here, or `null` while unknown / not reachable. This is
  // the hosted connect route for every tile (issue #599), so the grid cannot
  // decide what a Connect does without it.
  const [reach, setReach] = useState<ComposioReach | null>(null);
  // Whether this instance carries a platform-projected identity, as Composio
  // reports it. A second witness for the same host-level fact the connection
  // rows carry — and the only one available when the manifest declares no
  // connections, which is exactly when the #319 guard used to go dark.
  const [attested, setAttested] = useState(false);
  // Whether the Composio probe has answered. The grid must not paint before it
  // has: `refresh()` routinely resolves first, and a tile rendered on a null
  // `reach` reads "Not available on this host" — so every tile would flash that
  // and then flip to Connect a moment later.
  const [reachSettled, setReachSettled] = useState(false);
  // Poll timers for Composio sign-ins in flight, keyed by toolkit, so a company
  // switch or unmount cannot leave one running.
  const pollTimers = useRef<Record<string, number>>({});

  const refresh = useCallback(async () => {
    try {
      const list = await client.listConnections(company);
      setStates(Object.fromEntries(list.map((c) => [c.provider, c])));
      setLoad("ready");
    } catch {
      // No connections surface on this host yet — show the catalog read-only.
      setLoad("unavailable");
    }
  }, [client, company]);

  useEffect(() => {
    setLoad("loading");
    void refresh();
  }, [refresh]);

  // Composio status drives the hosted route for every tile. A host without the
  // feature, without the grant, or without a credential simply leaves `reach`
  // null, and `connectRoute` falls back to the native/managed/unavailable arms.
  useEffect(() => {
    let live = true;
    setReachSettled(false);
    void (async () => {
      try {
        // Bounded: the shared client has no abort or timeout, and the grid now
        // waits on this call before painting — so a host that accepts the
        // connection and never answers would hold the page on skeletons
        // forever. Losing the race is not an error, it is "no Composio route
        // we can confirm", which is what a null `reach` already means.
        const status = await Promise.race([
          getComposioStatus(client, company),
          new Promise<null>((resolve) => window.setTimeout(() => resolve(null), COMPOSIO_PROBE_TIMEOUT_MS)),
        ]);
        if (!live) return;
        if (!status) {
          setReach(null);
          setAttested(false);
          return;
        }
        setReach({
          inBuild: status.inBuild,
          granted: status.granted,
          hasCredential: status.credentialSource !== "none",
          openMode: status.openMode,
          effectiveToolkits: status.effectiveToolkits,
        });
        setAttested(status.credentialSource === "attested");
      } catch {
        // No Composio surface on this host — not an error for this page.
        if (live) {
          setReach(null);
          setAttested(false);
        }
      } finally {
        if (live) setReachSettled(true);
      }
    })();
    return () => {
      live = false;
    };
    // `credentialGeneration` is load-bearing, not decoration: this probe feeds
    // `reach.hasCredential` and `attested`, both of which are downstream of the
    // company credential. Setting a key flips `credentialSource` from `none` to
    // `company`, and without a re-probe the grid would keep every tile on the
    // "no credential" route while the Composio section right below it correctly
    // reported the opposite (issue #586).
  }, [client, company, credentialGeneration]);

  useEffect(() => {
    const timers = pollTimers.current;
    return () => {
      Object.values(timers).forEach((id) => window.clearTimeout(id));
      pollTimers.current = {};
    };
  }, [company]);

  useEffect(() => {
    let live = true;
    void (async () => {
      let admin = false;
      try {
        admin = (await fetchMe(client, company)).role === "admin";
      } catch {
        // No user plane on this host, or not signed in — treat as non-admin.
      }
      if (live) setCanManage(admin);
    })();
    return () => {
      live = false;
    };
  }, [client, company]);

  /** The self-hosted hatch: navigate the document to the host's authorize URL. */
  async function connectNative(p: ConnectionProvider) {
    const { url } = await client.startConnection(p.id, company);
    // Unlike the Composio sign-in below (which opens a tab and survives), this
    // navigates the whole document away — taking the product tour's
    // in-memory step state with it. Arm a resume marker so the operator comes
    // back to the stop they left instead of the tour restarting from step 1
    // (issue #300). No-op when no tour is running, and deliberately after the
    // start call succeeds: a provider that isn't configured 400s here and
    // never navigates, so it must not leave a marker behind.
    armTourResume(scope);
    window.location.href = url;
  }

  /**
   * The hosted route: Composio runs the OAuth on its own side, so there is no
   * local callback to wait on. Open its connect URL in a tab and poll this
   * company's connection list until the toolkit flips, mirroring
   * `ComposioSection.signIn`.
   *
   * `busy` is cleared by the poll rather than by the caller — the connect is not
   * finished when the tab opens, and clearing early would offer a second Connect
   * for a sign-in already in flight.
   */
  async function connectComposio(p: ConnectionProvider, toolkit: string) {
    // A sign-in for this toolkit is already polling (it can have been started
    // from a different tile sharing the slug). Clear the flag we just set rather
    // than leaving this tile spinning on someone else's flow.
    if (pollTimers.current[toolkit] !== undefined) {
      setBusy((b) => (b === p.id ? null : b));
      return;
    }
    const { connectUrl } = await startComposioAuthorize(client, company, toolkit);
    // `noopener` keeps the Composio tab from reaching back through
    // `window.opener` — it is a third-party page carrying an OAuth flow, so it
    // stays. The cost is that the handle is ALWAYS null: with `noopener` (or
    // `noreferrer`) set, `window.open` returns null on success exactly as it
    // does when a popup is blocked.
    //
    // So a null check here cannot detect a blocked popup — it fires on every
    // successful open. This was reviewed as "handle a blocked popup", written
    // that way, and caught in manual testing: the tab opened and the operator
    // was told it had not. Detecting the block would mean dropping `noopener`
    // and trading a real security property for a nicer error, which is the
    // wrong trade on a tab we hand an OAuth URL to. `ComposioSection.signIn`
    // opens the same URL the same way and likewise does not check.
    window.open(connectUrl, "_blank", "noopener,noreferrer");
    toast.message(`Complete ${p.name} sign-in in the new tab.`);
    const deadline = Date.now() + 120_000;
    const poll = async () => {
      delete pollTimers.current[toolkit];
      if (Date.now() > deadline) {
        setBusy((b) => (b === p.id ? null : b));
        toast.message(`${p.name} sign-in timed out. Try again if it didn't complete.`);
        return;
      }
      try {
        const rows = await listComposioConnections(client, company);
        if (rows.some((r) => r.toolkit.toLowerCase() === toolkit.toLowerCase() && r.connected)) {
          setBusy((b) => (b === p.id ? null : b));
          toast.success(`Connected ${p.name}.`);
          // Re-read the host's reconciled view so the tile flips to Disconnect.
          await refresh();
          return;
        }
      } catch {
        // Ignore transient probe errors while the operator finishes sign-in.
      }
      pollTimers.current[toolkit] = window.setTimeout(() => void poll(), 2_000);
    };
    pollTimers.current[toolkit] = window.setTimeout(() => void poll(), 2_000);
  }

  async function connect(p: ConnectionProvider, route: ConnectRoute) {
    if (busy) return;
    setBusy(p.id);
    try {
      if (route.kind === "composio") {
        await connectComposio(p, route.toolkit);
        return;
      }
      if (route.kind === "native") {
        await connectNative(p);
        return;
      }
      // `managed` / `unavailable` render no Connect button, so reaching here
      // would be a rendering bug rather than an operator action.
      setBusy(null);
    } catch {
      toast.error(`Couldn't start the ${p.name} connection.`);
      setBusy(null);
    }
  }

  async function disconnect(p: ConnectionProvider) {
    if (busy) return;
    setBusy(p.id);
    try {
      await client.disconnectConnection(p.id, company);
      toast.success(`Disconnected ${p.name}.`);
      await refresh();
    } catch {
      toast.error(`Couldn't disconnect ${p.name}.`);
    } finally {
      setBusy(null);
    }
  }

  const connectedCount = Object.values(states).filter((s) => s.connected).length;
  // `attested` is a property of the *instance*, not of one provider: it means
  // this pod carries a platform-minted identity, so every connection is the
  // platform's to run. One row reporting it is therefore enough to say so once
  // at the top, rather than only in per-tile copy (issue #319).
  //
  // Read from the Composio status as well as the connection rows (issue #599).
  // `GET …/connections` only answers for providers the manifest declares, so a
  // tenant that declares none produced no rows at all — and this went false on a
  // host that is unambiguously platform-managed, which is how eleven tiles ended
  // up offering a Connect that could only 400.
  const platformManaged =
    load === "ready" &&
    (Object.values(states).some((s) => s.credentialSource === "attested") || attested);

  // One decision per tile, made once and used for both the button and the click,
  // so what is rendered and what is called can never disagree.
  const routes = useMemo(() => {
    const out = new Map<string, { state?: ConnectionState; route: ConnectRoute }>();
    for (const provider of CONNECTION_PROVIDERS) {
      const state = connectionStateFor(provider, states);
      // A tile the host said nothing about still inherits the instance-level
      // `attested` fact — it is a property of the pod, not of one provider.
      const effective =
        state ?? (platformManaged ? ({ credentialSource: "attested" } as const) : undefined);
      out.set(provider.id, { state, route: connectRoute(provider, effective, reach) });
    }
    return out;
  }, [states, reach, platformManaged]);

  return (
    <div className="flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-5xl space-y-6 px-4 py-6">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-1">
            <h2 className="text-2xl font-semibold tracking-tight">Connections</h2>
            <p className="text-sm text-muted-foreground">
              Wire in the accounts your company acts through. It only uses what you connect.
            </p>
          </div>
          {load === "ready" && connectedCount > 0 && (
            <Badge variant="secondary">{connectedCount} connected</Badge>
          )}
        </div>

        {load === "unavailable" && (
          <Alert>
            <Info className="size-4" />
            <AlertTitle>Connections aren&apos;t wired on this host yet</AlertTitle>
            <AlertDescription>
              The catalog below shows what your company can connect once the host exposes its OAuth
              endpoints. Connecting is disabled until then.
            </AlertDescription>
          </Alert>
        )}

        {platformManaged && (
          <Alert data-testid="connections-platform-managed">
            <ShieldCheck className="size-4" />
            <AlertTitle>Connections are managed by the platform</AlertTitle>
            <AlertDescription>
              This instance signs in with its own platform identity, so there is no provider key to
              register here and nothing stored on this instance. Connect a provider from the
              platform and it shows up here.
            </AlertDescription>
          </Alert>
        )}

        {!canManage && (
          <Alert data-testid="connections-read-only">
            <Info className="size-4" />
            <AlertTitle>Only an admin can change what this company connects through</AlertTitle>
            <AlertDescription>
              A connection belongs to the company — it is the account your agents act through — so
              an admin manages it. You can see everything that is wired here; ask an admin to add,
              change or remove one.
            </AlertDescription>
          </Alert>
        )}

        <McpServersSection client={client} company={company} canManage={canManage} />

        <InferenceSection client={client} company={company} canManage={canManage} />

        {/* The general answer sits above the Composio-specific one: this key
            authorizes every brokered surface, and the Composio token below is
            the escape hatch (issue #586). */}
        <CompanyCredentialCard
          client={client}
          company={company}
          canManage={canManage}
          onChanged={() => setCredentialGeneration((n) => n + 1)}
        />

        {/* Remounted on a credential change so its status is re-read: the tier
            it reports (`company` vs `attested` vs `none`) is downstream of the
            key that was just set, and a stale badge would tell the operator
            their change did not land. */}
        <ComposioSection
          key={credentialGeneration}
          client={client}
          company={company}
          canManage={canManage}
        />

        <ChannelsSection client={client} company={company} canManage={canManage} />

        {load === "ready" && (
          <Alert data-testid="connections-catalog-advisory">
            <Info className="size-4" />
            <AlertTitle>Agents reach these providers through Composio</AlertTitle>
            <AlertDescription>
              Connecting below records the account against this company, but the tool belt your
              agents actually run on is wired in the Composio section above — that is where a
              connection becomes something an agent can do. Connect Gmail, GitHub or Slack there
              first (issue #396).
            </AlertDescription>
          </Alert>
        )}

        {load === "loading" || !reachSettled ? (
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            {Array.from({ length: 6 }).map((_, i) => (
              <Skeleton key={i} className="h-28 rounded-xl" />
            ))}
          </div>
        ) : (
          CONNECTION_CATEGORY_ORDER.map((category) => {
            const providers = CONNECTION_PROVIDERS.filter((p) => p.category === category);
            if (providers.length === 0) return null;
            return (
              <section key={category} className="space-y-3">
                <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
                  {category}
                </h3>
                <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
                  {providers.map((p) => {
                    const { state, route } = routes.get(p.id) ?? { route: { kind: "unavailable" as const } };
                    return (
                      <ConnectionCard
                        key={p.id}
                        provider={p}
                        state={state}
                        route={route}
                        disabled={load === "unavailable" || !canManage}
                        busy={busy === p.id}
                        onConnect={() => void connect(p, route)}
                        onDisconnect={() => void disconnect(p)}
                      />
                    );
                  })}
                </div>
              </section>
            );
          })
        )}
      </div>
    </div>
  );
}

/**
 * One provider tile.
 *
 * The unconnected foot of the card renders whatever {@link connectRoute}
 * decided, so the button an operator sees is the call the click makes:
 *
 * - `native` — this host holds a registered provider application for it (or the
 *   company already stored a token): the self-hosted hatch. The Connect button,
 *   exactly as before.
 * - `composio` — the hosted route. Also a Connect button, but it opens
 *   Composio's own OAuth in a tab rather than navigating this document.
 * - `managed` — a platform identity runs connections for this instance and
 *   there is no Composio route; nothing to set up locally.
 * - `unavailable` — no route can succeed, so the tile says so rather than
 *   offering a button that 400s. This is the state issue #599 reports missing:
 *   every undeclared tile fell through to a Connect that could never work.
 *
 * The connected foot offers **Disconnect only when there is something local to
 * revoke** — i.e. `via` includes `native`. A Composio-only connection has no
 * disconnect route on the host at all (`/composio` exposes status, token,
 * authorize and connections, and nothing else), so a Disconnect button there
 * would call `…/connections/{id}/disconnect`, blank a secret that was never
 * set, report success and change nothing. Naming where the connection lives is
 * the honest answer until a Composio disconnect route exists.
 */
function ConnectionCard({
  provider,
  state,
  route,
  disabled,
  busy,
  onConnect,
  onDisconnect,
}: {
  provider: ConnectionProvider;
  state?: ConnectionState;
  route: ConnectRoute;
  disabled: boolean;
  busy: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
}) {
  const connected = Boolean(state?.connected);
  const managedByPlatform = route.kind === "managed";
  const noRoute = route.kind === "unavailable";
  // Which namespace actually backs this connection. `native` alone means the
  // credential sits in the host's catalog, which no agent tool reads yet — worth
  // distinguishing from a Composio connection, which is a live capability.
  const via = state?.via ?? [];
  const nativeOnly = connected && via.length > 0 && !via.includes("composio");
  // Only a native credential can be revoked from here; see the doc above.
  // An empty `via` is "this host predates the field" (it is optional on the
  // wire), not "Composio owns it" — withholding Disconnect there would strip
  // the control from every connection on an older host, so the affordance is
  // withheld only when the host affirmatively named Composio and nothing else.
  const canDisconnect = connected && (via.length === 0 || via.includes("native"));
  const unverified = state?.unverified === true;
  return (
    <Card className={cn(connected && "border-primary/30")}>
      <CardContent className="flex h-full flex-col gap-3 py-4">
        <div className="flex items-start gap-3">
          <Monogram provider={provider} />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <p className="truncate font-medium">{provider.name}</p>
              {connected && (
                <span className="inline-flex items-center gap-1 text-xs font-medium text-status-done-text">
                  <Check className="size-3" /> Connected
                </span>
              )}
            </div>
            <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
              {connected && state?.account ? state.account : provider.description}
            </p>
            {connected && via.length > 0 && (
              <p className="mt-0.5 text-xs text-muted-foreground" data-testid={`connection-via-${provider.id}`}>
                via {via.join(" + ")}
                {nativeOnly && " — stored here; agents use the Composio connection"}
              </p>
            )}
            {!connected && unverified && (
              <p className="mt-0.5 text-xs text-status-blocked-text">
                Couldn&apos;t check the Composio connection — state unknown.
              </p>
            )}
          </div>
        </div>
        <div className="mt-auto">
          {canDisconnect ? (
            <Button variant="outline" size="sm" className="w-full" disabled={busy} onClick={onDisconnect}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : null}
              Disconnect
            </Button>
          ) : connected ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid={`connection-composio-managed-${provider.id}`}
            >
              Connected through Composio — manage it in the Composio section above.
            </p>
          ) : managedByPlatform ? (
            <p
              className="inline-flex items-center gap-1 text-xs text-status-done-text"
              data-testid={`connection-managed-${provider.id}`}
            >
              <ShieldCheck className="size-3" /> Managed by the platform — nothing to set up here
            </p>
          ) : noRoute ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid={`connection-unavailable-${provider.id}`}
            >
              Not available on this host.
            </p>
          ) : (
            <Button
              variant={disabled ? "outline" : "default"}
              size="sm"
              className="w-full"
              disabled={disabled || busy}
              onClick={onConnect}
            >
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Plug className="size-4" />}
              Connect
            </Button>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function Monogram({ provider }: { provider: ConnectionProvider }) {
  const label = provider.glyph ?? provider.name.charAt(0);
  return (
    <div
      className="flex size-10 shrink-0 items-center justify-center rounded-lg text-sm font-semibold text-white"
      style={{ backgroundColor: provider.color }}
      aria-hidden
    >
      {label}
    </div>
  );
}
