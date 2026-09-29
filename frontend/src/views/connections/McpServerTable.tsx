import { useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronRight,
  Download,
  Info,
  KeyRound,
  Loader2,
  LogIn,
  MoreHorizontal,
  Plug,
  Power,
  PowerOff,
  RefreshCw,
  Server,
  ShieldCheck,
  Trash2,
  Unplug,
  Wrench,
} from "lucide-react";

import type { McpCatalogueEntry } from "@/api/mcp-registry";
import type { McpHealth, McpServer } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { probedOn } from "@/lib/connection-detail";
import type { McpBridgeState } from "@/lib/mcp-bridge";
import { mcpHealthBadge } from "@/lib/mcp-bridge";
import { mcpRowControls, mcpSourceBadge } from "@/lib/mcp-registry";
import {
  McpReachCell,
  McpReachChips,
} from "@/views/connections/mcp-reach-cell";

/**
 * The company's servers and the directory's, as one table.
 *
 * One row grammar for both halves — a mark, a one-line description, where it
 * came from, whether it answers, who reaches it, and one action. Only the action
 * differs between the two.
 */

/** Everything a row's controls can do, owned by the section that holds the state. */
export interface McpRowActions {
  onOpen: (name: string) => void;
  onSignIn: (server: McpServer) => void;
  onAddToken: (server: McpServer) => void;
  onRotateEnv: (server: McpServer) => void;
  onLifecycle: (server: McpServer, direction: "connect" | "disconnect") => void;
  onTest: (server: McpServer) => void;
  onTools: (server: McpServer) => void;
  onPermissions: (name: string) => void;
  onToggle: (server: McpServer, enabled: boolean) => void;
  onRemove: (server: McpServer) => void;
}

/** Which one labelled action this server's state actually calls for. */
export type PrimaryAction =
  | { kind: "sign_in" }
  | { kind: "add_token" }
  | { kind: "rotate_env" }
  | { kind: "connect" }
  | null;

/** What the expander says about a server nobody declared a description for. */
function describe(server: McpServer): { text: string; muted: boolean } {
  const declared = server.description?.trim();
  if (declared) return { text: declared, muted: false };
  const probed = server.probedDescription?.trim();
  if (probed) return { text: probed, muted: false };
  return { text: server.endpoint, muted: true };
}

/** A server's mark, falling back to the letter tile the directory already uses. */
export function McpServerIcon({
  iconUrl,
  name,
  className = "size-7",
}: {
  iconUrl?: string;
  name: string;
  className?: string;
}) {
  const [failed, setFailed] = useState(false);
  if (!iconUrl || failed) {
    return (
      <span
        aria-hidden="true"
        className={`flex shrink-0 items-center justify-center rounded-md border border-border bg-muted/40 text-xs font-semibold text-muted-foreground ${className}`}
      >
        {name.charAt(0).toUpperCase() || <Server className="size-3.5" />}
      </span>
    );
  }
  return (
    // An inline `data:` image the host fetched itself; a remote server's own
    // address must never become a request from the operator's browser.
    <img
      src={iconUrl}
      alt=""
      aria-hidden="true"
      className={`shrink-0 rounded-md border border-border object-contain ${className}`}
      onError={() => setFailed(true)}
    />
  );
}

/** The health badge, in the register the bridge state entitles it to. */
function HealthBadge({
  health,
  authConfigured,
  bridge,
}: {
  health?: McpHealth;
  authConfigured: boolean;
  bridge: McpBridgeState;
}) {
  const badge = mcpHealthBadge(health, authConfigured, bridge);
  if (!badge) return null;
  const tone =
    badge.tone === "delivering"
      ? { className: "text-status-done-text", Icon: Check }
      : badge.tone === "configured"
        ? { className: "text-muted-foreground", Icon: Info }
        : badge.tone === "warn"
          ? { className: "text-status-blocked-text", Icon: AlertTriangle }
          : { className: "text-destructive", Icon: AlertTriangle };
  return (
    <span
      className={`inline-flex items-center gap-1 text-xs whitespace-nowrap ${tone.className}`}
    >
      <tone.Icon className="size-3" /> {badge.label}
    </span>
  );
}

const HEADERS = ["Server", "Source", "Status", "Reach", ""];

export function McpServerTable({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="-mx-2 overflow-x-auto px-2">
      <table className="w-full min-w-[46rem] border-collapse text-sm">
        <thead>
          <tr>
            <th className="w-6 border-b border-border pb-1.5" />
            {HEADERS.map((head, i) => (
              <th
                key={head || `spacer-${i}`}
                className="border-b border-border pr-3 pb-1.5 text-left text-3xs font-medium tracking-wide text-muted-foreground uppercase"
              >
                {head}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}

/** A heading inside the one list, carrying the distinction two tabs used to. */
export function McpGroupRow({
  label,
  count,
}: {
  label: string;
  count?: string;
}) {
  return (
    <tr data-testid="mcp-group-row">
      <td colSpan={6} className="border-b border-border pt-3 pb-1">
        <div className="flex items-center gap-2">
          <span className="text-3xs font-medium tracking-wide text-muted-foreground uppercase">
            {label}
          </span>
          {count && (
            <span className="text-3xs text-muted-foreground">{count}</span>
          )}
        </div>
      </td>
    </tr>
  );
}

/**
 * The caret at the leading edge of a row.
 *
 * The only disclosure. Hover only tints the row; it never reveals the detail.
 */
function Expander({
  open,
  label,
  onToggle,
}: {
  open: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <td className="border-b border-border py-2 align-top">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-label={label}
        data-testid="mcp-row-expander"
        className="flex size-5 items-center justify-center rounded-sm text-muted-foreground transition-transform hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      >
        <ChevronRight className={`size-3.5 ${open ? "rotate-90" : ""}`} />
      </button>
    </td>
  );
}

function Detail({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <tr data-testid="mcp-row-detail">
      <td />
      <td colSpan={5} className="border-b border-border pr-3 pb-3">
        <div className="grid gap-x-6 gap-y-3 rounded-md border border-border bg-muted/30 p-3 sm:grid-cols-[minmax(0,1fr)_minmax(11rem,13rem)]">
          {children}
        </div>
      </td>
    </tr>
  );
}

function DetailKey({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-3xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </span>
  );
}

function Facts({ rows }: { rows: [string, React.ReactNode][] }) {
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] items-baseline gap-x-3 gap-y-1 text-xs">
      {rows.map(([key, value]) => (
        <div key={key} className="col-span-2 grid grid-cols-subgrid">
          <dt className="whitespace-nowrap text-muted-foreground">{key}</dt>
          <dd className="min-w-0 truncate">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function McpServerRow({
  server,
  health,
  bridge,
  canManage,
  busy,
  primary,
  signingIn,
  toolsOpen,
  actions,
}: {
  server: McpServer;
  health?: McpHealth;
  bridge: McpBridgeState;
  canManage: boolean;
  /** The name of the row holding the mutation lock, or `null`. */
  busy: string | null;
  primary: PrimaryAction;
  /** Whether an OAuth sign-in for this row is still waiting on the other tab. */
  signingIn: boolean;
  toolsOpen: boolean;
  actions: McpRowActions;
}) {
  const [open, setOpen] = useState(false);
  const controls = mcpRowControls(server, health);
  const dial = controls.lifecycle === "none" ? null : controls.lifecycle;
  const badge = mcpSourceBadge(server.source);
  const description = describe(server);
  const reach = server.reachableBy;
  const locked = busy !== null;
  const mine = busy === server.name;

  // The testid names the affordance, not the slot, so it pins the decider's
  // answer to the control the row renders.
  const primaryLabel =
    primary === null
      ? null
      : primary.kind === "sign_in"
        ? {
            short: "Sign in",
            long: `Sign in to ${server.name}`,
            Icon: LogIn,
            testId: "mcp-sign-in",
          }
        : primary.kind === "connect"
          ? {
              short: "Connect",
              long: `Connect ${server.name}`,
              Icon: Plug,
              testId: "mcp-lifecycle",
            }
          : {
              short: server.authConfigured
                ? "Replace credential"
                : "Add credential",
              long: server.authConfigured
                ? `Replace ${server.name}'s API token`
                : `Add an API token for ${server.name}`,
              Icon: KeyRound,
              testId:
                primary.kind === "rotate_env"
                  ? "mcp-rotate-env"
                  : "mcp-add-token",
            };

  function runPrimary() {
    if (primary === null) return;
    if (primary.kind === "sign_in") actions.onSignIn(server);
    else if (primary.kind === "add_token") actions.onAddToken(server);
    else if (primary.kind === "rotate_env") actions.onRotateEnv(server);
    else actions.onLifecycle(server, "connect");
  }

  return (
    <>
      <tr
        data-testid="mcp-server-row"
        className="transition-colors hover:bg-muted/40"
      >
        <Expander
          open={open}
          label={`Show what ${server.name} is`}
          onToggle={() => setOpen((was) => !was)}
        />
        <td className="border-b border-border py-2 pr-3 align-top">
          <div className="flex min-w-0 items-center gap-2">
            <McpServerIcon iconUrl={server.iconUrl} name={server.name} />
            <div className="min-w-0">
              {/* The name is the link, so there is no View button: a row that
                  opens a page does not need a control saying so. */}
              <button
                type="button"
                data-testid="mcp-server-open"
                className="block max-w-full truncate rounded-sm text-left text-sm font-medium hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                onClick={() => actions.onOpen(server.name)}
                aria-label={`Open ${server.name}`}
              >
                {server.name}
              </button>
              <span
                className={`block max-w-[22rem] truncate text-xs ${description.muted ? "text-muted-foreground/70" : "text-muted-foreground"}`}
              >
                {description.muted && "no description — "}
                {description.text}
              </span>
            </div>
          </div>
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          <Badge variant={badge.variant} data-testid="mcp-source-badge">
            {badge.label}
          </Badge>
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          <div className="flex flex-col gap-0.5">
            <HealthBadge
              health={health}
              authConfigured={server.authConfigured}
              bridge={bridge}
            />
            {controls.toggle && !server.enabled && (
              <span
                className="text-3xs text-muted-foreground"
                data-testid="mcp-disabled-badge"
              >
                off
              </span>
            )}
            {signingIn && (
              <span
                className="text-3xs text-status-blocked-text"
                data-testid="mcp-signing-in"
              >
                waiting for sign-in
              </span>
            )}
          </div>
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          {/* The column carries the deployment fact into every row: with no
              bridge the banner above says no agent receives these tools, and a
              reach of three names underneath it would contradict that. */}
          {bridge === "absent" ? (
            <span className="text-xs text-muted-foreground">—</span>
          ) : reach === undefined ? (
            <span className="text-xs text-muted-foreground">—</span>
          ) : !server.enabled ? (
            <span className="text-xs text-muted-foreground">—</span>
          ) : (
            <McpReachCell
              agents={reach}
              serverName={server.name}
              onOverflow={() => setOpen(true)}
            />
          )}
        </td>
        <td className="border-b border-border py-2 align-top">
          <div className="flex items-center justify-end gap-1">
            {primaryLabel && canManage && (
              <Button
                size="sm"
                disabled={locked}
                aria-label={primaryLabel.long}
                data-mcp-primary="true"
                data-testid={primaryLabel.testId}
                onClick={runPrimary}
              >
                {mine ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <primaryLabel.Icon className="size-3.5" />
                )}
                {primaryLabel.short}
              </Button>
            )}
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`More actions for ${server.name}`}
                    data-testid="mcp-row-overflow"
                    disabled={locked}
                  />
                }
              >
                <MoreHorizontal className="size-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  data-testid="mcp-permissions"
                  onClick={() => actions.onPermissions(server.name)}
                >
                  <ShieldCheck className="mr-2 size-4" />
                  Tool permissions
                </DropdownMenuItem>
                {controls.probe && (
                  <>
                    <DropdownMenuItem
                      data-testid="mcp-test"
                      onClick={() => actions.onTest(server)}
                    >
                      <RefreshCw className="mr-2 size-4" />
                      Re-check
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      data-testid="mcp-tools"
                      onClick={() => actions.onTools(server)}
                    >
                      <Wrench className="mr-2 size-4" />
                      {toolsOpen ? "Hide its tools" : "List its tools"}
                    </DropdownMenuItem>
                  </>
                )}
                {/* Suppressed when it is already this row's one labelled
                    action, so the control is offered once. */}
                {dial !== null && canManage && primary?.kind !== "connect" && (
                  <DropdownMenuItem
                    data-testid="mcp-lifecycle"
                    onClick={() => actions.onLifecycle(server, dial)}
                  >
                    {dial === "connect" ? (
                      <Plug className="mr-2 size-4" />
                    ) : (
                      <Unplug className="mr-2 size-4" />
                    )}
                    {dial === "connect" ? "Connect" : "Disconnect"}
                  </DropdownMenuItem>
                )}
                {controls.toggle && canManage && (
                  <DropdownMenuItem
                    data-testid="mcp-toggle"
                    onClick={() => actions.onToggle(server, !server.enabled)}
                  >
                    {server.enabled ? (
                      <PowerOff className="mr-2 size-4" />
                    ) : (
                      <Power className="mr-2 size-4" />
                    )}
                    {server.enabled ? "Turn off" : "Turn on"}
                  </DropdownMenuItem>
                )}
                {controls.removal.kind !== "none" && canManage && (
                  <>
                    <DropdownMenuSeparator />
                    {/* Nothing destructive sits a mis-click from a scan. */}
                    <DropdownMenuItem
                      variant="destructive"
                      data-testid="mcp-remove"
                      onClick={() => actions.onRemove(server)}
                    >
                      <Trash2 className="mr-2 size-4" />
                      Remove
                    </DropdownMenuItem>
                  </>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </td>
      </tr>
      {open && (
        <Detail>
          <div className="flex min-w-0 flex-col gap-2">
            <DetailKey>What it does</DetailKey>
            {description.muted ? (
              <p className="text-xs text-muted-foreground italic">
                {server.source === "runtime"
                  ? "No description. One was never asked for when this server was added by URL."
                  : "This server sent no description, and its declaration carries none."}
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">
                {description.text}
              </p>
            )}
            <DetailKey>Endpoint</DetailKey>
            <code className="block truncate rounded-md border border-border bg-background px-2 py-1 font-mono text-xs">
              {server.endpoint}
            </code>
          </div>
          <div className="flex min-w-0 flex-col gap-2">
            <DetailKey>Details</DetailKey>
            <Facts
              rows={[
                [
                  "Tools",
                  health && health.checkedAtMillis > 0 ? (
                    String(health.toolCount)
                  ) : (
                    <span className="text-muted-foreground">not probed</span>
                  ),
                ],
                ["Added by", ADDED_BY[server.source]],
                [
                  "Last checked",
                  probedOn(health?.checkedAtMillis) ?? (
                    <span className="text-muted-foreground">never</span>
                  ),
                ],
                ...(server.websiteUrl
                  ? ([
                      [
                        "Website",
                        <a
                          key="site"
                          href={server.websiteUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="underline"
                        >
                          {server.websiteUrl}
                        </a>,
                      ],
                    ] as [string, React.ReactNode][])
                  : []),
              ]}
            />
          </div>
          <div className="flex flex-col gap-2 border-t border-border pt-2 sm:col-span-2">
            <DetailKey>Reachable by</DetailKey>
            {reach === undefined ? (
              <p className="text-xs text-muted-foreground">
                This host does not report who reaches a server.
              </p>
            ) : reach.length === 0 ? (
              <p className="text-xs text-muted-foreground">
                No teammate — no tool grant covers{" "}
                <code className="font-mono">mcp:{server.name}</code>.
              </p>
            ) : (
              <McpReachChips agents={reach} />
            )}
          </div>
        </Detail>
      )}
    </>
  );
}

const ADDED_BY: Record<string, string> = {
  manifest: "the manifest",
  registry: "the MCP directory",
  runtime: "this console",
  default: "the packaged install",
};

export function McpDirectoryRow({
  entry,
  installedAs,
  installing,
  canManage,
  onInstall,
}: {
  entry: McpCatalogueEntry;
  /** The name this company already has it under, when it already has it. */
  installedAs: string | null;
  installing: boolean;
  canManage: boolean;
  onInstall: (entry: McpCatalogueEntry) => void;
}) {
  const [open, setOpen] = useState(false);
  const installed = installedAs !== null;

  return (
    <>
      <tr
        data-testid="mcp-directory-row"
        className="transition-colors hover:bg-muted/40"
      >
        <Expander
          open={open}
          label={`Show what ${entry.displayName} is`}
          onToggle={() => setOpen((was) => !was)}
        />
        <td className="border-b border-border py-2 pr-3 align-top">
          <div className="flex min-w-0 items-center gap-2">
            <McpServerIcon
              iconUrl={entry.iconUrl}
              name={entry.displayName}
            />
            <div className="min-w-0">
              <span className="block truncate text-sm font-medium">
                {entry.displayName}
              </span>
              <span className="block max-w-[22rem] truncate text-xs text-muted-foreground">
                {entry.description ?? entry.qualifiedName}
              </span>
            </div>
          </div>
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          <Badge variant="outline">directory</Badge>
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          {/* There is nothing to probe yet, so this column carries the
              directory's claim about the publisher instead — which is never a
              claim about what the server does. */}
          {entry.official ? (
            <span className="inline-flex items-center gap-1 text-xs whitespace-nowrap text-status-done-text">
              <Check className="size-3" /> official
            </span>
          ) : (
            <span className="text-xs whitespace-nowrap text-status-blocked-text">
              unverified
            </span>
          )}
        </td>
        <td className="border-b border-border py-2 pr-3 align-top">
          <span className="text-xs text-muted-foreground">
            {installed ? "already yours" : "—"}
          </span>
        </td>
        <td className="border-b border-border py-2 align-top">
          <div className="flex items-center justify-end">
            {installed ? (
              <span
                className="inline-flex items-center gap-1 text-xs whitespace-nowrap text-status-done-text"
                data-testid="mcp-directory-installed"
              >
                <Check className="size-3" /> Installed
              </span>
            ) : (
              canManage && (
                <Button
                  size="sm"
                  disabled={installing}
                  data-testid="mcp-directory-install"
                  aria-label={`Install ${entry.displayName}`}
                  onClick={() => onInstall(entry)}
                >
                  {installing ? (
                    <Loader2 className="size-3.5 animate-spin" />
                  ) : (
                    <Download className="size-3.5" />
                  )}
                  Install
                </Button>
              )
            )}
          </div>
        </td>
      </tr>
      {open && (
        <Detail>
          <div className="flex min-w-0 flex-col gap-2">
            <DetailKey>What it does</DetailKey>
            <p className="text-xs text-muted-foreground">
              {entry.description ?? "The directory listing carries no description."}
            </p>
            <DetailKey>Published as</DetailKey>
            <code className="block truncate rounded-md border border-border bg-background px-2 py-1 font-mono text-xs">
              {entry.qualifiedName}
            </code>
          </div>
          <div className="flex min-w-0 flex-col gap-2">
            <DetailKey>Details</DetailKey>
            <Facts
              rows={[
                ["Catalogue", entry.source],
                [
                  "Publisher",
                  entry.official ? (
                    "official"
                  ) : (
                    <span className="text-muted-foreground">not verified</span>
                  ),
                ],
                [
                  "In this company",
                  installed ? (
                    `installed as ${installedAs}`
                  ) : (
                    <span className="text-muted-foreground">not installed</span>
                  ),
                ],
              ]}
            />
          </div>
          <div className="border-t border-border pt-2 sm:col-span-2">
            {entry.official ? (
              <p className="text-xs text-muted-foreground">
                <strong className="font-medium text-foreground">official</strong>{" "}
                is the directory&apos;s claim about the publisher — never a claim
                about what the server does.
              </p>
            ) : (
              <p
                className="flex items-start gap-2 rounded-md border border-status-blocked-text/30 bg-status-blocked-text/10 px-2 py-1 text-xs text-status-blocked-text"
                data-testid="mcp-directory-unverified"
              >
                <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                <span>
                  The directory is public and unvetted. Installing is allowed —
                  every tool it exposes still starts un-granted, and stays that
                  way until somebody sets it.
                </span>
              </p>
            )}
          </div>
        </Detail>
      )}
    </>
  );
}
