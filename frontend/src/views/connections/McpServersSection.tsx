import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openOutward } from "@/lib/external-links";
import {
  AlertTriangle,
  Info,
  Loader2,
  Plus,
  Search,
  Server,
} from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import {
  discoverMcpTools,
  listMcpServers,
  removeMcpServer,
  startMcpOAuth,
  testMcpServer,
  updateMcpServer,
} from "@/api/mcp";
import {
  connectMcpRegistryServer,
  disconnectMcpRegistryServer,
  getMcpRegistryEntry,
  installMcpRegistryEntry,
  uninstallMcpRegistryServer,
  updateMcpRegistryEnv,
  type McpCatalogueEntry,
} from "@/api/mcp-registry";
import {
  ApiError,
  type McpHealth,
  type McpServer,
  type McpSource,
  type McpStatus,
  type McpToolInfo,
  type RosterAgent,
} from "@/api/types";
import { type McpBridgeState, mcpBridgeState } from "@/lib/mcp-bridge";
import {
  missingEnvKeys,
  mcpRowControls,
  REGISTRY_OAUTH_UNSUPPORTED_NOTICE,
  REGISTRY_UNWIRED_NOTICE,
  registryOauthUnsupported,
  registryOutage,
} from "@/lib/mcp-registry";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { useHashParam } from "@/hooks/use-hash-param";
import { useMcpDirectorySearch } from "@/views/connections/McpRegistryBrowser";
import { McpAddServerDialog } from "@/views/connections/McpAddServerDialog";
import {
  McpDirectoryRow,
  McpGroupRow,
  McpServerRow,
  McpServerTable,
  type McpRowActions,
  type PrimaryAction,
} from "@/views/connections/McpServerTable";
import { McpServerPage } from "@/views/mcp/McpServerPage";

/**
 * What a server's health entitles its row to offer (issues #1260, #1270).
 *
 * A rule rather than inline conditions, because the two OAuth states differ by
 * exactly one thing an operator cannot see: whether the server advertises
 * dynamic client registration. `oauth_required` means the server asked for
 * OAuth; only the host knows whether this console can complete one, and it says
 * so by sending `static_token_required` instead. Reading them as the same state
 * is what put a Sign in button on a Slack row that could never sign in.
 *
 * Issue #1270 added a third control and a reason the hint alone cannot decide
 * between them. A **directory install** keeps its credentials as named env
 * values in the host's registry store, and its row's `name` is a display slug
 * addressing no List A declaration — so both List A controls are wrong for it
 * twice over: `POST …/oauth/start` and `PUT …/mcp/servers/{name}` answer *no
 * MCP server named …*, and the value they collect would be written to a store
 * this server is not dialled from. Such a row gets `rotate_env`, which is
 * `PUT …/mcp/registry/{serverId}/env`.
 *
 * `row.status` is consulted only for that case, and it has to be: the host's
 * registry projection emits a stable `authHint` only when the upstream
 * connection reported one, so a directory install refused for want of a
 * credential can arrive as `needs_config` with no hint at all. The hint still
 * has the last word on a registry row: `oauth_required` names a credential the
 * env form neither collects nor stores, and no route here can start that
 * sign-in, so such a row is offered no credential control at all. List A's
 * mapping below is untouched — it is still a function of the hint and nothing
 * else.
 */
export function credentialAffordance(
  authHint: string | undefined,
  row?: { source: McpSource; status?: McpStatus },
): "sign_in" | "add_token" | "rotate_env" | "none" {
  if (row?.source === "registry") {
    if (authHint === "oauth_required") return "none";
    return row.status === "needs_config" ? "rotate_env" : "none";
  }
  switch (authHint) {
    case "oauth_required":
      return "sign_in";
    case "static_token_required":
    // A plain credential prompt wants the same field; it simply never had a
    // sign-in button to withdraw.
    case "credential_required":
    // A refused credential is replaced through that same field.
    case "token_rejected":
      return "add_token";
    default:
      return "none";
  }
}

type McpLoad = "loading" | "ready" | "unavailable" | "error";
/**
 * The fields a directory install's credential rotation is asking for.
 *
 * They come from the *directory*, not from the row: `GET …/mcp/servers` reports
 * only whether a credential is stored (`authConfigured`), never which keys hold
 * it, so the names have to be re-read from the catalogue entry the install came
 * from. That is also why this can fail while the rotation route itself is
 * perfectly healthy — a directory outage costs the field names, so the form
 * says so instead of guessing at them.
 */
type EnvFields =
  | { kind: "loading" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; keys: string[] };
type ToolsState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "unwired" }
  | { kind: "error"; message: string }
  | { kind: "ready"; tools: McpToolInfo[] };

/**
 * One server's sign-in, while the operator is still in the other tab.
 */
interface SignInFlight {
  authorizeUrl: string;
  /** The tab could not be created — a blocked popup, or a desktop webview. */
  blocked: boolean;
  checkedAtMillis: number;
  timedOut: boolean;
}

/**
 * How the page around this section frames it.
 *
 * - `inline` — a section among others. The page has plenty else to show, so a
 *   host with no MCP surface renders nothing here at all. The default, and
 *   since the Connections split no page embeds it.
 * - `standalone` — the whole page is this section (`#/connections/mcp`). The
 *   page supplies the heading, and a host with no MCP surface says so, because
 *   the alternative is a page that is simply blank.
 */
export type McpSectionChrome = "inline" | "standalone";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** Whether this viewer may add, edit or remove servers (issue #403). */
  canManage: boolean;
  chrome?: McpSectionChrome;
  /** The roster, for the per-teammate lens on a server's permissions. */
  agents?: RosterAgent[];
}

/**
 * The company's MCP tool servers: one searchable list, and the directory in it.
 *
 * A table: the four things an operator scans for, one labelled action per row
 * and everything else behind an overflow. The one search field covers this
 * company *and* the directory.
 *
 * This is the console's **only** MCP surface, and it has exactly one caller:
 * [`McpServersView`](../McpServersView.tsx), the `#/connections/mcp` page.
 * Settings used to carry a second implementation of this screen against an API
 * no host has ever served, which crashed on open — a second surface is how the
 * two came to disagree, so there is one (issue #414).
 */
export function McpServersSection({
  client,
  company,
  canManage,
  chrome = "inline",
  agents = [],
}: Props) {
  const [load, setLoad] = useState<McpLoad>("loading");
  // Whether the agent-side MCP bridge is compiled into this host (issue #567).
  // Starts `unknown` so nothing is claimed before the capability read lands.
  const [bridge, setBridge] = useState<McpBridgeState>("unknown");
  // Whether a `needs_approval` mode parks anything on this host. `undefined`
  // until the capability read answers, and left that way when it cannot: the
  // notice claims nothing on a host that has not said.
  const [approvalsPark, setApprovalsPark] = useState<boolean | undefined>(
    undefined,
  );
  const [servers, setServers] = useState<McpServer[]>([]);
  // The name of the row currently mutating. Every mutating handler serialises on
  // it with an `if (busy) return`, so while one is in flight the controls on ALL
  // rows disable, not just the busy one (issue #1475): the guard used to be
  // invisible on the other rows, which accepted clicks and silently did nothing.
  const [busy, setBusy] = useState<string | null>(null);
  const [tools, setTools] = useState<Record<string, ToolsState>>({});
  // Live health from an on-demand re-check, overriding the persisted badge.
  const [tested, setTested] = useState<Record<string, McpHealth>>({});
  // In-flight OAuth sign-in poll timers, keyed by server name. A row with a live
  // timer is still "signing in" even after its `busy` flag clears, so a repeat
  // click can't spawn a second overlapping poll.
  const pollTimers = useRef<Record<string, number>>({});
  const [signIns, setSignIns] = useState<Record<string, SignInFlight>>({});
  // Opens the detail panel on the permissions section. Kept as its own key so
  // links already written against it keep landing where they meant to.
  const [permissionsFor, setPermissionsFor] = useHashParam("permissions");
  // The server whose detail page is open. A name rather than the row itself, so
  // an open page re-derives from `servers` after a refresh.
  const [openedName, setOpenedName] = useHashParam("server");
  const opened = openedName ?? permissionsFor;
  const closeDetail = () => {
    setOpenedName(null);
    setPermissionsFor(null);
  };
  // One field over both halves. The company's own servers are filtered locally,
  // so searching them costs nothing; the directory is not called until something
  // is typed.
  const [query, setQuery] = useState("");
  const searchBox = useRef<HTMLInputElement | null>(null);
  const [adding, setAdding] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);

  /**
   * The server whose credential field is open, and its draft value (issue #1260).
   *
   * Per-row rather than a shared field: the add dialog's Token creates a *new*
   * server, so pointing an operator at it to fix an existing one would have them
   * add a second copy. The host has accepted a credential rotation on
   * `PUT …/mcp/servers/{name}` all along — this is the control that was missing.
   */
  const [credentialFor, setCredentialFor] = useState<string | null>(null);
  const [credentialDraft, setCredentialDraft] = useState("");
  /**
   * The registry row whose credential-rotation form is open, and the fields it
   * is showing (issue #1270).
   *
   * Separate state from `credentialFor` because the two collect different things
   * for different stores: List A's is one bearer token written to this company's
   * secret store, a directory install's is a set of *named* env values written to
   * the host's registry store. Which of the two a row offers is
   * `credentialAffordance`'s decision and nothing else's.
   */
  const [envFor, setEnvFor] = useState<string | null>(null);
  const [envFields, setEnvFields] = useState<EnvFields>({ kind: "loading" });
  const [envDraft, setEnvDraft] = useState<Record<string, string>>({});
  const [envError, setEnvError] = useState<string | null>(null);
  // Set by the unmount cleanup below. A sign-in poll that is mid-`await` when
  // this component goes away has already removed its own timer entry, so the
  // cleanup has nothing left to cancel — it checks this instead of re-arming.
  const unmounted = useRef(false);
  // Which company's answers are still wanted, bumped whenever the scope changes.
  // `refresh` reads it before asking and again on arrival, and drops the answer
  // if it moved: without this, switching company while the list request is in
  // flight lets the older response resolve last and write one company's servers
  // into another company's view.
  const scope = useRef(0);
  // Removal is irreversible and takes the server's stored credential with it, so
  // it is asked rather than done on the press.
  const [pendingRemoval, setPendingRemoval] = useState<McpServer | null>(null);
  // Bumped per server whenever a probe rewrote its stored tool inventory, so an
  // open permissions panel re-reads instead of rendering the pre-probe list.
  const [probedAt, setProbedAt] = useState<Record<string, number>>({});

  const directory = useMcpDirectorySearch(client, company, query);

  const refresh = useCallback(async () => {
    const mine = scope.current;
    try {
      const list = await listMcpServers(client, company);
      if (scope.current !== mine) return;
      setServers(list);
      setLoad("ready");
    } catch (err) {
      if (scope.current !== mine) return;
      // A 404 is a host with no MCP surface: a fact about the build, not a
      // failure. Anything else (offline, 5xx, a body that wasn't the list the
      // route promises) means we do not know what this company has, and saying
      // "no MCP here" would be a claim we cannot make (issue #414).
      setLoad(
        err instanceof ApiError && err.status === 404 ? "unavailable" : "error",
      );
    }
  }, [client, company]);

  useEffect(() => {
    scope.current += 1;
    setLoad("loading");
    void refresh();
  }, [refresh]);

  // Whether this build can actually run what this screen manages (issue #567).
  // Read separately from the server list, and deliberately not fatal to it: the
  // list is the screen's job, the build state is a caption on it, so a host that
  // cannot answer the capability read still gets a working MCP tab — it just
  // gets no claim about the bridge.
  useEffect(() => {
    let alive = true;
    client
      .capabilityStatus(company)
      .then((status) => {
        if (alive) {
          setBridge(mcpBridgeState(status));
          setApprovalsPark(
            typeof status.approvalsPark === "boolean"
              ? status.approvalsPark
              : undefined,
          );
        }
      })
      // A host with no `…/capabilities` surface 404s. Unknown, not absent.
      .catch(() => {
        if (alive) {
          setBridge("unknown");
          setApprovalsPark(undefined);
        }
      });
    return () => {
      alive = false;
    };
  }, [client, company]);

  // Cancel any in-flight sign-in polls when the view unmounts so their timers
  // don't fire against a torn-down component, and tell a poll that is currently
  // between its own `delete` and its next arm that there is nothing to come
  // back to.
  useEffect(() => {
    const timers = pollTimers.current;
    return () => {
      unmounted.current = true;
      for (const id of Object.values(timers)) window.clearTimeout(id);
    };
  }, []);

  async function test(server: McpServer) {
    if (busy) return;
    setBusy(server.name);
    try {
      const health = await testMcpServer(client, company, server.name);
      setTested((t) => ({ ...t, [server.name]: health }));
      setProbedAt((p) => ({ ...p, [server.name]: Date.now() }));
    } catch (err) {
      if (err instanceof ApiError && err.code === "not_wired") {
        toast.message(
          "Live testing isn't enabled in this build (the agent harness is off).",
        );
      } else {
        toast.error(
          err instanceof ApiError ? err.message : "Couldn't test the server.",
        );
      }
    } finally {
      setBusy(null);
    }
  }

  /**
   * Rotate one server's credential from its own row (issue #1260).
   *
   * Re-tests on success rather than trusting the write: the whole point of the
   * flow is that the operator does not know whether the token is the right one
   * until the server answers, and a silent save would leave the amber badge
   * sitting there with no way to tell "wrong token" from "not saved".
   */
  async function saveCredential(server: McpServer) {
    if (busy) return;
    const token = credentialDraft.trim();
    if (!token) return;
    setBusy(server.name);
    try {
      // Rotate the credential VALUE only — never the auth scheme (issue #1464).
      // The read model carries no `authKind`, so the console cannot know whether
      // this server was added as a bearer token, an `X-Api-Key:` header or a
      // `?api_key=` query parameter. Sending `authKind: "bearer"` here silently
      // rewrote a header/query server to bearer, after which it rejected every
      // request. Omitting the field leaves the host's stored scheme untouched.
      await updateMcpServer(client, company, server.name, { token });
      setCredentialFor(null);
      setCredentialDraft("");
      await refresh();
      await test(server);
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Couldn't save the token.",
      );
    } finally {
      setBusy(null);
    }
  }

  /** Stop watching for a sign-in the operator has given up on. */
  function cancelSignIn(name: string) {
    const timer = pollTimers.current[name];
    if (timer !== undefined) window.clearTimeout(timer);
    delete pollTimers.current[name];
    setSignIns(({ [name]: _dropped, ...rest }) => rest);
  }

  // Browser OAuth sign-in (issue #90): open the authorization URL in a new tab,
  // then poll the server's health until it flips to `ok` (the host stores the
  // token on its callback route) so the amber badge turns green on its own. The
  // row holds the waiting state throughout, because a toast fired at the moment
  // the operator acts is gone long before the poll is.
  async function signIn(server: McpServer) {
    // Guard both the shared `busy` flag and a per-server poll already in flight:
    // the poll outlives `busy`, so without the second check a repeat click would
    // spawn a second overlapping sign-in (duplicate token exchange + toasts).
    if (busy || pollTimers.current[server.name] !== undefined) return;
    setBusy(server.name);
    try {
      const { authorizeUrl } = await startMcpOAuth(
        client,
        company,
        server.name,
      );
      // See `OAuthView`: in the desktop shell a webview cannot create this tab,
      // so the authorization page never opens.
      let opened = openOutward(authorizeUrl);
      if (!opened) {
        opened = window.open(authorizeUrl, "_blank", "noopener,noreferrer") !== null;
      }
      setSignIns((s) => ({
        ...s,
        [server.name]: {
          authorizeUrl,
          blocked: !opened,
          checkedAtMillis: Date.now(),
          timedOut: false,
        },
      }));
      // Poll for completion for up to ~2 minutes; stop as soon as it's healthy.
      const deadline = Date.now() + 120_000;
      const poll = async () => {
        // The entry goes before the probe, so from here to the arm at the bottom
        // this poll is invisible to the unmount cleanup — which is why every step
        // below re-checks.
        delete pollTimers.current[server.name];
        if (unmounted.current) return;
        if (Date.now() > deadline) {
          setSignIns((s) => {
            const flight = s[server.name];
            return flight ? { ...s, [server.name]: { ...flight, timedOut: true } } : s;
          });
          return;
        }
        try {
          const health = await testMcpServer(client, company, server.name);
          if (unmounted.current) return;
          setTested((t) => ({ ...t, [server.name]: health }));
          setSignIns((s) => {
            const flight = s[server.name];
            return flight
              ? { ...s, [server.name]: { ...flight, checkedAtMillis: Date.now() } }
              : s;
          });
          if (health.status === "ok") {
            cancelSignIn(server.name);
            toast.success(`Signed in to ${server.name}.`);
            await refresh();
            return;
          }
        } catch {
          // Ignore transient probe errors while the operator finishes sign-in.
        }
        if (unmounted.current) return;
        pollTimers.current[server.name] = window.setTimeout(
          () => void poll(),
          2_000,
        );
      };
      pollTimers.current[server.name] = window.setTimeout(
        () => void poll(),
        2_000,
      );
    } catch (err) {
      if (err instanceof ApiError && err.code === "not_wired") {
        toast.message(
          "OAuth sign-in isn't enabled in this build (the agent harness is off).",
        );
      } else {
        toast.error(
          err instanceof ApiError ? err.message : "Couldn't start sign-in.",
        );
      }
    } finally {
      setBusy(null);
    }
  }

  async function toggle(server: McpServer, enabled: boolean) {
    if (busy) return;
    setBusy(server.name);
    try {
      await updateMcpServer(client, company, server.name, { enabled });
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Couldn't update the server.",
      );
    } finally {
      setBusy(null);
    }
  }

  /**
   * Remove a server through whichever route owns it (issue #1270).
   *
   * The dispatch is [`mcpRowControls`](@/lib/mcp-registry)'s, not a condition
   * here, because the two routes key on different things and neither accepts the
   * other's key: List A deletes by `name`, a directory install by its
   * `serverId`. A registry row's `name` is a slug the host mints for the merged
   * view — sending it to `DELETE …/mcp/servers/{name}` addresses a declaration
   * that does not exist, and on an unlucky slug collision would address someone
   * else's.
   */
  async function remove(server: McpServer) {
    if (busy) return;
    const removal = mcpRowControls(
      server,
      tested[server.name] ?? server.health,
    ).removal;
    if (removal.kind === "none") return;
    setBusy(server.name);
    try {
      if (removal.kind === "install") {
        await uninstallMcpRegistryServer(client, company, removal.serverId);
      } else {
        await removeMcpServer(client, company, removal.name);
      }
      toast.success(`Removed ${server.name}.`);
      await refresh();
    } catch (err) {
      if (err instanceof ApiError && err.code === "not_wired") {
        toast.message(REGISTRY_UNWIRED_NOTICE);
      } else {
        toast.error(
          err instanceof ApiError ? err.message : "Couldn't remove the server.",
        );
      }
    } finally {
      setBusy(null);
    }
  }

  /**
   * Dial or drop a directory install's session (issue #1270).
   *
   * A disconnect keeps the install and its stored credentials — it closes the
   * session, it does not uninstall — so the two are separate controls rather
   * than one destructive toggle. A refused connection is **not** an error: the
   * host says so, and a server that answers "needs a credential" has told the
   * operator exactly what to do next.
   */
  async function lifecycle(
    server: McpServer,
    direction: "connect" | "disconnect",
  ) {
    if (busy || !server.serverId) return;
    setBusy(server.name);
    try {
      const res =
        direction === "connect"
          ? await connectMcpRegistryServer(client, company, server.serverId)
          : await disconnectMcpRegistryServer(client, company, server.serverId);
      const after = res.test;
      if (after) setTested((t) => ({ ...t, [server.name]: after }));
      // No state came back: drop any stale override so the row falls back to the
      // health the refresh below is about to bring.
      else setTested(({ [server.name]: _dropped, ...rest }) => rest);
      await refresh();
    } catch (err) {
      if (err instanceof ApiError && err.code === "not_wired") {
        toast.message(REGISTRY_UNWIRED_NOTICE);
      } else {
        toast.error(
          err instanceof ApiError
            ? err.message
            : `Couldn't ${direction} ${server.name}.`,
        );
      }
    } finally {
      setBusy(null);
    }
  }

  /**
   * Install a directory entry, with no credential.
   *
   * Every tool it exposes starts un-granted either way, and a credential the
   * entry needs is collected on the row the install lands as — which is the one
   * control that writes to the store that install is actually dialled from.
   */
  async function install(entry: McpCatalogueEntry) {
    if (installing) return;
    setInstalling(entry.qualifiedName);
    try {
      const res = await installMcpRegistryEntry(client, company, {
        qualifiedName: entry.qualifiedName,
      });
      // An install that lands "needs a credential" is NOT a rollback — the host
      // says so explicitly — so it is reported where the operator can act on it
      // rather than dressed up as a failed install.
      if (res.test && res.test.status !== "ok") {
        toast.message(
          `Installed ${entry.displayName}. ${res.test.message} Add its credential from its row.`,
        );
      } else {
        toast.success(`Installed ${entry.displayName}. ${res.note}`);
      }
      await refresh();
    } catch (err) {
      const outage = registryOutage(err);
      toast.error(
        outage.kind === "unwired" ? REGISTRY_UNWIRED_NOTICE : outage.message,
      );
    } finally {
      setInstalling(null);
    }
  }

  /**
   * Open a directory install's credential rotation, reading its field names from
   * the catalogue entry it was installed from.
   *
   * The row cannot supply them: the merged read reports only that *a* credential
   * is stored. See {@link EnvFields}.
   */
  async function openEnvRotation(server: McpServer) {
    setEnvDraft({});
    setEnvError(null);
    setEnvFor(server.name);
    if (!server.qualifiedName) {
      setEnvFields({
        kind: "failed",
        message:
          "This install doesn't name a directory entry, so its credential fields are unknown.",
      });
      return;
    }
    setEnvFields({ kind: "loading" });
    try {
      const detail = await getMcpRegistryEntry(
        client,
        company,
        server.qualifiedName,
      );
      setEnvFields({ kind: "ready", keys: detail.requiredEnvKeys });
    } catch (err) {
      const outage = registryOutage(err);
      setEnvFields({
        kind: "failed",
        message:
          outage.kind === "unwired"
            ? REGISTRY_UNWIRED_NOTICE
            : `${outage.message} Its credential fields are named in the directory, so they can't be read right now.`,
      });
    }
  }

  /**
   * Rotate a directory install's credentials.
   *
   * Write-only in both directions, exactly like List A's token: the values go to
   * `PUT …/mcp/registry/{serverId}/env` and come back only as the row's
   * `authConfigured` flag. The host merges the supplied keys over the stored ones
   * and reconnects, so the post-write connection state is the answer to "was that
   * the right credential" and is recorded as this row's live health.
   */
  async function saveEnvRotation(server: McpServer, keys: string[]) {
    if (busy || !server.serverId) return;
    const missing = missingEnvKeys(keys, envDraft);
    if (missing.length > 0) {
      setEnvError(`Fill in ${missing.join(", ")} before saving.`);
      return;
    }
    setBusy(server.name);
    setEnvError(null);
    try {
      const res = await updateMcpRegistryEnv(
        client,
        company,
        server.serverId,
        envDraft,
      );
      const after = res.test;
      if (after) setTested((t) => ({ ...t, [server.name]: after }));
      if (after && after.status !== "ok") {
        setEnvError(
          after.message.trim() ||
            `${server.name} still isn't connected with those credentials.`,
        );
        await refresh();
        return;
      }
      setEnvFor(null);
      setEnvDraft({});
      await refresh();
    } catch (err) {
      setEnvError(
        err instanceof ApiError
          ? err.message
          : "Couldn't save those credentials.",
      );
    } finally {
      setBusy(null);
    }
  }

  async function discover(server: McpServer) {
    // Toggle closed if already shown.
    if (tools[server.name]?.kind === "ready") {
      setTools((t) => ({ ...t, [server.name]: { kind: "idle" } }));
      return;
    }
    setTools((t) => ({ ...t, [server.name]: { kind: "loading" } }));
    try {
      const list = await discoverMcpTools(client, company, server.name);
      setTools((t) => ({
        ...t,
        [server.name]: { kind: "ready", tools: list },
      }));
    } catch (err) {
      if (err instanceof ApiError && err.code === "not_wired") {
        setTools((t) => ({ ...t, [server.name]: { kind: "unwired" } }));
      } else {
        setTools((t) => ({
          ...t,
          [server.name]: {
            kind: "error",
            message:
              err instanceof ApiError ? err.message : "Discovery failed.",
          },
        }));
      }
    }
  }

  const actions: McpRowActions = {
    onOpen: (name) => setOpenedName(name),
    onSignIn: (server) => void signIn(server),
    onAddToken: (server) => {
      setCredentialDraft("");
      setCredentialFor(server.name);
    },
    onRotateEnv: (server) => void openEnvRotation(server),
    onLifecycle: (server, direction) => void lifecycle(server, direction),
    onTest: (server) => void test(server),
    onTools: (server) => void discover(server),
    onPermissions: (name) => setPermissionsFor(name),
    onToggle: (server, enabled) => void toggle(server, enabled),
    onRemove: (server) => setPendingRemoval(server),
  };

  // Re-derived from the list every render rather than captured on click, so the
  // open dialog reflects the last refresh — a toggle, a completed sign-in or a
  // removal all reach it without a second copy of the row to keep in step.
  const removalDialog = (
    <AlertDialog
      open={pendingRemoval !== null}
      onOpenChange={(open) => !open && setPendingRemoval(null)}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove {pendingRemoval?.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            Its agents stop seeing this server&apos;s tools on their next turn,
            and the stored credential goes with it — a token is never shown
            again, so adding the server back means pasting a new one.
          </AlertDialogDescription>
          <AlertDialogDescription>
            Its per-tool permissions are removed too.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy !== null}>Keep it</AlertDialogCancel>
          <AlertDialogAction
            disabled={busy !== null}
            onClick={() => {
              const server = pendingRemoval;
              setPendingRemoval(null);
              if (server) void remove(server);
            }}
          >
            Remove
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );

  const openedServer = useMemo(
    () => servers.find((s) => s.name === opened) ?? null,
    [servers, opened],
  );

  const term = query.trim().toLowerCase();
  const matches = useMemo(
    () =>
      term === ""
        ? servers
        : servers.filter((s) =>
            [s.name, s.description ?? "", s.probedTitle ?? "", s.endpoint].some(
              (field) => field.toLowerCase().includes(term),
            ),
          ),
    [servers, term],
  );

  if (load === "unavailable") {
    if (chrome === "inline") return null;
    return (
      <Alert data-testid="mcp-unavailable">
        <Info className="size-4" />
        <AlertTitle>MCP servers aren&apos;t wired on this host</AlertTitle>
        <AlertDescription>
          This host serves no MCP routes, so there is nothing to manage here yet.
        </AlertDescription>
      </Alert>
    );
  }

  if (openedServer !== null) {
    return (
      <>
        <McpServerPage
          client={client}
          company={company}
          server={openedServer}
          health={tested[openedServer.name] ?? openedServer.health}
          canManage={canManage}
          bridge={bridge}
          approvalsPark={approvalsPark}
          agents={agents}
          reloadKey={probedAt[openedServer.name] ?? 0}
          focusPermissions={openedName === null && permissionsFor !== null}
          onDisconnect={
            mcpRowControls(
              openedServer,
              tested[openedServer.name] ?? openedServer.health,
            ).removal.kind === "none"
              ? null
              : () => setPendingRemoval(openedServer)
          }
          onBack={closeDetail}
        />
        {removalDialog}
      </>
    );
  }

  return (
    <section className="space-y-4">
      {/* `h2` in both chromes, and it lands one level under the page's `h1`
          either way (issue #1392). `test/unit/page-section-heading-level.test.ts`
          pins that pairing: heading at `h3` under that `h1` would read to a
          screen reader as a subsection of a section that does not exist. */}
      <div className="flex flex-wrap items-center gap-2">
        {chrome === "inline" && (
          <Server className="size-4 text-muted-foreground" />
        )}
        <h2 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {chrome === "inline" ? "MCP Servers" : "Your servers"}
        </h2>
        <span className="flex-1" />
        {load === "ready" && (servers.length > 0 || term !== "") && (
          <div className="relative min-w-0 flex-1 sm:max-w-xs">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              ref={searchBox}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search your servers and the directory…"
              aria-label="Search your servers and the directory"
              data-testid="mcp-search"
              className="h-8 pl-8"
            />
          </div>
        )}
        {canManage && load === "ready" && (
          <Button
            size="sm"
            data-testid="mcp-add-open"
            onClick={() => setAdding(true)}
          >
            <Plus className="size-4" />
            Add server
          </Button>
        )}
      </div>

      {/* Issue #567: this screen's routes ship in every build, the agent-side
          bridge does not. Said before the list rather than per row, because it is
          a fact about the deployment and not about any one server — and said only
          on an explicit `false`, never on a host that stayed silent. */}
      {bridge === "absent" && (
        <Alert data-testid="mcp-bridge-absent">
          <AlertTriangle className="size-4" />
          <AlertTitle>
            No agent can use tool servers in this deployment
          </AlertTitle>
          <AlertDescription>
            The MCP bridge isn&apos;t compiled into this build, so servers added
            here are stored and can be probed, but no agent ever receives their
            tools. The configuration survives — rebuild this deployment with the{" "}
            <code className="font-mono">mcp</code> feature and the servers below
            start reaching agents on the next turn.
          </AlertDescription>
        </Alert>
      )}

      {load === "error" ? (
        // Not an empty list: an empty list is a company with no tool servers, and
        // this host did not tell us that (issue #414).
        <>
          <Alert variant="destructive" data-testid="mcp-load-error">
            <AlertTriangle className="size-4" />
            <AlertTitle>
              Couldn&apos;t load this company&apos;s MCP servers
            </AlertTitle>
            <AlertDescription>
              The host didn&apos;t answer with its server list, so what is
              installed is unknown. Reload to try again.
            </AlertDescription>
          </Alert>
          <p className="text-xs text-muted-foreground">
            The directory half of the search still works — it reads the
            directory, not this company. There is nothing to add to or search
            here until the list can be read.
          </p>
        </>
      ) : load === "loading" ? (
        <Skeleton className="h-24 rounded-xl" />
      ) : servers.length === 0 && term === "" ? (
        <Card>
          <CardContent className="space-y-2">
            <p className="text-sm font-medium">No tool servers yet</p>
            <p className="text-sm text-muted-foreground">
              An MCP server gives your agents tools they do not have natively — a
              Notion workspace, a Linear board, an internal database. Add one by
              URL, or install one from the public directory.
            </p>
            {canManage && (
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <Button size="sm" onClick={() => setAdding(true)}>
                  <Plus className="size-4" />
                  Add by URL
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  data-testid="mcp-browse-directory"
                  onClick={() => {
                    setQuery(" ");
                    searchBox.current?.focus();
                  }}
                >
                  <Search className="size-4" />
                  Browse the directory
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      ) : (
        <>
          <McpServerTable>
            {term !== "" && (
              <McpGroupRow
                label="In this company"
                count={
                  matches.length === 1 ? "1 match" : `${matches.length} matches`
                }
              />
            )}
            {matches.map((server) => {
              const health = tested[server.name] ?? server.health;
              const credential = credentialAffordance(health?.authHint, {
                source: server.source,
                status: health?.status,
              });
              const dial = mcpRowControls(server, health).lifecycle;
              // At most ONE labelled action per row: the one this server's state
              // actually calls for.
              const primary: PrimaryAction =
                credential === "sign_in"
                  ? { kind: "sign_in" }
                  : credential === "add_token"
                    ? credentialFor === server.name
                      ? null
                      : { kind: "add_token" }
                    : credential === "rotate_env"
                      ? envFor === server.name
                        ? null
                        : { kind: "rotate_env" }
                      : dial === "connect"
                        ? { kind: "connect" }
                        : null;
              return (
                <McpServerRow
                  key={server.name}
                  server={server}
                  health={health}
                  bridge={bridge}
                  canManage={canManage}
                  busy={busy}
                  primary={primary}
                  signingIn={signIns[server.name] !== undefined}
                  toolsOpen={tools[server.name]?.kind === "ready"}
                  actions={actions}
                />
              );
            })}
            {term !== "" &&
              directory.kind === "ready" &&
              directory.entries.length > 0 && (
                <>
                  <McpGroupRow
                    label="Not installed — from the public directory"
                    count={
                      directory.totalPages > 1
                        ? `showing ${directory.entries.length} of the first ${directory.totalPages} pages`
                        : `${directory.entries.length} matches`
                    }
                  />
                  {directory.entries.map((entry) => (
                    <McpDirectoryRow
                      key={entry.qualifiedName}
                      entry={entry}
                      installedAs={installedAs(servers, entry)}
                      installing={installing === entry.qualifiedName}
                      canManage={canManage}
                      onInstall={(e) => void install(e)}
                    />
                  ))}
                </>
              )}
          </McpServerTable>

          {directory.kind === "loading" && (
            <p className="flex items-center gap-1 text-xs text-muted-foreground">
              <Loader2 className="size-3 animate-spin" /> Searching the public
              directory…
            </p>
          )}

          {/* Half a result beats an empty page: a directory outage degrades the
              answer instead of taking this company's own servers off screen. */}
          {directory.kind === "outage" &&
            (directory.outage.kind === "unwired" ? (
              <p
                className="text-xs text-muted-foreground"
                data-testid="mcp-registry-unwired"
              >
                {REGISTRY_UNWIRED_NOTICE}
              </p>
            ) : (
              <p
                className="text-xs text-status-blocked-text"
                data-testid="mcp-registry-error"
              >
                <strong className="font-medium">
                  The directory isn&apos;t answering
                </strong>
                , so only this company&apos;s own servers were searched.{" "}
                {directory.outage.message} The directories are federated and
                either can be down; nothing about your servers changes.
              </p>
            ))}

          {term !== "" &&
            matches.length === 0 &&
            directory.kind === "ready" &&
            directory.entries.length === 0 && (
              <Card data-testid="mcp-search-nothing">
                <CardContent className="space-y-2">
                  <p className="text-sm text-muted-foreground">
                    Nothing in this company matches{" "}
                    <strong className="font-medium text-foreground">
                      {query.trim()}
                    </strong>
                    , and the directory has no listing for it.
                  </p>
                  <p className="text-sm text-muted-foreground">
                    A server that is not published in any directory — something
                    running inside your own network — is connected by pasting its
                    endpoint.
                  </p>
                  {canManage && (
                    <Button size="sm" onClick={() => setAdding(true)}>
                      <Plus className="size-4" />
                      Add by URL
                    </Button>
                  )}
                </CardContent>
              </Card>
            )}

          {term === "" && servers.length > 0 && (
            <p className="text-xs text-muted-foreground" data-testid="mcp-tally">
              {servers.length} server{servers.length === 1 ? "" : "s"} ·{" "}
              {servers.filter((s) => s.enabled).length} on ·{" "}
              {servers.filter((s) => !s.enabled).length} off.{" "}
              {bridge === "absent"
                ? "None of them reaches an agent in this build."
                : "Agents pick up a change on their next turn."}
            </p>
          )}

          {/* The rows that need something said about them, said once below the
              table rather than as a second line inside a cell. */}
          {matches.map((server) => {
            const health = tested[server.name] ?? server.health;
            const flight = signIns[server.name];
            const toolState = tools[server.name] ?? { kind: "idle" };
            const credentialOpen = credentialFor === server.name && canManage;
            const envOpen = envFor === server.name && canManage;
            // The host's own sentence about why a server is not answering, kept
            // verbatim.
            const complaint =
              health && health.status !== "ok" && health.message.trim()
                ? health.message
                : null;
            if (
              !flight &&
              !credentialOpen &&
              !envOpen &&
              complaint === null &&
              toolState.kind === "idle" &&
              !registryOauthUnsupported(server, health) &&
              !(bridge !== "absent" && server.enabled && server.reachableBy?.length === 0)
            ) {
              return null;
            }
            return (
              <div key={server.name} className="space-y-1.5">
                <p className="text-xs font-medium">{server.name}</p>
                {bridge !== "absent" &&
                  server.enabled &&
                  server.reachableBy?.length === 0 && (
                    <p
                      data-testid="mcp-reachability-none"
                      className="flex items-start gap-1.5 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-xs font-medium text-destructive"
                    >
                      <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                      <span>
                        No agent can reach this server — no tool grant covers{" "}
                        <code className="font-mono">mcp:{server.name}</code>.
                        Widen a company or per-agent tool grant, or this server
                        is unused.
                      </span>
                    </p>
                  )}
                {complaint && (
                  <p className="text-xs text-muted-foreground">{complaint}</p>
                )}
                {registryOauthUnsupported(server, health) && (
                  <p
                    className="text-xs text-muted-foreground"
                    data-testid="mcp-no-credential-control"
                  >
                    {REGISTRY_OAUTH_UNSUPPORTED_NOTICE}
                  </p>
                )}
                {flight && (
                  <SignInFlightPanel
                    name={server.name}
                    flight={flight}
                    onCancel={() => cancelSignIn(server.name)}
                  />
                )}
                {credentialOpen && (
                  <div
                    className="flex items-end gap-2"
                    data-testid="mcp-token-inline"
                  >
                    <div className="flex-1 space-y-1">
                      <Label
                        htmlFor={`mcp-token-${server.name}`}
                        className="text-xs"
                      >
                        API token for {server.name}
                        {/* The value is write-only and unrecoverable, so say
                            when saving it overwrites an existing one
                            (issue #1464). */}
                        {server.authConfigured
                          ? " — replaces the stored credential"
                          : ""}
                      </Label>
                      <Input
                        id={`mcp-token-${server.name}`}
                        type="password"
                        autoComplete="new-password"
                        placeholder="write-only"
                        value={credentialDraft}
                        onChange={(e) => setCredentialDraft(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void saveCredential(server);
                          if (e.key === "Escape") setCredentialFor(null);
                        }}
                      />
                    </div>
                    <Button
                      size="sm"
                      data-testid="mcp-token-save"
                      disabled={busy !== null || !credentialDraft.trim()}
                      onClick={() => void saveCredential(server)}
                    >
                      {busy === server.name ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        "Save"
                      )}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy !== null}
                      onClick={() => setCredentialFor(null)}
                    >
                      Cancel
                    </Button>
                  </div>
                )}
                {envOpen && (
                  <div
                    className="space-y-2 rounded-md bg-muted/40 p-2"
                    data-testid="mcp-env-inline"
                  >
                    <p className="text-xs text-muted-foreground">
                      Saving merges these values with the stored credentials and
                      reconnects this server.
                    </p>
                    {envFields.kind === "loading" ? (
                      <p className="flex items-center gap-1 text-xs text-muted-foreground">
                        <Loader2 className="size-3 animate-spin" /> Reading this
                        server&apos;s credential fields…
                      </p>
                    ) : envFields.kind === "failed" ? (
                      <p
                        className="text-xs text-destructive"
                        data-testid="mcp-env-unavailable"
                      >
                        {envFields.message}
                      </p>
                    ) : envFields.keys.length === 0 ? (
                      <p className="text-xs text-muted-foreground">
                        This server asks for no credentials.
                      </p>
                    ) : (
                      envFields.keys.map((key) => (
                        <div key={key} className="space-y-1">
                          <Label
                            htmlFor={`mcp-env-${server.name}-${key}`}
                            className="font-mono text-xs"
                          >
                            {key}
                          </Label>
                          <Input
                            id={`mcp-env-${server.name}-${key}`}
                            type="password"
                            autoComplete="new-password"
                            placeholder="write-only"
                            value={envDraft[key] ?? ""}
                            onChange={(e) =>
                              setEnvDraft({
                                ...envDraft,
                                [key]: e.target.value,
                              })
                            }
                          />
                        </div>
                      ))
                    )}
                    {envError && (
                      <p className="text-xs text-destructive">{envError}</p>
                    )}
                    <div className="flex items-center gap-2">
                      {envFields.kind === "ready" &&
                        envFields.keys.length > 0 && (
                          <Button
                            size="sm"
                            data-testid="mcp-env-save"
                            disabled={busy !== null}
                            onClick={() =>
                              void saveEnvRotation(server, envFields.keys)
                            }
                          >
                            {busy === server.name ? (
                              <Loader2 className="size-4 animate-spin" />
                            ) : (
                              "Save"
                            )}
                          </Button>
                        )}
                      <Button
                        size="sm"
                        variant="ghost"
                        disabled={busy !== null}
                        onClick={() => setEnvFor(null)}
                      >
                        {envFields.kind === "ready" && envFields.keys.length > 0
                          ? "Cancel"
                          : "Close"}
                      </Button>
                    </div>
                  </div>
                )}
                <McpToolsList state={toolState} />
              </div>
            );
          })}
        </>
      )}

      {removalDialog}

      <McpAddServerDialog
        client={client}
        company={company}
        open={adding}
        bridge={bridge}
        onOpenChange={setAdding}
        onAdded={() => void refresh()}
        onOpenServer={(name) => setOpenedName(name)}
      />

    </section>
  );
}

/** The name this company already holds a directory entry under, if it does. */
function installedAs(
  servers: McpServer[],
  entry: McpCatalogueEntry,
): string | null {
  const byQualified = servers.find(
    (s) => s.qualifiedName === entry.qualifiedName,
  );
  if (byQualified) return byQualified.name;
  const slug = entry.displayName.trim().toLowerCase();
  const byName = servers.find((s) => s.name.trim().toLowerCase() === slug);
  return byName?.name ?? null;
}

/**
 * A sign-in the operator is still finishing somewhere else.
 */
function SignInFlightPanel({
  name,
  flight,
  onCancel,
}: {
  name: string;
  flight: SignInFlight;
  onCancel: () => void;
}) {
  const ago = Math.max(0, Math.round((Date.now() - flight.checkedAtMillis) / 1000));
  return (
    <div
      className="space-y-2 rounded-md border border-border bg-muted/30 p-2"
      data-testid="mcp-signin-flight"
    >
      {flight.timedOut ? (
        <p className="text-xs text-status-blocked-text">
          Sign-in for {name} timed out. Nothing was stored — start it again when
          you are ready.
        </p>
      ) : flight.blocked ? (
        <p className="text-xs text-status-blocked-text" data-testid="mcp-signin-blocked">
          <strong className="font-medium">
            The sign-in tab could not be opened.
          </strong>{" "}
          A blocked popup, or a desktop webview that cannot create one. Open this
          address by hand to finish:
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          <strong className="font-medium text-foreground">
            Finish in the {name} tab that just opened.
          </strong>{" "}
          This page is watching and will update itself — you do not need to come
          back and press anything.
        </p>
      )}
      <code className="block truncate rounded-md border border-border bg-background px-2 py-1 font-mono text-xs">
        {flight.authorizeUrl}
      </code>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="outline"
          data-testid="mcp-signin-reopen"
          onClick={() => {
            if (!openOutward(flight.authorizeUrl)) {
              window.open(flight.authorizeUrl, "_blank", "noopener,noreferrer");
            }
          }}
        >
          Reopen the {name} tab
        </Button>
        <Button
          size="sm"
          variant="ghost"
          data-testid="mcp-signin-cancel"
          onClick={onCancel}
        >
          {flight.timedOut ? "Dismiss" : "Cancel"}
        </Button>
        {!flight.timedOut && (
          <span className="text-3xs text-muted-foreground">
            checked {ago}s ago
          </span>
        )}
      </div>
    </div>
  );
}

/** Renders the live-discovered tool list for one server. */
function McpToolsList({ state }: { state: ToolsState }) {
  if (state.kind === "idle") return null;
  if (state.kind === "loading") {
    return (
      <p className="flex items-center gap-1 text-xs text-muted-foreground">
        <Loader2 className="size-3 animate-spin" /> Discovering tools…
      </p>
    );
  }
  if (state.kind === "unwired") {
    return (
      <p className="text-xs text-muted-foreground">
        Live tool discovery isn&apos;t enabled in this build (the agent harness
        is off).
      </p>
    );
  }
  if (state.kind === "error") {
    return <p className="text-xs text-destructive">{state.message}</p>;
  }
  if (state.tools.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        This server exposed no tools.
      </p>
    );
  }
  return (
    <ul className="space-y-1 rounded-md bg-muted/40 p-2">
      {state.tools.map((tool) => (
        <li key={tool.name} className="text-xs">
          <span className="font-mono font-medium">{tool.name}</span>
          {tool.description ? (
            <span className="text-muted-foreground"> — {tool.description}</span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}
