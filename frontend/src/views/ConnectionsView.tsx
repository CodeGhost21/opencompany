import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Info, ShieldCheck } from "lucide-react";
import { toast } from "sonner";

import { me as fetchMe } from "@/api/auth";
import type { OpenCompanyClient } from "@/api/client";
import {
  getComposioStatus,
  listComposioConnections,
  startComposioAuthorize,
  type ComposioStatus,
} from "@/api/composio";
import type { ConnectionState } from "@/api/types";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { catalogWarning } from "@/lib/composio-catalog";
import { type ComposioReach } from "@/lib/connections";
import { buildGridProviders, type GridProvider } from "@/lib/provider-grid";
import { armTourResume } from "@/tour/state";
import { InferenceSection } from "@/views/connections/InferenceSection";
import { McpServersSection } from "@/views/connections/McpServersSection";
import { CompanyCredentialCard } from "@/views/connections/CompanyCredentialCard";
import { ComposioSection } from "@/views/connections/ComposioSection";
import { ProvidersSection } from "@/views/connections/ProvidersSection";
import { RepositoriesCard } from "@/views/connections/RepositoriesCard";
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
  // The host's Composio answer, or `null` while unknown / not reachable.
  //
  // The whole status, not just the routing facts it used to be narrowed to: the
  // page's one provider grid is built from `effectiveCatalog` (issue #582), and
  // the catalog's honesty markers (`catalogSource`, `catalogNotice`) travel with
  // it. Narrowing here and re-fetching the same call elsewhere for the rest is
  // how the page ended up with two lists.
  const [status, setStatus] = useState<ComposioStatus | null>(null);
  // Slugs connected through the by-slug escape hatch this session, so they keep
  // a tile instead of vanishing after a successful sign-in (issue #397).
  const [extraToolkits, setExtraToolkits] = useState<string[]>([]);
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
        const probed = await Promise.race([
          getComposioStatus(client, company),
          new Promise<null>((resolve) => window.setTimeout(() => resolve(null), COMPOSIO_PROBE_TIMEOUT_MS)),
        ]);
        if (!live) return;
        setStatus(probed);
        setAttested(probed?.credentialSource === "attested");
      } catch {
        // No Composio surface on this host — not an error for this page.
        if (live) {
          setStatus(null);
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
  async function connectNative(p: GridProvider) {
    const { url } = await client.startConnection(p.providerId, company);
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
  async function connectComposio(p: { providerId: string; label: string }, toolkit: string) {
    // A sign-in for this toolkit is already polling (it can have been started
    // from a different tile sharing the slug). Clear the flag we just set rather
    // than leaving this tile spinning on someone else's flow.
    if (pollTimers.current[toolkit] !== undefined) {
      setBusy((b) => (b === p.providerId ? null : b));
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
    toast.message(`Complete ${p.label} sign-in in the new tab.`);
    const deadline = Date.now() + 120_000;
    const poll = async () => {
      delete pollTimers.current[toolkit];
      if (Date.now() > deadline) {
        setBusy((b) => (b === p.providerId ? null : b));
        toast.message(`${p.label} sign-in timed out. Try again if it didn't complete.`);
        return;
      }
      try {
        const rows = await listComposioConnections(client, company);
        if (rows.some((r) => r.toolkit.toLowerCase() === toolkit.toLowerCase() && r.connected)) {
          setBusy((b) => (b === p.providerId ? null : b));
          toast.success(`Connected ${p.label}.`);
          // Re-read the host's reconciled view so the tile flips to connected.
          // This is the page's only status source now (issue #582), so one
          // refresh updates everything that speaks about this provider — there
          // is no second list left to go stale behind it.
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

  async function connect(p: GridProvider) {
    if (busy) return;
    setBusy(p.providerId);
    try {
      if (p.route.kind === "composio") {
        await connectComposio(p, p.route.toolkit);
        return;
      }
      if (p.route.kind === "native") {
        await connectNative(p);
        return;
      }
      // `managed` / `unavailable` render no Connect button, so reaching here
      // would be a rendering bug rather than an operator action.
      setBusy(null);
    } catch {
      toast.error(`Couldn't start the ${p.label} connection.`);
      setBusy(null);
    }
  }

  /**
   * Sign in to a toolkit typed into the by-slug hatch.
   *
   * It has no tile until the host reports it, so it is added to `extraToolkits`
   * first — `buildGridProviders` gives it one from local metadata, and the
   * refresh after a successful poll then fills in the real status.
   */
  async function connectSlug(slug: string) {
    if (busy || !slug) return;
    setExtraToolkits((t) => (t.includes(slug) ? t : [...t, slug]));
    setBusy(slug);
    try {
      await connectComposio({ providerId: slug, label: slug }, slug);
    } catch {
      toast.error(`Couldn't start the ${slug} connection.`);
      setBusy(null);
    }
  }

  /**
   * Release a connection this host actually holds.
   *
   * Only offered for a tile whose `via` includes `native` (see
   * `GridProvider.canDisconnect`): this route blanks the host's own
   * `oauth/{provider}` secret, and no host route can release a Composio
   * connection. Offering it on a Composio-only tile would report success and
   * leave the tile connected on the next refresh.
   */
  async function disconnect(p: GridProvider) {
    if (busy) return;
    setBusy(p.providerId);
    try {
      await client.disconnectConnection(p.providerId, company);
      toast.success(`Disconnected ${p.label}.`);
      await refresh();
    } catch {
      toast.error(`Couldn't disconnect ${p.label}.`);
    } finally {
      setBusy(null);
    }
  }

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

  // The routing facts, narrowed out of the status the page already holds.
  const reach: ComposioReach | null = useMemo(
    () =>
      status
        ? {
            inBuild: status.inBuild,
            granted: status.granted,
            hasCredential: status.credentialSource !== "none",
            openMode: status.openMode,
            effectiveToolkits: status.effectiveToolkits,
          }
        : null,
    [status],
  );

  // The page's one provider list. Status, route and metadata resolved together,
  // once, so nothing on this screen can answer differently from anything else
  // on it (issue #582).
  const providers = useMemo(
    () =>
      buildGridProviders(
        status?.effectiveCatalog ?? [],
        extraToolkits,
        states,
        reach,
        platformManaged,
      ),
    [status?.effectiveCatalog, extraToolkits, states, reach, platformManaged],
  );

  // Counted off the rendered grid, not off the raw host rows. The badge used to
  // count every row `GET …/connections` returned while the grid below drew only
  // the eleven tiles the console had metadata for — so the header could read "3
  // connected" above a grid showing none of them (issue #582).
  const connectedCount = providers.filter((p) => p.connected).length;

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
          onChanged={() => setCredentialGeneration((n) => n + 1)}
        />

        <ChannelsSection client={client} company={company} canManage={canManage} />

        <RepositoriesCard client={client} company={company} canManage={canManage} />

        {/* The page's one provider list (issue #582). It used to be two — this
            grid and a categorised grid of eleven hardcoded tiles below it — and
            they disagreed by construction, so a provider could show as connected
            in one and offer a Connect button in the other on the same screen. */}
        <ProvidersSection
          providers={providers}
          canManage={canManage && load !== "unavailable"}
          busy={busy}
          noCredential={status?.credentialSource === "none"}
          granted={status?.granted !== false}
          openMode={status?.openMode === true}
          degraded={status ? catalogWarning(status) : null}
          loading={load === "loading" || !reachSettled}
          onConnect={(p) => void connect(p)}
          onDisconnect={(p) => void disconnect(p)}
          onConnectSlug={(slug) => void connectSlug(slug)}
        />
      </div>
    </div>
  );
}
