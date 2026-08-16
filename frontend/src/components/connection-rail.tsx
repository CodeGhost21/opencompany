// The hosts this console is connected to, down the left edge.
//
// ## The rule this component exists to keep
//
// The rail is a **filter over N live things**, not a selector that makes one
// live. Clicking a row changes which console is on screen and nothing else: no
// connection is torn down, no stream is closed, no storage is re-scoped.
//
// That is the difference between this and block/buzz's workspace rail, which
// looks the same and is not. Switching there is a stateful *apply* — it
// re-scopes the retention database, re-resolves identity and restarts the
// managed agents — because its app state holds one `relay_url_override`. The
// rail is the visible part; the singleton behind it is why buzz cannot hold two
// workspaces at once.
//
// So: selection lives in `App`'s local state, never in the registry, and no
// code path here mutates a connection.

import { Plus, Building2 } from "lucide-react";
import { useState } from "react";

import { isDesktopRuntime } from "@/api/transport";
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
import type { Connection, ConnectionId, ConnectionStatus } from "@/connections/types";

interface Props {
  connections: Connection[];
  selected: ConnectionId | null;
  onSelect: (id: ConnectionId) => void;
  onAdd: (baseUrl: string) => void;
  /**
   * Whether this is a hub deployment, which draws the rail at any count.
   *
   * Passed rather than read from config here, so this component keeps taking
   * everything it renders from its props and stays drivable from a test
   * without a global to arrange.
   */
  hub?: boolean;
}

/** How a connection's state reads, and what colour says so. */
const STATUS_COPY: Record<ConnectionStatus, { label: string; dot: string }> = {
  connecting: { label: "Connecting…", dot: "bg-muted-foreground/50" },
  live: { label: "Connected", dot: "bg-status-done" },
  degraded: { label: "Partly available", dot: "bg-status-blocked" },
  down: { label: "Unreachable", dot: "bg-destructive" },
  unauthenticated: { label: "Sign-in needed", dot: "bg-status-blocked" },
};

/**
 * How wide the rail is, as a CSS length — `w-14`.
 *
 * Exported because the app shell's sidebar is `position: fixed` and therefore
 * positions against the viewport, not against the flex column it lives in. It
 * has to be told how far in that column starts, and this is the only place
 * that knows.
 */
export const CONNECTION_RAIL_WIDTH = "3.5rem";

/**
 * Whether the rail is on screen.
 *
 * One host is the ordinary web deployment, and a rail listing exactly one
 * thing is furniture. It appears when there is a choice to make.
 *
 * **The desktop always draws it**, whatever the count. The old rule made an
 * exception only for zero, on the reasoning that a browser always has its
 * bootstrap connection so a real zero could only happen in the desktop — and
 * that a desktop holding nothing needs the "+" the rail carries. Issue #613
 * moved where that bites: the desktop no longer writes a same-origin bootstrap
 * row, so a desktop whose embedded host *did* start now holds exactly **one**
 * connection, which is its ordinary state rather than a rare one. Under the
 * old rule that state drew no rail, and the "+" is the only way to reach a
 * second host — so the working case became the one with no way out of it,
 * which is the dead end the zero exception existed to prevent.
 *
 * **A hub always draws it too**, for the same reason and by the same argument.
 * A hub has no bootstrap connection either (its origin serves assets, not a
 * host), so it holds exactly the hosts someone added — and at zero the "+" in
 * this rail is the only way to add the first one. Hiding the rail at one host
 * would strand a hub with a single host as surely as it stranded the desktop.
 *
 * An ordinary same-origin browser console is unaffected: neither
 * `isDesktopRuntime()` nor `hub` is true there, so one host still draws
 * nothing.
 *
 * Exported so the shell can offset the sidebar by the same condition that
 * draws the rail — two copies of this rule is how the sidebar ends up clipped
 * under it again.
 */
export function connectionRailVisible(count: number, hub = false): boolean {
  return count >= 2 || hub || isDesktopRuntime();
}

export function ConnectionRail({ connections, selected, onSelect, onAdd, hub }: Props) {
  const [adding, setAdding] = useState(false);
  const [url, setUrl] = useState("");

  if (!connectionRailVisible(connections.length, hub)) return null;

  return (
    <nav
      aria-label="Connected hosts"
      data-testid="connection-rail"
      // `relative z-50`: the app shell's own sidebar is positioned, and without a
      // stacking context of its own the rail sits underneath it — visible, but
      // with every click landing on the sidebar behind.
      className="relative z-50 flex w-14 shrink-0 flex-col items-center gap-2 border-r bg-sidebar py-3"
    >
      {connections.map((connection) => {
        const status = STATUS_COPY[connection.status];
        return (
          <button
            key={connection.id}
            type="button"
            data-testid={`connection-row-${connection.id}`}
            data-status={connection.status}
            aria-current={connection.id === selected}
            // Native `title`, not a tooltip component: the status has to be
            // readable without hovering anyway (see the data attributes, which
            // are what the degrade test asserts on), and this keeps a row that
            // must render while its host is unreachable free of extra machinery.
            title={`${connection.label} — ${status.label}${
              connection.error ? `\n${connection.error}` : ""
            }`}
            onClick={() => onSelect(connection.id)}
            className={`relative flex size-10 items-center justify-center rounded-lg border transition ${
              connection.id === selected
                ? "border-primary bg-primary/10"
                : "border-transparent hover:bg-muted"
            }`}
          >
            <Building2 className="size-4" />
            <span
              data-testid={`connection-status-${connection.id}`}
              className={`absolute -bottom-0.5 -right-0.5 size-2.5 rounded-full ring-2 ring-sidebar ${status.dot}`}
            />
            <span className="sr-only">
              {connection.label}: {status.label}
            </span>
          </button>
        );
      })}

      <Button
        variant="ghost"
        size="icon"
        aria-label="Add a host"
        title="Add a host"
        data-testid="connection-add"
        onClick={() => setAdding(true)}
      >
        <Plus className="size-4" />
      </Button>

      <Dialog open={adding} onOpenChange={setAdding}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add a host</DialogTitle>
            <DialogDescription>
              The base URL of an OpenCompany host. It stays connected alongside the others.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="connection-url">Host URL</Label>
            <Input
              id="connection-url"
              placeholder="https://acme.example.com"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setAdding(false)}>
              Cancel
            </Button>
            <Button
              disabled={!url.trim()}
              onClick={() => {
                onAdd(url.trim());
                setUrl("");
                setAdding(false);
              }}
            >
              Add
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </nav>
  );
}
