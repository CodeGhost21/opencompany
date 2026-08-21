// Which host this console is looking at, in the sidebar header.
//
// It replaces the icon rail down the left edge (issue #1142), which showed one
// building glyph per host with a status dot and a `+`: no names, no keyboard
// access, and status reduced to a colour.
//
// ## The rule this component inherits
//
// Choosing a host is a **filter over N live things**, not a selector that makes
// one live. It changes which console is on screen and nothing else: no
// connection is torn down, no stream is closed, no storage is re-scoped.
// Selection lives in `App`'s local state and reaches here through
// `HostsContext`, which carries the long version of that rule.
//
// ## Why the trigger carries a dot
//
// The rail's amber dots were the only at-a-glance sign that something was wrong
// on a host you were *not* looking at. A dropdown hides its rows, so the signal
// has to survive on the trigger: the dot there is the **worst** state across
// every host, and `data-worst-status` says the same thing to a test. Without it
// removing the rail would be a net loss for anyone running more than one host.

import { Building2, Check, ChevronsUpDown, Loader2, Play, Plus, Square } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { isDesktopRuntime } from "@/api/transport";
import type { LocalInstance } from "@/api/transport/desktop";
import { hostShortcutLabel, useHosts } from "@/connections/HostsContext";
import type {
  Connection,
  ConnectionStatus,
  ConnectorKind,
  SshTarget,
} from "@/connections/types";
import { DEFAULT_REMOTE_PORT, availableConnectors } from "@/connections/types";
import { cn } from "@/lib/utils";

/**
 * How a connection's state reads, and what colour says so.
 *
 * The `label` is the load-bearing half and the dot is the shorthand, not the
 * other way round: a row that is anything but `live` prints its state (issue
 * #1167), because hue alone tells an operator that *something* differs without
 * saying what, and tells someone who cannot separate amber from green nothing.
 */
export const STATUS_COPY: Record<ConnectionStatus, { label: string; dot: string }> = {
  connecting: { label: "Connecting…", dot: "bg-muted-foreground/50" },
  live: { label: "Connected", dot: "bg-status-done" },
  degraded: { label: "Partly available", dot: "bg-status-blocked" },
  down: { label: "Unreachable", dot: "bg-destructive" },
  unauthenticated: { label: "Sign-in needed", dot: "bg-status-blocked" },
};

/**
 * How a host's state reads on its own row.
 *
 * {@link STATUS_COPY} keyed by status, except for the one case where the
 * status alone is not the whole answer: a cloud tenant that has been idle is
 * being *started*, not contacted, and that takes seconds rather than
 * milliseconds. "Connecting…" for that long reads as a hang, so the connector
 * supplies the word — which is why this takes a connection rather than a
 * status.
 *
 * Deliberately not a fifth `ConnectionStatus`. Waking is a connecting host and
 * ranks exactly like one on the trigger; a new status would need a place in the
 * severity ordering, and every `switch` over the union would have to grow a
 * branch for a state that behaves identically.
 */
export function statusCopy(connection: Connection): { label: string; dot: string } {
  const copy = STATUS_COPY[connection.status];
  return connection.waking ? { ...copy, label: "Waking…" } : copy;
}

/**
 * How loudly each state asks to be noticed.
 *
 * Highest wins on the trigger. `down` outranks `unauthenticated` because one is
 * a host that is gone and the other is a host that is there and waiting; and
 * `connecting` outranks `live` only so a roster still settling does not claim
 * to be entirely fine.
 */
const SEVERITY: Record<ConnectionStatus, number> = {
  live: 0,
  connecting: 1,
  degraded: 2,
  unauthenticated: 3,
  down: 4,
};

/** The state the trigger reports: the worst one any host is in. */
export function worstStatus(connections: Connection[]): ConnectionStatus | null {
  let worst: ConnectionStatus | null = null;
  for (const connection of connections) {
    if (worst === null || SEVERITY[connection.status] > SEVERITY[worst]) {
      worst = connection.status;
    }
  }
  return worst;
}

/**
 * Whether the switcher opens a menu, or is only a nameplate.
 *
 * One host is the ordinary web deployment, and a chevron over a menu of one is
 * furniture. It becomes a control when there is a choice to make.
 *
 * **The desktop is always interactive**, whatever the count. The old rule made
 * an exception only for zero, on the reasoning that a browser always has its
 * bootstrap connection so a real zero could only happen in the desktop — and
 * that a desktop holding nothing needs the "Add a host" this menu carries.
 * Issue #613 moved where that bites: the desktop no longer writes a same-origin
 * bootstrap row, so a desktop whose embedded host *did* start now holds exactly
 * **one** connection, which is its ordinary state rather than a rare one. Under
 * the old rule that state offered no menu, and "Add a host" is the only way to
 * reach a second host — so the working case became the one with no way out of
 * it, which is the dead end the zero exception existed to prevent.
 *
 * **A hub is always interactive too**, for the same reason and by the same
 * argument. A hub has no bootstrap connection either (its origin serves assets,
 * not a host), so it holds exactly the hosts someone added — and at zero "Add a
 * host" is the only way to add the first one.
 *
 * An ordinary same-origin browser console is unaffected: neither
 * `isDesktopRuntime()` nor `hub` is true there, so one host still draws a plain
 * nameplate with no chevron.
 *
 * This is the predicate `connectionRailVisible` used to be — same rule, moved
 * with the control it governs.
 */
export function hostSwitcherInteractive(count: number, hub = false): boolean {
  return count >= 2 || hub || isDesktopRuntime();
}

interface Props {
  /**
   * Which chrome this is drawn in.
   *
   * `sidebar` is the home: the header row of the app shell. `standalone` is for
   * the screens that have no shell — a host that cannot be reached, a sign-in,
   * the desktop's "no host to show" — where the switcher is the only way back
   * to a host that works, and where its absence is what stranded an operator
   * when the rail was the one holding it.
   */
  variant?: "sidebar" | "standalone";
  /**
   * The company on screen, when there is one.
   *
   * The trigger names the company because that is what the person is looking
   * at; the host is the second line, and only when more than one is connected.
   * Absent on the standalone chrome, which has no company yet.
   */
  companyName?: string;
}

export function HostSwitcher({ variant = "sidebar", companyName }: Props) {
  const hosts = useHosts();
  const { connections, selected, hub } = hosts;

  const interactive = hostSwitcherInteractive(connections.length, hub);
  const active = connections.find((c) => c.id === selected) ?? null;
  const worst = worstStatus(connections);

  // Nothing to switch and no shell to sit in: the ordinary single-host web
  // console gets exactly what it had before, which is nothing at all.
  if (variant === "standalone" && !interactive) return null;

  // Shown when it has something to say. On the ordinary single-host console the
  // switcher is a nameplate, and a permanently green dot beside it is furniture
  // — that console's host being unreachable is a full-screen error, not a
  // detail to notice in the corner. The dot earns its place once there is a
  // host you are *not* looking at, or once something is actually wrong.
  const dot =
    worst && (interactive || worst !== "live") ? (
      <span
        data-testid="host-switcher-status"
        className={cn(
          "absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2",
          // The ring is a CUT-OUT of the ground, and the two variants stand on
          // different grounds (issue #1178). In the shell the trigger has no
          // fill at rest, so half this ring lands on the window chrome; the
          // standalone console draws its own `bg-sidebar` card behind it.
          variant === "sidebar" ? "ring-chrome" : "ring-sidebar",
          STATUS_COPY[worst].dot,
        )}
      />
    ) : null;

  const glyph = (
    <div className="relative flex aspect-square size-8 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
      <Building2 className="size-4" />
      {dot}
    </div>
  );

  const primary = companyName ?? active?.label ?? "No host";
  // The host on the second line, but only when it says something the first line
  // does not. Most hosts serve one company and are named after it, so repeating
  // the name in both rows would spend the only line the trigger has on nothing.
  const secondary = companyName
    ? connections.length > 1 && active && active.label !== companyName
      ? active.label
      : "Current company"
    : worst
      ? STATUS_COPY[worst].label
      : "Not connected";

  const nameplate = (
    <>
      {glyph}
      <div className="grid flex-1 text-left leading-tight">
        <span className="truncate text-sm font-semibold">{primary}</span>
        <span className="truncate text-xs text-muted-foreground">{secondary}</span>
      </div>
    </>
  );

  // Read off the closed trigger, so host count and cross-host health stay
  // answerable — by an operator and by a test — without opening anything. That
  // is what the rail gave away for free and what a dropdown otherwise hides.
  const triggerData = {
    "data-testid": "host-switcher",
    "data-host-count": connections.length,
    "data-worst-status": worst ?? "none",
  };

  if (!interactive) {
    return (
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            size="lg"
            className="cursor-default hover:bg-transparent"
            {...triggerData}
          >
            {nameplate}
          </SidebarMenuButton>
        </SidebarMenuItem>
      </SidebarMenu>
    );
  }

  const renderMenu = (side: "right" | "bottom") => (
    <DropdownMenuContent className="min-w-72 rounded-lg" align="start" side={side} sideOffset={4}>
      {/* `DropdownMenuLabel` is Base UI's `Menu.GroupLabel`, and it throws
            outside a `Menu.Group` — the group is what it labels. */}
      <DropdownMenuGroup>
        <DropdownMenuLabel>Hosts</DropdownMenuLabel>
        {connections.map((connection, index) => {
          const status = statusCopy(connection);
          const shortcut = hostShortcutLabel(index);
          return (
            <DropdownMenuItem
              key={connection.id}
              data-testid={`host-row-${connection.id}`}
              data-status={connection.status}
              aria-current={connection.id === selected}
              // Native `title`, not a tooltip component: the reason a host is
              // unreachable has to be readable without extra machinery on a
              // row that must render while its host is down.
              title={`${connection.label} — ${status.label}${
                connection.error ? `\n${connection.error}` : ""
              }`}
              onClick={() => hosts.onSelect(connection.id)}
              className="gap-2"
            >
              <span className={cn("size-2 shrink-0 rounded-full", status.dot)} />
              <span className={cn("flex-1 truncate", connection.id === selected && "font-medium")}>
                {connection.label}
              </span>
              {connection.status === "live" ? (
                // Nothing to say out loud on a host that is simply working, so
                // the state stays where a screen reader can still reach it.
                <span className="sr-only">{status.label}</span>
              ) : (
                // In words, not in hue (issue #1167). A colour is no help to
                // anyone who cannot tell these two apart, and even where it is
                // read correctly "amber" does not say *what* is wrong —
                // leaving an operator to discover an unreachable host by
                // landing on its failure. This is the row telling them first.
                <span
                  data-testid={`host-row-state-${connection.id}`}
                  className="shrink-0 text-xs text-muted-foreground"
                >
                  {status.label}
                </span>
              )}
              {shortcut && <DropdownMenuShortcut>{shortcut}</DropdownMenuShortcut>}
              {connection.id === selected && <Check className="size-4 shrink-0" />}
            </DropdownMenuItem>
          );
        })}
        {connections.length === 0 && (
          <p className="px-1.5 py-1 text-xs text-muted-foreground">No hosts yet.</p>
        )}
      </DropdownMenuGroup>
      <DropdownMenuSeparator />
      <DropdownMenuItem
        data-testid="host-switcher-add"
        onClick={() => hosts.setAddingHost(true)}
        className="gap-2"
      >
        <Plus className="size-4" />
        Add a host
      </DropdownMenuItem>
    </DropdownMenuContent>
  );

  if (variant === "standalone") {
    return (
      <DropdownMenu>
        <DropdownMenuTrigger
          // No `aria-label`: the name has to come from the content, the same
          // way it does in the sidebar. A label saying "Switch host" would
          // override the host's own name and its status — which on this chrome
          // is the whole reason the control is on screen.
          render={
            <button
              type="button"
              className="flex w-64 max-w-[calc(100vw-2rem)] items-center gap-2 rounded-lg border bg-sidebar p-2 text-left shadow-sm transition hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            />
          }
          {...triggerData}
        >
          {nameplate}
          <ChevronsUpDown className="ml-auto size-4 shrink-0 text-muted-foreground" />
        </DropdownMenuTrigger>
        {renderMenu("bottom")}
      </DropdownMenu>
    );
  }

  return (
    <SidebarHeaderTrigger triggerData={triggerData} nameplate={nameplate} renderMenu={renderMenu} />
  );
}

/**
 * The switcher over a screen that has no app shell.
 *
 * A host that cannot be reached, a sign-in, a setup wizard, a desktop holding
 * no host at all: none of these mount a sidebar, and all of them are exactly
 * when an operator needs to get to a host that works. The rail used to be on
 * screen behind every one of them for free, because it lived outside the
 * console; the switcher has to be put there deliberately.
 *
 * It draws nothing on an ordinary single-host console, so those screens are
 * unchanged there.
 */
export function ConsoleChrome({ children }: { children: React.ReactNode }) {
  return (
    <div className="relative min-h-svh">
      <div className="absolute top-4 left-4 z-50">
        <HostSwitcher variant="standalone" />
      </div>
      {children}
    </div>
  );
}

/**
 * The sidebar chrome for the trigger.
 *
 * Split out only because `useSidebar` throws outside a `SidebarProvider`, and
 * the standalone variant renders on screens that have no sidebar at all.
 */
function SidebarHeaderTrigger({
  triggerData,
  nameplate,
  renderMenu,
}: {
  triggerData: Record<string, string | number>;
  nameplate: React.ReactNode;
  /** Mobile has no room to the right of the sidebar, so the menu drops instead. */
  renderMenu: (side: "right" | "bottom") => React.ReactNode;
}) {
  const { isMobile } = useSidebar();
  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          {/* The sidebar button renders *as* the trigger, rather than the
              trigger rendering as the button. Under React 18 a function
              component cannot be handed a ref, so the outer form leaves Base UI
              without an anchor to measure and the menu opens invisible at the
              top-left corner of the window. This way the DOM element is the
              trigger's own, and the sidebar's classes are merged onto it. */}
          <SidebarMenuButton
            size="lg"
            className="data-[popup-open]:bg-sidebar-accent data-[popup-open]:text-sidebar-accent-foreground"
            render={<DropdownMenuTrigger />}
            {...triggerData}
          >
            {nameplate}
            <ChevronsUpDown className="ml-auto size-4 shrink-0" />
          </SidebarMenuButton>
          {renderMenu(isMobile ? "bottom" : "right")}
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}

/**
 * Where a host comes from: an address somewhere else, or this machine.
 *
 * Lifted out of the rail unchanged — it is the same dialog, reached from the
 * menu's last item instead of from a `+` at the bottom of a strip of icons.
 *
 * **Mounted beside the console, not inside the switcher.** Creating a host on
 * this computer selects it, which remounts the console — and the switcher with
 * it — so a dialog owned by the switcher would close itself the moment it
 * succeeded, taking the roster away while the operator was still working in it.
 * Its open flag lives in `HostsContext` for the same reason.
 */
export function AddHostDialog() {
  const hosts = useHosts();
  const {
    addingHost: open,
    setAddingHost: onOpenChange,
    localInstances,
    onAddLocal,
    onAddSsh,
    onStartLocal,
    onStopLocal,
  } = hosts;
  // The two connectors that need a process on this machine are offered only
  // where one can be started. `onAddLocal` rather than `isDesktopRuntime()`:
  // `App` withholds these handlers on a shell too old to honour them, and a tab
  // whose button does nothing is worse than a tab that is not there.
  const tabs = availableConnectors(Boolean(onAddLocal)).filter(
    (kind) => kind !== "ssh" || onAddSsh,
  );
  const onAdd = (baseUrl: string, kind: ConnectorKind) => {
    hosts.onAdd(baseUrl, kind === "cloud" ? { kind, tenant: tenantOf(baseUrl) } : { kind: "remote" });
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add a host</DialogTitle>
          <DialogDescription>
            A host is one OpenCompany server. Run one on this computer, use one hosted
            for you, or point at one somewhere else — over ssh if it is not on the open
            internet. Whichever you choose stays connected alongside the others, and it
            decides where a <em>new</em> company lives rather than moving one you have.
          </DialogDescription>
        </DialogHeader>
        {/*
          The desktop leads with the local half, because starting a host is the
          thing it can do that a browser cannot — and because a person who has
          just installed the application has no URL to type. A browser leads
          with the cloud, which is the only one of the four it can offer that
          nobody has to run themselves.
        */}
        <Tabs defaultValue={tabs[0]}>
          <TabsList className="w-full">
            {tabs.map((kind) => (
              <TabsTrigger key={kind} value={kind} data-testid={`add-host-${kind}`}>
                {CONNECTOR_LABELS[kind]}
              </TabsTrigger>
            ))}
          </TabsList>
          {onAddLocal ? (
            <TabsContent value="local">
              <LocalInstances
                instances={localInstances}
                onAdd={onAddLocal}
                onStart={onStartLocal}
                onStop={onStopLocal}
              />
            </TabsContent>
          ) : null}
          <TabsContent value="cloud">
            <CloudHost
              onAdd={(baseUrl) => onAdd(baseUrl, "cloud")}
              onCancel={() => onOpenChange(false)}
            />
          </TabsContent>
          <TabsContent value="remote">
            <RemoteHost
              onAdd={(baseUrl) => onAdd(baseUrl, "remote")}
              onCancel={() => onOpenChange(false)}
              desktop={Boolean(onAddLocal)}
            />
          </TabsContent>
          {onAddSsh ? (
            <TabsContent value="ssh">
              <SshHost onAdd={onAddSsh} onDone={() => onOpenChange(false)} />
            </TabsContent>
          ) : null}
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

/** What each connector is called where an operator has to choose between them. */
const CONNECTOR_LABELS: Record<ConnectorKind, string> = {
  local: "On this computer",
  cloud: "TinyHumans Cloud",
  remote: "Another gateway",
  ssh: "Over SSH",
};

/**
 * Which tenant a cloud address names.
 *
 * The first label of the host name, which is the subdomain the control plane
 * gives a tenant. Only ever a *name* — it decides nothing about where requests
 * go, which is `baseUrl`'s job — so a url shaped some other way degrading to
 * the whole authority costs nothing but a longer word in a row.
 */
function tenantOf(baseUrl: string): string {
  try {
    const { hostname } = new URL(baseUrl);
    return hostname.split(".")[0] || hostname;
  } catch {
    return baseUrl;
  }
}

/** A tenant of the hosted platform, at the address it was given. */
function CloudHost({
  onAdd,
  onCancel,
}: {
  onAdd: (baseUrl: string) => void;
  onCancel: () => void;
}) {
  const [url, setUrl] = useState("");
  return (
    <>
      <div className="space-y-2">
        <Label htmlFor="cloud-url">Company address</Label>
        <Input
          id="cloud-url"
          placeholder="https://acme.opencompany.cloud"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
        {/*
          Worth saying before the first probe rather than after it: a tenant
          that has been idle is not running, and the platform starts it when a
          request arrives. The row says "Waking…" for as long as that takes,
          and an operator who has not been told that reads it as a hang.
        */}
        <p className="text-muted-foreground text-xs">
          A company hosted for you. If it has been idle it is started on the next
          request, so the first connection can take a few seconds.
        </p>
      </div>
      <DialogFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button disabled={!url.trim()} onClick={() => onAdd(url.trim())}>
          Add
        </Button>
      </DialogFooter>
    </>
  );
}

/** The address of a host running somewhere this application did not start it. */
function RemoteHost({
  onAdd,
  onCancel,
  desktop,
}: {
  onAdd: (baseUrl: string) => void;
  onCancel: () => void;
  desktop: boolean;
}) {
  const [url, setUrl] = useState("");
  return (
    <>
      <div className="space-y-2">
        <Label htmlFor="connection-url">Host URL</Label>
        <Input
          id="connection-url"
          placeholder="https://acme.example.com"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
        {/*
          The most likely support question this tab generates, answered where
          it is cheapest to answer. A browser reaches a gateway from *this*
          origin, so the gateway has to allow it — and there is no wildcard,
          because the session is a credential. The desktop proxies from Rust,
          where no origin is involved and none of this applies.
        */}
        {desktop ? null : (
          <p className="text-muted-foreground text-xs">
            A gateway you run yourself. It has to allow this console's address in{" "}
            <code>OPENCOMPANY_CORS_ORIGINS</code>, or your browser will block every
            reply.
          </p>
        )}
      </div>
      <DialogFooter>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button disabled={!url.trim()} onClick={() => onAdd(url.trim())}>
          Add
        </Button>
      </DialogFooter>
    </>
  );
}

/**
 * A host on another machine that is bound to loopback there, reached through a
 * tunnel this application opens.
 *
 * The destination leads and everything else has a default, because the shape
 * this is for is `acme-vps` — a `Host` alias out of `~/.ssh/config`. Anyone
 * with a bastion, a jump host or a non-default key has already written that
 * file, and a form that re-asks for its contents is one they will fill in
 * wrong.
 */
function SshHost({
  onAdd,
  onDone,
}: {
  onAdd: (target: SshTarget) => Promise<void>;
  onDone: () => void;
}) {
  const [destination, setDestination] = useState("");
  const [remotePort, setRemotePort] = useState(String(DEFAULT_REMOTE_PORT));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function add(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await onAdd({
        destination: destination.trim(),
        remotePort: Number(remotePort) || DEFAULT_REMOTE_PORT,
      });
      onDone();
    } catch (err) {
      // `ssh`'s own words, kept: "Host key verification failed" and
      // "Permission denied (publickey)" each name a specific thing to go and
      // fix, and a summary of either would name none of them.
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div className="space-y-2">
        <Label htmlFor="ssh-destination">Machine</Label>
        <Input
          id="ssh-destination"
          placeholder="acme-vps, or deploy@10.0.0.4"
          value={destination}
          onChange={(e) => setDestination(e.target.value)}
        />
        <Label htmlFor="ssh-remote-port">Port it serves on there</Label>
        <Input
          id="ssh-remote-port"
          inputMode="numeric"
          placeholder={String(DEFAULT_REMOTE_PORT)}
          value={remotePort}
          onChange={(e) => setRemotePort(e.target.value)}
        />
        <p className="text-muted-foreground text-xs">
          The host stays reachable only from that machine; this application
          forwards it over your own ssh. Your key has to be one ssh can use
          without asking — an agent key, or one with no passphrase.
        </p>
        {error ? <p className="text-destructive text-xs">{error}</p> : null}
      </div>
      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button disabled={busy || !destination.trim()} onClick={() => void add()}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : null}
          Connect
        </Button>
      </DialogFooter>
    </>
  );
}

/**
 * The hosts this machine runs: what is listening, what is not, and a name field
 * for one more.
 *
 * Stopped instances are listed rather than hidden, and this is the only place
 * they appear. A stopped instance has no address, so it cannot be a row in the
 * switcher — the menu lists connections, and a connection with nothing
 * listening at it is a permanent probe failure. Here it is a row with a Start
 * button, which is what it actually is.
 */
function LocalInstances({
  instances,
  onAdd,
  onStart,
  onStop,
}: {
  instances: LocalInstance[];
  onAdd: (label: string) => Promise<void>;
  onStart?: (id: string) => Promise<void>;
  onStop?: (id: string) => Promise<void>;
}) {
  const [label, setLabel] = useState("");
  /** Which instance has a command in flight, or `"new"` for the create. */
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /**
   * Runs one command, keeping its failure on screen.
   *
   * Every one of these can fail for a reason the operator has to read — most
   * often the data root being held by an `opencompany serve` in a terminal —
   * and a rejected promise with no `catch` is a console that silently does
   * nothing when the button is pressed.
   */
  async function run(key: string, action: () => Promise<void>): Promise<void> {
    setBusy(key);
    setError(null);
    try {
      await action();
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="space-y-4" data-testid="local-instances">
      <ul className="space-y-1">
        {instances.map((instance) => (
          <li
            key={instance.id}
            data-testid={`local-instance-${instance.id}`}
            data-running={instance.running}
            className="flex items-center justify-between gap-2 rounded-md border px-3 py-2"
          >
            <div className="min-w-0">
              <div className="truncate text-sm">{instance.label}</div>
              <div className="truncate text-xs text-muted-foreground">
                {instance.running ? instance.baseUrl : (instance.error ?? "Stopped")}
              </div>
            </div>
            {instance.running ? (
              <Button
                variant="ghost"
                size="sm"
                disabled={!onStop || busy !== null}
                onClick={() => void run(instance.id, () => onStop!(instance.id))}
              >
                {busy === instance.id ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Square className="size-4" />
                )}
                Stop
              </Button>
            ) : (
              <Button
                variant="ghost"
                size="sm"
                disabled={!onStart || busy !== null}
                onClick={() => void run(instance.id, () => onStart!(instance.id))}
              >
                {busy === instance.id ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <Play className="size-4" />
                )}
                Start
              </Button>
            )}
          </li>
        ))}
      </ul>

      <div className="space-y-2">
        <Label htmlFor="local-instance-label">Name</Label>
        <Input
          id="local-instance-label"
          placeholder="Acme"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
        <p className="text-xs text-muted-foreground">
          A new host on this computer, with its own data and its own companies. Nothing is shared
          with the ones above.
        </p>
      </div>

      {error ? (
        <p data-testid="local-instance-error" className="text-xs text-destructive">
          {error}
        </p>
      ) : null}

      <DialogFooter>
        <Button
          data-testid="local-instance-add"
          disabled={!label.trim() || busy !== null}
          onClick={() =>
            void run("new", async () => {
              await onAdd(label.trim());
              setLabel("");
            })
          }
        >
          {busy === "new" ? <Loader2 className="size-4 animate-spin" /> : null}
          Run it here
        </Button>
      </DialogFooter>
    </div>
  );
}
