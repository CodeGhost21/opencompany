import { useState } from "react";
import { AlertTriangle, ArrowLeft, Unplug } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { updateMcpServer } from "@/api/mcp";
import { ApiError, type McpHealth, type McpServer, type RosterAgent } from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { Textarea } from "@/components/ui/textarea";
import {
  connectedOn,
  mcpProviderSlug,
  mcpStanding,
  probedOn,
} from "@/lib/connection-detail";
import type { McpBridgeState } from "@/lib/mcp-bridge";
import {
  UsageSection,
  useConnectionUsage,
} from "@/views/connections/connection-usage";
import { mcpProvenanceNote, mcpRemovalNote } from "@/lib/mcp-registry";
import { McpServerIcon } from "@/views/connections/McpServerTable";
import { McpToolPermissions } from "@/views/mcp/McpToolPermissions";

const PROVENANCE_LABELS: Record<string, string> = {
  manifest: "manifest",
  registry: "directory",
  runtime: "console",
  default: "built in",
};

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  server: McpServer;
  health: McpHealth | undefined;
  canManage: boolean;
  /** Whether the agent-side MCP bridge is compiled into this host (issue #567). */
  bridge: McpBridgeState;
  /**
   * Whether a `needs_approval` mode parks a call on this host. `undefined` means
   * no read answered it, and nothing is claimed either way.
   */
  approvalsPark?: boolean;
  /** The roster, for the per-teammate lens on this server's tool permissions. */
  agents?: RosterAgent[];
  /** Bumped when a probe re-ran, so the permissions read is not stale. */
  reloadKey: number;
  /** Scroll the permissions panel into view once it has something to show. */
  focusPermissions: boolean;
  /** Absent when this row cannot be removed from the console. */
  onDisconnect: (() => void) | null;
  onBack: () => void;
}

/**
 * One MCP server, as a page: what it is, who can reach it, and what each of
 * its tools is allowed to do.
 */
export function McpServerPage({
  client,
  company,
  server,
  health,
  canManage,
  bridge,
  approvalsPark,
  agents = [],
  reloadKey,
  focusPermissions,
  onDisconnect,
  onBack,
}: Props) {
  const standing = mcpStanding(server, health);
  const probedAt = probedOn(health?.checkedAtMillis);
  const usage = useConnectionUsage(
    client,
    company,
    mcpProviderSlug(server.name),
  );

  return (
    <div className="space-y-4" data-testid="mcp-server-page">
      <div className="flex flex-wrap items-center gap-3">
        <Button
          size="sm"
          variant="ghost"
          onClick={onBack}
          className="-ml-2 gap-1 text-muted-foreground"
          data-testid="mcp-page-back"
        >
          <ArrowLeft className="size-4" />
          MCP Servers
        </Button>
        <McpServerIcon
          iconUrl={server.iconUrl}
          name={server.name}
          className="size-6"
        />
        <h2 className="text-base font-semibold">{server.name}</h2>
        <Badge
          variant="outline"
          className="font-normal"
          data-testid="mcp-page-provenance"
        >
          {PROVENANCE_LABELS[server.source] ?? server.source}
        </Badge>
        <span
          className="text-xs text-muted-foreground"
          data-testid="mcp-page-standing"
        >
          {standing.summary}
        </span>
        {onDisconnect && canManage && (
          <Button
            size="sm"
            variant="outline"
            onClick={onDisconnect}
            className="ml-auto gap-1"
            data-testid="mcp-page-disconnect"
          >
            <Unplug className="size-3.5" />
            Disconnect
          </Button>
        )}
      </div>

      <Separator />

      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="space-y-1">
          <p className="font-mono text-xs break-all text-muted-foreground">
            {server.endpoint}
          </p>
          <McpDescription
            client={client}
            company={company}
            server={server}
            canManage={canManage}
          />
          {server.websiteUrl && (
            <p className="text-xs text-muted-foreground">
              {/* The address the server reported for itself. A link; nothing
                  here fetches it. */}
              <a
                href={server.websiteUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="underline"
                data-testid="mcp-page-website"
              >
                {server.websiteUrl}
              </a>
            </p>
          )}
          <p
            className="text-xs text-muted-foreground"
            data-testid="mcp-page-probe"
          >
            {standing.probe}
            {probedAt !== null && ` · ${probedAt}`}
          </p>
          <p
            className="text-xs text-muted-foreground"
            data-testid="mcp-page-connected-on"
          >
            {connectedOn(undefined)} —{" "}
            {server.source === "registry"
              ? "this host does connect a directory install, but records no date for it."
              : "MCP has no connect step to record one."}
          </p>
          {health && health.status !== "ok" && health.message && (
            <p className="text-xs text-muted-foreground">{health.message}</p>
          )}
        </div>
      </div>

      {bridge !== "absent" &&
        standing.live &&
        server.reachableBy !== undefined && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          {server.reachableBy.length === 0 ? (
            <p
              className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-xs font-medium text-destructive"
              data-testid="mcp-page-reachability"
            >
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              <span>
                No agent can reach this server — no teammate&apos;s tool grants
                cover <code className="font-mono">mcp:{server.name}</code>, so
                every permission below is inert. It answers, and nobody can call
                it.{" "}
                {/* No button, because none could work: `mcp` is not in the set
                    the console may widen, so offering one that 403s would be
                    worse than naming the dead end. */}
                <span className="font-normal">
                  <code className="font-mono">mcp:</code> grants are not in the
                  set the console may widen, so this one is added in{" "}
                  <code className="font-mono">company.toml</code> — and on a
                  hosted tenant that file is a read-only boot snapshot.
                </span>
              </span>
            </p>
          ) : (
            <p
              className="text-xs text-muted-foreground"
              data-testid="mcp-page-reachability"
            >
              Reachable by:{" "}
              <span className="font-medium text-foreground">
                {server.reachableBy.map((agent) => agent.name).join(", ")}
              </span>
            </p>
          )}
          <a
            href="#/company"
            className="text-xs font-medium text-muted-foreground underline"
            data-testid="mcp-page-edit-agents"
          >
            Edit agents
          </a>
        </div>
      )}

      {!standing.live && (
        <p className="flex items-start gap-2 rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
          <AlertTriangle className="mt-px size-3 shrink-0" />
          <span>
            This server is turned off, so no agent receives its tools whatever
            their grants say and whatever the endpoint answers. Its
            configuration and any stored credential survive — turning it back on
            restores its tools on the next turn.
          </span>
        </p>
      )}

      <Separator />

      <McpToolPermissions
        client={client}
        company={company}
        server={server}
        canManage={canManage}
        agents={agents}
        approvalsPark={approvalsPark}
        reloadKey={reloadKey}
        focus={focusPermissions}
      />

      <Separator />

      <UsageSection
        usage={usage}
        perConnection={`Successful tool calls your agents made through ${server.name}, counted under mcp:${server.name.trim().toLowerCase()} so a Composio provider of the same name cannot be read as this one.`}
      />

      <Separator />

      <section className="space-y-1" aria-label="Removing this server">
        <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          What a disconnect reaches
        </h4>
        <p
          className="text-xs text-muted-foreground"
          data-testid="mcp-page-disconnect-scope"
        >
          {mcpRemovalNote(server.source)} Nothing is revoked at the
          server&apos;s own end: no token it issued is invalidated and no
          session there is closed. Revoke those where they were issued.
        </p>
        <p className="text-xs text-muted-foreground">
          {mcpProvenanceNote(server.source)}
        </p>
      </section>
    </div>
  );
}

/**
 * What this server is for, and the field that sets it.
 *
 * What the server calls itself is offered as the starting point, never written
 * over a declaration.
 */
function McpDescription({
  client,
  company,
  server,
  canManage,
}: {
  client: OpenCompanyClient;
  company: string | null;
  server: McpServer;
  canManage: boolean;
}) {
  const declared = server.description?.trim() ?? "";
  const probed = server.probedDescription?.trim() ?? "";
  // Only a console-added server: a manifest declaration is re-read from
  // `company.toml` on every boot, so a description saved over one would not
  // survive a restart.
  const editable = canManage && server.source === "runtime";
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(declared || probed);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await updateMcpServer(client, company, server.name, {
        description: draft.trim(),
      });
      setEditing(false);
    } catch (err) {
      setError(
        err instanceof ApiError
          ? err.message
          : "Couldn't save that description.",
      );
    } finally {
      setSaving(false);
    }
  }

  if (editing) {
    return (
      <div className="max-w-prose space-y-1" data-testid="mcp-page-description-edit">
        <Textarea
          rows={2}
          value={draft}
          aria-label={`What ${server.name} does`}
          onChange={(e) => setDraft(e.target.value)}
        />
        {probed && draft.trim() !== probed && (
          <button
            type="button"
            className="text-3xs text-muted-foreground underline"
            data-testid="mcp-page-use-probed"
            onClick={() => setDraft(probed)}
          >
            Use what this server calls itself: {probed}
          </button>
        )}
        {error && <p className="text-xs text-destructive">{error}</p>}
        <div className="flex items-center gap-2">
          <Button size="sm" disabled={saving} onClick={() => void save()}>
            {saving ? "Saving…" : "Save"}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            disabled={saving}
            onClick={() => setEditing(false)}
          >
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  const shown = declared || probed;
  return (
    <p className="max-w-prose text-xs text-muted-foreground" data-testid="mcp-page-description">
      {shown || "No description — this server says nothing about itself, and none was declared."}
      {editable && (
        <button
          type="button"
          className="ml-2 underline"
          data-testid="mcp-page-describe"
          onClick={() => {
            setDraft(declared || probed);
            setEditing(true);
          }}
        >
          {shown ? "Edit" : "Describe it"}
        </button>
      )}
    </p>
  );
}
