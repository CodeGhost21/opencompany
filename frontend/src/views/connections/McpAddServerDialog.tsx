import { useState } from "react";
import { AlertTriangle, Loader2, Plus } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import { addMcpServer, type McpAuthKind, updateMcpServer } from "@/api/mcp";
import { ApiError, type McpMutationResponse } from "@/api/types";
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
import { Textarea } from "@/components/ui/textarea";
import type { McpBridgeState } from "@/lib/mcp-bridge";

/**
 * Connecting a server by URL.
 *
 * On a build with no MCP bridge this flow can report a green probe no agent will
 * ever act on, so it says so here. The outcome lands on the server it is about.
 */

type Phase =
  | { kind: "form" }
  | { kind: "saving" }
  /** Saved. The probe's answer, and the description the server reports. */
  | { kind: "added"; result: McpMutationResponse };

export function McpAddServerDialog({
  client,
  company,
  open,
  bridge,
  onOpenChange,
  onAdded,
  onOpenServer,
}: {
  client: OpenCompanyClient;
  company: string | null;
  open: boolean;
  bridge: McpBridgeState;
  onOpenChange: (open: boolean) => void;
  /** Called once the list should re-read. */
  onAdded: () => void;
  onOpenServer: (name: string) => void;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "form" });
  const [name, setName] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [description, setDescription] = useState("");
  const [token, setToken] = useState("");
  const [authKind, setAuthKind] = useState<McpAuthKind>("bearer");
  const [authFieldName, setAuthFieldName] = useState("");
  /** A refusal that belongs on the Name field, not in a banner over all four. */
  const [nameError, setNameError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [describing, setDescribing] = useState(false);

  function reset() {
    setPhase({ kind: "form" });
    setName("");
    setEndpoint("");
    setDescription("");
    setToken("");
    setAuthKind("bearer");
    setAuthFieldName("");
    setNameError(null);
    setFormError(null);
  }

  function close() {
    if (phase.kind === "saving") return;
    onOpenChange(false);
    reset();
  }

  async function submit() {
    if (phase.kind === "saving") return;
    setNameError(null);
    setFormError(null);
    if (!name.trim() || !endpoint.trim()) {
      setFormError("A server needs a name and an https endpoint.");
      return;
    }
    if (authKind !== "bearer" && token.trim() && !authFieldName.trim()) {
      setFormError(
        authKind === "header"
          ? "A custom-header credential needs a header name."
          : "A query-parameter credential needs a parameter name.",
      );
      return;
    }
    setPhase({ kind: "saving" });
    try {
      const result = await addMcpServer(client, company, {
        name: name.trim(),
        endpoint: endpoint.trim(),
        description: description.trim() || undefined,
        token: token.trim() || undefined,
        authKind,
        headerName:
          authKind === "header" ? authFieldName.trim() || undefined : undefined,
        paramName:
          authKind === "query_param"
            ? authFieldName.trim() || undefined
            : undefined,
      });
      setPhase({ kind: "added", result });
      onAdded();
    } catch (err) {
      const sentence =
        err instanceof ApiError ? err.message : "Couldn't add the server.";
      // The host rejects a duplicate by name, so the message sits on the Name
      // field.
      if (/already exists|already configured/i.test(sentence)) {
        setNameError(sentence);
      } else {
        setFormError(sentence);
      }
      setPhase({ kind: "form" });
    }
  }

  /** Adopt what the server calls itself, on a row that was saved without one. */
  async function useProbedDescription(server: string, probed: string) {
    setDescribing(true);
    try {
      await updateMcpServer(client, company, server, { description: probed });
      onAdded();
      close();
    } catch (err) {
      setFormError(
        err instanceof ApiError
          ? err.message
          : "Couldn't save that description.",
      );
    } finally {
      setDescribing(false);
    }
  }

  const saving = phase.kind === "saving";

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : close())}>
      <DialogContent data-testid="mcp-add-dialog">
        {phase.kind === "added" ? (
          <Outcome
            result={phase.result}
            describing={describing}
            error={formError}
            onUseProbed={useProbedDescription}
            onOpenServer={(server) => {
              close();
              onOpenServer(server);
            }}
            onDone={close}
          />
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Add MCP server</DialogTitle>
              <DialogDescription>
                <strong className="font-medium text-foreground">
                  Only connect servers you trust.
                </strong>{" "}
                OpenCompany cannot verify what tools a server exposes, or whether
                they change after you connect it. Every agent you grant this
                server can call whatever it offers.
              </DialogDescription>
            </DialogHeader>

            <form
              className="space-y-3"
              onSubmit={(event) => {
                event.preventDefault();
                void submit();
              }}
            >
              <div className="space-y-1">
                <Label htmlFor="mcp-name" className="text-xs">
                  Name
                </Label>
                <Input
                  id="mcp-name"
                  data-testid="mcp-add-name"
                  value={name}
                  placeholder="notion"
                  aria-invalid={nameError !== null}
                  onChange={(e) => {
                    setName(e.target.value);
                    setNameError(null);
                  }}
                />
                {nameError && (
                  <p
                    className="text-xs text-destructive"
                    data-testid="mcp-add-name-error"
                  >
                    {nameError} Open it instead, or pick another name.
                  </p>
                )}
              </div>
              <div className="space-y-1">
                <Label htmlFor="mcp-endpoint" className="text-xs">
                  MCP server URL
                </Label>
                <Input
                  id="mcp-endpoint"
                  name="mcp-endpoint-url"
                  data-testid="mcp-add-endpoint"
                  value={endpoint}
                  placeholder="https://mcp.example.com/mcp"
                  autoComplete="url"
                  className="font-mono"
                  onChange={(e) => setEndpoint(e.target.value)}
                />
              </div>
              <div className="space-y-1">
                <Label htmlFor="mcp-description" className="text-xs">
                  What it does
                </Label>
                <Textarea
                  id="mcp-description"
                  data-testid="mcp-add-description"
                  rows={2}
                  value={description}
                  placeholder="Search, read and update pages across your Notion workspace."
                  onChange={(e) => setDescription(e.target.value)}
                />
                {/* Every server added by URL has been permanently nameless: the
                    route has always accepted a description and the form never
                    asked for one. */}
                <p className="text-3xs text-muted-foreground">
                  One line, so the list says what this server is for rather than
                  repeating its address. Left blank, the first check offers what
                  the server calls itself.
                </p>
              </div>
              <div className="grid gap-2 sm:grid-cols-[auto_1fr] sm:items-end">
                <div className="space-y-1">
                  <Label htmlFor="mcp-auth-kind" className="text-xs">
                    Auth
                  </Label>
                  <select
                    id="mcp-auth-kind"
                    value={authKind}
                    onChange={(e) => setAuthKind(e.target.value as McpAuthKind)}
                    className="h-9 w-full rounded-md border border-input bg-transparent px-3 text-sm shadow-xs"
                  >
                    <option value="bearer">Bearer token</option>
                    <option value="header">Custom header</option>
                    <option value="query_param">Query parameter</option>
                  </select>
                </div>
                {authKind !== "bearer" ? (
                  <div className="space-y-1">
                    <Label htmlFor="mcp-auth-field" className="text-xs">
                      {authKind === "header" ? "Header name" : "Parameter name"}
                    </Label>
                    <Input
                      id="mcp-auth-field"
                      value={authFieldName}
                      placeholder={
                        authKind === "header" ? "X-Api-Key" : "apiKey"
                      }
                      autoComplete="off"
                      onChange={(e) => setAuthFieldName(e.target.value)}
                    />
                  </div>
                ) : (
                  <div className="space-y-1">
                    <Label htmlFor="mcp-token" className="text-xs">
                      Token (optional)
                    </Label>
                    <Input
                      id="mcp-token"
                      name="mcp-token-secret"
                      type="password"
                      value={token}
                      placeholder="write-only"
                      autoComplete="new-password"
                      onChange={(e) => setToken(e.target.value)}
                    />
                  </div>
                )}
              </div>
              {authKind !== "bearer" && (
                <div className="space-y-1">
                  <Label htmlFor="mcp-token" className="text-xs">
                    Credential value
                  </Label>
                  <Input
                    id="mcp-token"
                    name="mcp-token-secret"
                    type="password"
                    value={token}
                    placeholder="write-only"
                    autoComplete="new-password"
                    onChange={(e) => setToken(e.target.value)}
                  />
                </div>
              )}

              {bridge === "absent" && (
                <p
                  className="flex items-start gap-2 rounded-md border border-status-blocked-text/30 bg-status-blocked-text/10 px-2 py-1 text-xs text-status-blocked-text"
                  data-testid="mcp-add-no-bridge"
                >
                  <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                  <span>
                    This deployment has no MCP bridge, so the server will be
                    stored but no agent will receive its tools. The configuration
                    survives a rebuild with the{" "}
                    <code className="font-mono">mcp</code> feature.
                  </span>
                </p>
              )}

              {formError && (
                <p
                  className="text-xs text-destructive"
                  data-testid="mcp-add-error"
                >
                  {formError}
                </p>
              )}

              <DialogFooter>
                <Button
                  type="button"
                  variant="ghost"
                  disabled={saving}
                  onClick={close}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  data-testid="mcp-add-submit"
                  disabled={saving || nameError !== null}
                >
                  {saving ? (
                    <Loader2 className="size-4 animate-spin" />
                  ) : (
                    <Plus className="size-4" />
                  )}
                  {bridge === "absent" ? "Save anyway" : "Add server"}
                </Button>
              </DialogFooter>
            </form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

/**
 * What happened, on the server it happened to.
 *
 * Added-and-broken is its own outcome and not an error on the form: the server
 * exists, is enabled, and is attached to every agent that reaches it.
 */
function Outcome({
  result,
  describing,
  error,
  onUseProbed,
  onOpenServer,
  onDone,
}: {
  result: McpMutationResponse;
  describing: boolean;
  error: string | null;
  onUseProbed: (server: string, probed: string) => void;
  onOpenServer: (server: string) => void;
  onDone: () => void;
}) {
  const server = result.server;
  const test = result.test;
  const connected = test?.status === "ok";
  const probed = server.probedDescription?.trim();
  const offerProbed =
    probed !== undefined && probed !== "" && !server.description?.trim();

  return (
    <>
      <DialogHeader>
        <DialogTitle>
          {connected ? `Added ${server.name}` : "Added, but not answering"}
        </DialogTitle>
        <DialogDescription>{result.note}</DialogDescription>
      </DialogHeader>
      <div className="space-y-3">
        <code className="block truncate rounded-md border border-border bg-muted/40 px-2 py-1 font-mono text-xs">
          {server.endpoint}
        </code>
        {test && (
          <p
            className={`text-xs ${connected ? "text-muted-foreground" : "text-status-blocked-text"}`}
            data-testid="mcp-add-outcome"
          >
            {connected
              ? `Connected. ${test.toolCount} tool${test.toolCount === 1 ? "" : "s"} found. Every one of them starts on its tier's default until you decide otherwise.`
              : `${test.message} It is saved and listed with your servers. No tools were discovered, so no permissions are set yet — re-check it once the endpoint is up.`}
          </p>
        )}
        {result.warning && (
          <p className="text-xs text-status-blocked-text">{result.warning}</p>
        )}
        {offerProbed && (
          <div
            className="space-y-2 rounded-md border border-border bg-muted/30 p-2"
            data-testid="mcp-add-probed-description"
          >
            <p className="text-xs text-muted-foreground">
              This server describes itself as:{" "}
              <span className="text-foreground">{probed}</span>
            </p>
            <Button
              size="sm"
              variant="outline"
              disabled={describing}
              data-testid="mcp-add-use-probed"
              onClick={() => onUseProbed(server.name, probed)}
            >
              {describing ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                "Use that description"
              )}
            </Button>
          </div>
        )}
        {error && (
          <p className="text-xs text-destructive" data-testid="mcp-add-error">
            {error}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button variant="ghost" onClick={onDone}>
          Done
        </Button>
        <Button
          data-testid="mcp-add-open-server"
          onClick={() => onOpenServer(server.name)}
        >
          Open the server
        </Button>
      </DialogFooter>
    </>
  );
}
