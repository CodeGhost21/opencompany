import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, Loader2, LogIn, Search, Unplug } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  availableCategories,
  permissionHint,
  visibleProviderRows,
  type ProviderCategory,
} from "@/lib/composio-catalog";
import type { GridProvider } from "@/lib/provider-grid";
import { cn } from "@/lib/utils";

interface Props {
  /** Every provider this company can reach, connected first (issue #582). */
  providers: GridProvider[];
  /** Whether this viewer may change what the company connects through (#403). */
  canManage: boolean;
  /** The `providerId` whose connect/disconnect is in flight, if any. */
  busy: string | null;
  /** Nothing to authorize against — no credential of any tier resolves. */
  noCredential: boolean;
  /**
   * Whether the company grants the `composio` tool namespace.
   *
   * A caveat, never a gate (issue #582). A connection made without it is real —
   * the account is linked and the tile is honest to say so — but no agent
   * receives its tools until the grant exists, and the operator has to be told
   * that where they can see the connected badge, not only in a section above.
   */
  granted: boolean;
  /** Open mode: any slug the backend permits is reachable, so offer the hatch. */
  openMode: boolean;
  /** Why the catalog on screen is not the backend's real one, when it is not. */
  degraded: string | null;
  /** The host has not answered yet. */
  loading: boolean;
  onConnect: (provider: GridProvider) => void;
  onDisconnect: (provider: GridProvider) => void;
  onConnectSlug: (slug: string) => void;
}

/**
 * The Connections page's one provider grid (issue #582).
 *
 * The page used to carry two: this tile grid (then inside `ComposioSection`,
 * fed by `GET …/composio/connections`) and a categorised grid of eleven
 * hardcoded tiles fed by `GET …/connections`. They disagreed — routinely and by
 * construction, not as a race — so Gmail could show "connected" in one and an
 * actionable Connect button in the other, on one screen.
 *
 * What survives is this grid, because it is the one that was already right: the
 * providers come from the backend's live catalog rather than a list compiled
 * into the console, and #600 gave it the names, logos, descriptions and
 * categories that made the hardcoded tiles worth keeping in the first place.
 * What it gained is the other grid's job — the reconciled status from
 * `GET …/connections`, the native self-hosted route, and Disconnect — so the
 * page has one list and that list has one answer.
 *
 * Presentational on purpose: it owns the filter chips and the search box, and
 * nothing else. Connected state, the route decision and both writes belong to
 * `ConnectionsView`, which holds the host state they are derived from. A tile
 * that decided its own status is how the page came to have two of them.
 */
export function ProvidersSection({
  providers,
  canManage,
  busy,
  noCredential,
  granted,
  openMode,
  degraded,
  loading,
  onConnect,
  onDisconnect,
  onConnectSlug,
}: Props) {
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<ProviderCategory>("All");
  const [otherToolkit, setOtherToolkit] = useState("");

  const categories = useMemo(() => availableCategories(providers), [providers]);
  const visible = useMemo(
    () => visibleProviderRows(providers, category, query),
    [providers, category, query],
  );

  // A chip can go away under the operator — a company narrows its manifest, or a
  // refresh returns a shorter catalog. Falling back to All beats leaving them
  // staring at an empty grid filtered by a bucket that no longer exists.
  useEffect(() => {
    if (!categories.includes(category)) setCategory("All");
  }, [categories, category]);

  const connectedCount = providers.filter((p) => p.connected).length;

  if (loading) {
    return (
      <section className="space-y-3">
        <SectionHeading count={null} />
        <Skeleton className="h-64 rounded-xl" />
      </section>
    );
  }

  return (
    <section className="space-y-3">
      <SectionHeading count={connectedCount} />
      <Card>
        <CardContent className="space-y-2 py-4">
          {noCredential && (
            <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
              {canManage
                ? "No credential is available for this company yet, so there is nothing to authorize against. Set the company's TinyHumans credential — one key authorizes the providers the platform brokers — or paste a Composio token below to use a Composio account of your own."
                : "No credential is available for this company yet, so there is nothing to authorize against. An admin has to set the company's credential before providers can be connected."}
            </p>
          )}
          {!granted && connectedCount > 0 && (
            // Stated here rather than only in the credential section above,
            // because this is where the connected badges are: the operator
            // reading "connected" is the one who needs to know their agents
            // still cannot use it. The connection itself is real — the grant
            // governs the tool belt, not the handshake (issue #582).
            <p className="flex items-start gap-2 rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
              <AlertTriangle className="mt-px size-3 shrink-0" />
              <span>
                These accounts are connected, but this company does not grant the{" "}
                <span className="font-mono">composio</span> tool namespace, so its agents will not
                receive their tools yet. Add <span className="font-mono">composio</span> to the
                company&apos;s tool grants.
              </span>
            </p>
          )}
          {degraded && (
            // The host could not read Composio's catalog. Say so — a built-in
            // list rendered like a fetched one is a claim we cannot back (#397).
            <p className="flex items-start gap-2 rounded-md bg-status-blocked-soft p-2 text-xs text-status-blocked-text">
              <AlertTriangle className="mt-px size-3 shrink-0" />
              <span>{degraded}</span>
            </p>
          )}
          {!degraded && openMode && (
            <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
              This company allows <span className="font-medium">any</span> provider Composio offers
              — {providers.length} in total, connected first. Filter by category or search by name,
              slug, or what a provider does.
            </p>
          )}

          {providers.length > 8 && (
            <div className="relative">
              <Search className="absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                aria-label="Search providers"
                autoComplete="off"
                className="pl-7"
                placeholder={`Search ${providers.length} providers by name or what they do…`}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
          )}

          {categories.length > 2 && (
            // Chips only earn their row when there is more than one real bucket
            // to choose between — `availableCategories` always includes "All",
            // so two means one bucket, which is not a choice.
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

          <ul
            className="grid gap-2"
            style={{
              // Uniform rows so a grid of 123 tiles reads as a grid and not as
              // ragged masonry — the tile is a fixed slot, and the label clamps.
              gridTemplateColumns: "repeat(auto-fill, minmax(8.5rem, 1fr))",
              gridAutoRows: "5.5rem",
            }}
          >
            {visible.map((row) => (
              <ProviderTile
                key={row.slug}
                row={row}
                canManage={canManage}
                busy={busy === row.providerId}
                anyBusy={busy !== null}
                noCredential={noCredential}
                onConnect={() => onConnect(row)}
                onDisconnect={() => onDisconnect(row)}
              />
            ))}
          </ul>

          {visible.length === 0 && (
            <p className="py-2 text-xs text-muted-foreground">
              {query.trim() !== "" ? (
                <>
                  No provider matches “{query.trim()}”
                  {category !== "All" && <> in {category}</>}. Composio&apos;s slug may differ from
                  the product name — try another category, or connect it by slug below.
                </>
              ) : (
                <>No provider in {category}.</>
              )}
            </p>
          )}

          {openMode && canManage && (
            <div className="flex items-end gap-2 border-t border-border pt-3">
              <div className="flex-1 space-y-1">
                <Label htmlFor="providers-other-toolkit" className="text-xs">
                  Connect by slug
                </Label>
                <Input
                  id="providers-other-toolkit"
                  autoComplete="off"
                  placeholder="composio toolkit slug, e.g. hubspot"
                  value={otherToolkit}
                  disabled={noCredential}
                  onChange={(e) => setOtherToolkit(e.target.value)}
                />
              </div>
              <Button
                variant="outline"
                disabled={busy !== null || noCredential || !otherToolkit.trim()}
                onClick={() => {
                  const slug = otherToolkit.trim().toLowerCase();
                  setOtherToolkit("");
                  onConnectSlug(slug);
                }}
              >
                <LogIn className="size-4" />
                Sign in
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

function SectionHeading({ count }: { count: number | null }) {
  return (
    <div className="space-y-1">
      <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
        Providers
      </h3>
      <p className="text-sm text-muted-foreground">
        Every account this company can act through, and which are wired.
        {count !== null && count > 0 && ` ${count} connected.`}
      </p>
    </div>
  );
}

/**
 * One tile.
 *
 * The whole tile is the affordance when there is exactly one thing to do with
 * it. An 8.5rem tile has no room for a label AND a button, and a tile that looks
 * clickable but is not would be worse than either.
 *
 * Connected tiles are the exception: the click target moves to a small
 * Disconnect control in the glyph slot, so the destructive action is never the
 * whole surface an operator brushes past while scanning.
 */
function ProviderTile({
  row,
  canManage,
  busy,
  anyBusy,
  noCredential,
  onConnect,
  onDisconnect,
}: {
  row: GridProvider;
  canManage: boolean;
  busy: boolean;
  anyBusy: boolean;
  noCredential: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
}) {
  // `managed` and `unavailable` render no action at all — that is the whole
  // point of routing the tile (issue #599): a button that could only 400 is
  // never drawn.
  const connectable =
    canManage && !row.connected && (row.route.kind === "composio" || row.route.kind === "native");
  const state = row.connected
    ? row.via.length > 0
      ? `connected via ${row.via.join(" + ")}`
      : "connected"
    : busy
      ? "signing in"
      : row.unverified
        ? "could not check"
        : row.route.kind === "managed"
          ? "managed by the platform"
          : row.route.kind === "unavailable"
            ? "not available here"
            : "not connected";

  const shell = cn(
    "flex size-full flex-col items-start justify-between gap-1 rounded-lg border p-2.5 text-left",
    row.connected ? "border-status-done/30 bg-status-done-soft" : "border-border bg-card",
  );

  const glyph = row.connected ? (
    canManage && row.canDisconnect ? (
      <button
        type="button"
        disabled={anyBusy}
        onClick={onDisconnect}
        title={`Disconnect ${row.label}`}
        aria-label={`Disconnect ${row.label}`}
        className={cn(
          "shrink-0 rounded p-0.5 text-status-done-text",
          "hover:bg-status-done/20 hover:text-foreground",
          "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
          "disabled:cursor-not-allowed disabled:opacity-60",
        )}
      >
        {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Unplug className="size-3.5" />}
      </button>
    ) : (
      <Check
        className="size-3.5 shrink-0 text-status-done-text"
        // Composio holds this connection and no host route can release it —
        // `DELETE …/connections/{provider}` only blanks a local secret this
        // provider never had. Saying where it lives beats a button that
        // reports success and changes nothing.
        aria-hidden="true"
      />
    )
  ) : busy ? (
    <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
  ) : connectable ? (
    <LogIn className="size-3.5 shrink-0 text-muted-foreground" />
  ) : null;

  const body = (
    <>
      <div className="flex w-full items-start justify-between gap-1">
        <ProviderLogo row={row} />
        {glyph}
      </div>
      <div className="w-full min-w-0">
        <span className="line-clamp-2 text-xs leading-tight font-medium">{row.label}</span>
        <span
          className={cn(
            "block truncate text-3xs",
            row.connected ? "text-status-done-text" : "text-muted-foreground",
          )}
        >
          {row.account ?? state}
        </span>
      </div>
    </>
  );

  const title =
    row.connected && !row.canDisconnect
      ? `${row.label} is connected through Composio; manage or revoke it there.`
      : row.description || undefined;

  return (
    // Keyed by the Composio slug, which is what the host calls it and what every
    // other surface in this issue reconciles on — so a spec that names a tile
    // names the same thing the backend does.
    <li className="min-w-0" data-testid={`provider-${row.slug}`}>
      {connectable ? (
        <button
          type="button"
          disabled={anyBusy || (row.route.kind === "composio" && noCredential)}
          onClick={onConnect}
          title={row.description || undefined}
          aria-label={`Connect ${row.label}. ${state}. Authorises: ${permissionHint(row.category)}.`}
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
        // Connected, unroutable, or a viewer who cannot manage: the tile itself
        // is not a control. Rendered as a div rather than a disabled button so
        // it stays in the reading order — "Gmail, connected" is exactly what a
        // member opened this page to learn, and a disabled button is
        // unfocusable.
        <div className={shell} title={title} aria-label={`${row.label}. ${state}.`}>
          {body}
        </div>
      )}
    </li>
  );
}

function ProviderLogo({ row }: { row: GridProvider }) {
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
