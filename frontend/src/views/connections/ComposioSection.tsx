import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  KeyRound,
  Loader2,
  LogIn,
  Plug,
  Save,
  Search,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import {
  getComposioStatus,
  listComposioConnections,
  setComposioToken,
  startComposioAuthorize,
  type ComposioStatus,
} from "@/api/composio";
import { ApiError } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import {
  availableCategories,
  buildProviderRows,
  catalogWarning,
  permissionHint,
  toolkitLabel,
  visibleProviderRows,
  type ProviderCategory,
  type ProviderRow,
} from "@/lib/composio-catalog";

/**
 * A provider's branded logo, degrading to a monogram.
 *
 * The URL is best-effort by construction: the backend publishes one for most
 * catalog entries, and every other tile falls back to a slug-derived guess at
 * Composio's logo CDN. A guess that 404s must look like a plain tile, never a
 * broken-image glyph — so the error is caught and swapped rather than left to
 * the browser.
 */
function ProviderLogo({ row }: { row: ProviderRow }) {
  const [failed, setFailed] = useState(false);
  // A company can repoint at a different backend, which re-keys the logo. Reset
  // on URL change so one dead image does not poison the slot for good.
  useEffect(() => setFailed(false), [row.logoUrl]);

  if (failed) {
    return (
      <span
        aria-hidden="true"
        className="flex size-8 items-center justify-center rounded-lg bg-muted text-xs font-semibold text-muted-foreground"
      >
        {row.label.charAt(0).toUpperCase()}
      </span>
    );
  }
  return (
    <img
      src={row.logoUrl}
      alt=""
      aria-hidden="true"
      loading="lazy"
      className="size-8 rounded-lg object-contain"
      onError={() => setFailed(true)}
    />
  );
}

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /**
   * Whether this viewer may change what the company connects through (issue
   * #403) — the credential its agents present, and which provider accounts
   * they act through.
   *
   * **Courtesy, not enforcement.** The host refuses both writes with a 403
   * whatever this says. What it prevents is offering a Sign in button that
   * cannot complete, or a token field whose Save is refused only after the
   * operator has already pasted a live credential into it.
   */
  canManage: boolean;
}

/**
 * Per-tenant Composio connection management (issue #110, Cell D).
 *
 * Two layers, and they are independent:
 *
 * 1. **Which credential this company reaches Composio with**, reported as
 *    `credentialSource`:
 *    - `attested` (hosted) — the instance already holds a platform identity, so
 *      there is nothing to paste and nothing stored here. A company that wants
 *      to use its OWN Composio account can still override.
 *    - `company` (issue #586) — this company's own TinyHumans credential, set
 *      by its admin. Composio is brokered through it, so there is nothing to
 *      paste here either: the one key already authorizes this. The override
 *      still exists for a company that wants its own Composio account.
 *    - `static` — a Composio token this company pasted, or a static instance key.
 *    - `none` — no credential can be obtained, so there is nothing to authorize
 *      against and agents get no Composio tools.
 * 2. **Which providers are connected**, via a per-provider OAuth "Sign in" list
 *    driven by the granted toolkits. Composio runs the hosted OAuth; we open it
 *    in a tab and poll until the toolkit reports connected.
 *
 * The pasted token is WRITE-ONLY: stored and never shown again. A set/clear
 * takes effect on the agents' next turn, no restart. Hidden entirely when the
 * feature is not in the build.
 */
export function ComposioSection({ client, company, canManage }: Props) {
  const [load, setLoad] = useState<"loading" | "ready" | "unavailable">("loading");
  const [status, setStatus] = useState<ComposioStatus | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState<"save" | "clear" | null>(null);
  // toolkit slug -> connected. Absent = unknown / not yet fetched.
  const [connected, setConnected] = useState<Record<string, boolean>>({});
  // toolkit slug currently mid-sign-in (open tab + poll).
  const [signingIn, setSigningIn] = useState<string | null>(null);
  // Only meaningful in the attested state, where the paste card is an override
  // rather than the way in.
  const [showOverride, setShowOverride] = useState(false);
  // Open mode only: a slug typed into the "connect by slug" field. It is now an
  // escape hatch rather than the way to reach the tail — the rows above are the
  // backend's real catalog — but it still earns its place: it is the only way to
  // connect a provider when the catalog could not be fetched, and the only way
  // to reach one the catalog happens to omit (issue #397).
  const [otherToolkit, setOtherToolkit] = useState("");
  // Slugs connected through that field this session, so they get a row of their
  // own instead of vanishing after a successful sign-in.
  const [extraToolkits, setExtraToolkits] = useState<string[]>([]);
  // Provider search. In open mode the host now hands over the backend's real
  // catalog — roughly a hundred toolkits — so finding one by scrolling stopped
  // being reasonable (issue #397).
  const [query, setQuery] = useState("");
  // The selected category chip. Composed with the search rather than replacing
  // it, so an operator can look for "invoices" inside Platform (issue #600).
  const [category, setCategory] = useState<ProviderCategory>("All");

  const requestGeneration = useRef(0);
  const pollTimers = useRef<Record<string, number>>({});

  // Clear any in-flight poll timers on unmount / company switch.
  useEffect(() => {
    const timers = pollTimers.current;
    return () => {
      Object.values(timers).forEach((id) => window.clearTimeout(id));
      pollTimers.current = {};
    };
  }, [company]);

  const refreshConnections = useCallback(
    async (s: ComposioStatus) => {
      // Listing connections needs *a* credential — any tier. With none there is
      // nothing to authorize against, and the call 409s. Keyed off the
      // EFFECTIVE list, not the manifest one: in open mode the manifest list is
      // empty precisely because everything is allowed (issue #397).
      if (s.credentialSource === "none" || s.effectiveToolkits.length === 0) {
        setConnected({});
        return;
      }
      try {
        const rows = await listComposioConnections(client, company);
        setConnected(Object.fromEntries(rows.map((r) => [r.toolkit.toLowerCase(), r.connected])));
      } catch {
        // Non-fatal: leave the provider rows in their "sign in" state.
        setConnected({});
      }
    },
    [client, company],
  );

  const refresh = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const s = await getComposioStatus(client, company);
      if (generation !== requestGeneration.current) return;
      setStatus(s);
      // Hide the whole section when the feature is not compiled into this build.
      setLoad(s.inBuild ? "ready" : "unavailable");
      if (s.inBuild) void refreshConnections(s);
    } catch {
      if (generation !== requestGeneration.current) return;
      setLoad("unavailable");
    }
  }, [client, company, refreshConnections]);

  useEffect(() => {
    setStatus(null);
    setConnected({});
    setShowOverride(false);
    setOtherToolkit("");
    setExtraToolkits([]);
    setQuery("");
    setCategory("All");
    setLoad("loading");
    void refresh();
  }, [refresh]);

  async function save() {
    if (!token.trim()) return;
    setBusy("save");
    try {
      const res = await setComposioToken(client, company, token.trim());
      setStatus(res.status);
      setToken("");
      toast.success(res.note);
      void refreshConnections(res.status);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "Could not save the token.");
    } finally {
      setBusy(null);
    }
  }

  async function clear() {
    setBusy("clear");
    try {
      const res = await setComposioToken(client, company, "");
      setStatus(res.status);
      setToken("");
      setConnected({});
      // Clearing an override falls back to whatever tier remains — re-probe
      // rather than assuming there is no credential left.
      toast.success("Composio token cleared.");
      void refreshConnections(res.status);
    } catch (err) {
      toast.error(err instanceof ApiError ? err.message : "Could not clear the token.");
    } finally {
      setBusy(null);
    }
  }

  // Per-provider OAuth: open Composio's hosted connect URL in a new tab, then
  // poll the connection list every 2s up to ~2 minutes until this toolkit flips
  // to connected (Composio completes the OAuth on its side — no local callback).
  async function signIn(toolkit: string) {
    // Guard the shared flag AND a poll already in flight for this toolkit.
    if (signingIn || pollTimers.current[toolkit] !== undefined) return;
    setSigningIn(toolkit);
    const label = toolkitLabel(toolkit);
    try {
      const { connectUrl } = await startComposioAuthorize(client, company, toolkit);
      window.open(connectUrl, "_blank", "noopener,noreferrer");
      toast.message(`Complete ${label} sign-in in the new tab.`);
      const deadline = Date.now() + 120_000;
      const poll = async () => {
        delete pollTimers.current[toolkit];
        if (Date.now() > deadline) {
          setSigningIn((t) => (t === toolkit ? null : t));
          toast.message(`${label} sign-in timed out. Try again if it didn't complete.`);
          return;
        }
        try {
          const rows = await listComposioConnections(client, company);
          const map = Object.fromEntries(rows.map((r) => [r.toolkit.toLowerCase(), r.connected]));
          setConnected(map);
          if (map[toolkit.toLowerCase()]) {
            setSigningIn((t) => (t === toolkit ? null : t));
            toast.success(`Connected ${label}.`);
            return;
          }
        } catch {
          // Ignore transient probe errors while the operator finishes sign-in.
        }
        pollTimers.current[toolkit] = window.setTimeout(() => void poll(), 2_000);
      };
      pollTimers.current[toolkit] = window.setTimeout(() => void poll(), 2_000);
    } catch (err) {
      setSigningIn((t) => (t === toolkit ? null : t));
      toast.error(err instanceof ApiError ? err.message : `Couldn't start ${label} sign-in.`);
    }
  }

  // Ordered once per status/connection change: connected first, then the
  // handful everyone reaches for, then the tail alphabetically. Ordering,
  // bucketing and filtering live in `@/lib/composio-catalog` so they are
  // testable without a document — see `vitest.config.ts` on where the line sits.
  //
  // Built from `effectiveCatalog`, not `effectiveToolkits`: the slug list is
  // still the wire contract every host call uses, but it is the catalog entries
  // that carry the name, description, logo and categories this grid is made of
  // (issue #600).
  const rows = useMemo(
    () => buildProviderRows(status?.effectiveCatalog ?? [], extraToolkits, connected),
    [status?.effectiveCatalog, extraToolkits, connected],
  );
  const categories = useMemo(() => availableCategories(rows), [rows]);
  const visible = useMemo(
    () => visibleProviderRows(rows, category, query),
    [rows, category, query],
  );
  const degraded = status ? catalogWarning(status) : null;

  // A chip can go away under the operator — a company narrows its manifest, or
  // a refresh returns a shorter catalog. Falling back to All beats leaving them
  // staring at an empty grid filtered by a bucket that no longer exists.
  useEffect(() => {
    if (!categories.includes(category)) setCategory("All");
  }, [categories, category]);

  if (load === "unavailable") return null;

  const attested = status?.credentialSource === "attested";
  // The company's own key already authorizes Composio (issue #586), so this
  // reads like `attested` everywhere the question is "is there anything to
  // paste?" — the difference is whose identity it is, which the copy states.
  const companyKey = status?.credentialSource === "company";
  const byoToken = status?.credentialSource === "static";
  // No credential of any tier: there is nothing to authorize a provider against.
  const noCredential = status?.credentialSource === "none";
  // Issue #397: gate on the EFFECTIVE list. The old gate read
  // `toolkits.length > 0`, and an empty manifest list means "defer to the
  // backend allowlist" — allow everything — so the one value meaning *every
  // provider* rendered *no* providers on 19 of 20 shipped templates.
  const showProviders = load === "ready" && status !== null && status.effectiveToolkits.length > 0;
  const openMode = status?.openMode === true;
  // In the attested state the paste card is a deliberate override; everywhere
  // else it is the only way to connect, so it is always on screen.
  // Issue #403: the credential card is an admin's. A member still sees the
  // status line above ("token set" / "linked via cluster identity"), which is
  // what tells them why their agents can reach Gmail; what they do not get is a
  // field that invites them to paste a credential the host will refuse.
  // A company already brokered through its own key is in the same position as an
  // attested one: the paste card is a deliberate override, not the way in.
  const credentialed = attested || companyKey;
  const showTokenCard = canManage && (!credentialed || showOverride || byoToken);

  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2">
        <Plug className="size-4 text-muted-foreground" />
        <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Composio (Gmail / Slack / GitHub)
        </h3>
      </div>
      <p className="text-sm text-muted-foreground">
        {!canManage
          ? "Your agents reach Gmail, Slack & GitHub through Composio. Which account they act through belongs to the company, so an admin manages it — this is what is wired today."
          : attested
          ? "Give your agents Gmail, Slack & GitHub via Composio. This company is linked through this instance's own cluster identity — there is no key to copy and nothing stored here. Sign in per provider below."
          : companyKey
          ? "Give your agents Gmail, Slack & GitHub via Composio. This company's own TinyHumans credential already authorizes it — there is no separate Composio token to paste and no provider app to register. Sign in per provider below; every agent in the company can then use what you connect."
          : "Give your agents Gmail, Slack & GitHub via Composio. Paste this company's Composio OAuth token — it is the identity the backend bills and isolates, stored securely and never shown again — then sign in per provider below. A change takes effect on the next turn, no restart."}
      </p>

      {load === "loading" ? (
        <Skeleton className="h-32 rounded-xl" />
      ) : (
        <>
          {status && (
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={status.granted ? "secondary" : "outline"}>
                {status.granted ? "granted" : "not granted"}
              </Badge>
              {attested ? (
                <span className="inline-flex items-center gap-1 text-xs text-status-done-text">
                  <ShieldCheck className="size-3" /> Linked via cluster identity — nothing stored
                </span>
              ) : companyKey ? (
                <span className="inline-flex items-center gap-1 text-xs text-status-done-text">
                  <ShieldCheck className="size-3" /> Linked via this company&apos;s own credential
                </span>
              ) : byoToken ? (
                <span className="inline-flex items-center gap-1 text-xs text-status-done-text">
                  <Check className="size-3" /> token set
                </span>
              ) : (
                <span className="text-xs text-muted-foreground">not connected</span>
              )}
            </div>
          )}

          {status && !status.granted && (
            <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
              This company does not grant the <span className="font-mono">composio</span> tool
              namespace, so agents will not receive Composio tools even once connected. Add
              <span className="font-mono"> composio</span> to the company&apos;s tool grants first.
            </p>
          )}

          {showProviders && status && (
            <Card>
              <CardContent className="space-y-2 py-4">
                <p className="text-xs font-medium text-muted-foreground">Sign in per provider</p>
                {/* Branches on `canManage` like the intro above it. A member has
                    neither the credential field nor the paste card, so telling
                    them to "set the credential or paste a token below" points at
                    controls that are not on their screen. */}
                {noCredential && (
                  <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                    {canManage
                      ? "No credential is available for this company yet, so there is nothing to authorize against. Set the company's TinyHumans credential — one key authorizes the providers the platform brokers — or paste a Composio token below to use a Composio account of your own."
                      : "No credential is available for this company yet, so there is nothing to authorize against. An admin has to set the company's credential before providers can be connected."}
                  </p>
                )}
                {degraded ? (
                  // The host could not read Composio's catalog. Say so — a
                  // built-in list rendered like a fetched one is a claim we
                  // cannot back (issue #397).
                  <p className="flex items-start gap-2 rounded-md bg-status-blocked-soft p-2 text-xs text-status-blocked-text">
                    <AlertTriangle className="mt-px size-3 shrink-0" />
                    <span>{degraded}</span>
                  </p>
                ) : (
                  openMode && (
                    <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                      This company allows <span className="font-medium">any</span> provider Composio
                      offers — {rows.length} in total, connected first. Filter by category or
                      search by name, slug, or what a provider does.
                    </p>
                  )
                )}
                {rows.length > 8 && (
                  <div className="relative">
                    <Search className="absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                    <Input
                      aria-label="Search providers"
                      autoComplete="off"
                      className="pl-7"
                      placeholder={`Search ${rows.length} providers by name or what they do…`}
                      value={query}
                      onChange={(e) => setQuery(e.target.value)}
                    />
                  </div>
                )}
                {categories.length > 2 && (
                  // Chips only earn their row when there is more than one real
                  // bucket to choose between — `availableCategories` always
                  // includes "All", so two means one bucket, which is not a
                  // choice. A two-provider manifest list gets the grid without
                  // a filter that can only ever do nothing.
                  <div
                    role="group"
                    aria-label="Filter providers by category"
                    className="flex gap-1.5 overflow-x-auto pb-1"
                  >
                    {categories.map((c) => (
                      <Button
                        key={c}
                        type="button"
                        size="sm"
                        variant={c === category ? "secondary" : "ghost"}
                        aria-pressed={c === category}
                        className={cn(
                          "h-7 shrink-0 rounded-full px-3 text-xs",
                          c !== category && "text-muted-foreground",
                        )}
                        onClick={() => setCategory(c)}
                      >
                        {c}
                      </Button>
                    ))}
                  </div>
                )}
                {/*
                  A dense tile grid, not rows (issue #600). 123 full-width rows
                  are unreadable at any scroll depth, which is why the old list
                  hid all but twelve behind a "show all" button — the cut was a
                  workaround for the layout. Compact branded tiles fit the whole
                  catalog on a screen or two, so the button is gone rather than
                  relabelled.
                */}
                <ul
                  className="grid gap-2"
                  style={{
                    // Uniform rows so a grid of 123 tiles reads as a grid and
                    // not as ragged masonry — the tile is a fixed slot, and the
                    // label clamps to fit it.
                    gridTemplateColumns: "repeat(auto-fill, minmax(8.5rem, 1fr))",
                    gridAutoRows: "5.5rem",
                  }}
                >
                  {visible.map((row) => {
                    const isSigningIn = signingIn === row.slug;
                    // The whole tile is the affordance. An 8.5rem tile has no
                    // room for a label AND a button, and a tile that looks
                    // clickable but is not would be worse than either.
                    const actionable = canManage && !row.connected;
                    // Named `state`, not `status` — `status` is the component's
                    // ComposioStatus, and shadowing it here would be a quiet
                    // trap for the next edit.
                    const state = row.connected
                      ? "connected"
                      : isSigningIn
                        ? "signing in"
                        : "not connected";
                    const shell = cn(
                      "flex size-full flex-col items-start justify-between gap-1 rounded-lg border p-2.5 text-left",
                      row.connected
                        ? "border-status-done/30 bg-status-done-soft"
                        : "border-border bg-card",
                    );
                    const body = (
                      <>
                        <div className="flex w-full items-start justify-between gap-1">
                          <ProviderLogo row={row} />
                          {row.connected ? (
                            <Check className="size-3.5 shrink-0 text-status-done-text" />
                          ) : isSigningIn ? (
                            <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
                          ) : actionable ? (
                            <LogIn className="size-3.5 shrink-0 text-muted-foreground" />
                          ) : null}
                        </div>
                        <div className="w-full min-w-0">
                          <span className="line-clamp-2 text-xs leading-tight font-medium">
                            {row.label}
                          </span>
                          <span
                            className={cn(
                              "block text-3xs",
                              row.connected
                                ? "text-status-done-text"
                                : "text-muted-foreground",
                            )}
                          >
                            {state}
                          </span>
                        </div>
                      </>
                    );
                    return (
                      <li key={row.slug} className="min-w-0">
                        {actionable ? (
                          <button
                            type="button"
                            disabled={signingIn !== null || noCredential}
                            onClick={() => void signIn(row.slug)}
                            title={row.description || undefined}
                            aria-label={`Sign in to ${row.label}. ${state}. Authorises: ${permissionHint(row.category)}.`}
                            className={cn(
                              shell,
                              "transition-colors hover:border-foreground/20 hover:bg-accent",
                              "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                              "disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:border-border disabled:hover:bg-card",
                            )}
                          >
                            {body}
                          </button>
                        ) : (
                          // Connected, or a viewer who cannot manage: there is
                          // nothing to click. Rendered as a div rather than a
                          // disabled button so it stays in the reading order —
                          // "Gmail, connected" is exactly what a member opened
                          // this panel to learn, and a disabled button is
                          // unfocusable.
                          <div
                            className={shell}
                            title={row.description || undefined}
                            aria-label={`${row.label}. ${state}.`}
                          >
                            {body}
                          </div>
                        )}
                      </li>
                    );
                  })}
                </ul>
                {visible.length === 0 && (
                  <p className="py-2 text-xs text-muted-foreground">
                    {query.trim() !== "" ? (
                      <>
                        No provider matches “{query.trim()}”
                        {category !== "All" && <> in {category}</>}. Composio&apos;s slug may differ
                        from the product name — try another category, or connect it by slug below.
                      </>
                    ) : (
                      <>No provider in {category}.</>
                    )}
                  </p>
                )}
                {openMode && canManage && (
                  <div className="flex items-end gap-2 border-t border-border pt-3">
                    <div className="flex-1 space-y-1">
                      <Label htmlFor="composio-other-toolkit" className="text-xs">
                        Connect by slug
                      </Label>
                      <Input
                        id="composio-other-toolkit"
                        autoComplete="off"
                        placeholder="composio toolkit slug, e.g. hubspot"
                        value={otherToolkit}
                        disabled={noCredential}
                        onChange={(e) => setOtherToolkit(e.target.value)}
                      />
                    </div>
                    <Button
                      variant="outline"
                      disabled={signingIn !== null || noCredential || !otherToolkit.trim()}
                      onClick={() => {
                        const slug = otherToolkit.trim().toLowerCase();
                        setOtherToolkit("");
                        setExtraToolkits((t) => (t.includes(slug) ? t : [...t, slug]));
                        void signIn(slug);
                      }}
                    >
                      <LogIn className="size-4" />
                      Sign in
                    </Button>
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          {/* Gated on `credentialed`, not `attested`: a company brokered through
              its own TinyHumans key is equally "already credentialled", so it
              equally needs a way back to the BYO card. Gating this on `attested`
              alone left a company-key admin with the paste card hidden and no
              control to reveal it — the override became unreachable. */}
          {canManage && credentialed && !showTokenCard && (
            <Button variant="outline" size="sm" onClick={() => setShowOverride(true)}>
              <KeyRound className="size-4" />
              Use your own Composio account instead
            </Button>
          )}

          {showTokenCard && (
            <Card>
              <CardContent className="space-y-4 py-4">
                {/* The explainer has to name what the token would displace,
                    and that differs by tier: an attested company falls back to
                    the pod's cluster identity, a company-key one falls back to
                    its own credential. Saying "cluster identity" to the latter
                    would describe a fallback it does not have. */}
                {companyKey ? (
                  <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                    Optional. This company&apos;s own TinyHumans credential already authorizes
                    Composio. A token set here replaces it for Composio only — use it when the
                    company has a separate Composio account. Clear it to go back to the company
                    credential.
                  </p>
                ) : attested ? (
                  <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
                    Optional. A token set here overrides the instance identity for this company only
                    — use it when the company has its own Composio account. Clear it to go back to
                    the cluster identity.
                  </p>
                ) : null}
                <div className="space-y-1">
                  <Label htmlFor="composio-token" className="text-xs">
                    Composio token {byoToken ? "— set (paste a new value to rotate)" : ""}
                  </Label>
                  <Input
                    id="composio-token"
                    type="password"
                    autoComplete="off"
                    placeholder="paste the company's Composio OAuth token"
                    value={token}
                    onChange={(e) => setToken(e.target.value)}
                  />
                  {status && (
                    <p className="truncate text-xs text-muted-foreground">{status.backendUrl}</p>
                  )}
                </div>

                <div className="flex items-center gap-2">
                  <Button disabled={busy !== null || !token.trim()} onClick={() => void save()}>
                    {busy === "save" ? (
                      <Loader2 className="size-4 animate-spin" />
                    ) : (
                      <Save className="size-4" />
                    )}
                    Save token
                  </Button>
                  {byoToken && (
                    <Button variant="outline" disabled={busy !== null} onClick={() => void clear()}>
                      {busy === "clear" ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Trash2 className="size-4" />
                      )}
                      Clear
                    </Button>
                  )}
                  <KeyRound className="ml-auto size-4 text-muted-foreground" />
                </div>
              </CardContent>
            </Card>
          )}
        </>
      )}
    </section>
  );
}
