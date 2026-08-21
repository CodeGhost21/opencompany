import { useCallback, useEffect, useRef, useState } from "react";
import { KeyRound, Loader2, Save, ShieldCheck, Trash2 } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import {
  getMcpDirectoryCredential,
  setMcpDirectoryCredential,
  type McpDirectoryCredential,
} from "@/api/mcp-registry";
import { ApiError } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** Called after a successful write so the directory search re-runs. */
  onChanged?: () => void;
}

/**
 * The company's Smithery key — what decides whether the directory above has
 * hosted servers to show (issue #1287).
 *
 * Sits inside the directory browser rather than beside the other credential
 * cards, because it is not a general identity: it buys one specific thing an
 * operator is looking straight at when they need it. The empty-search notice
 * points down here in as many words.
 *
 * Write-only, like every credential the console handles: the key goes out on
 * `PUT`, lands in the tenant's secret store and is never returned, so the field
 * is always empty on load and "set" is reported by a flag rather than by a
 * masked value we would have had to receive.
 *
 * Unlike the routes it sits under, this one is **not** behind the host's `mcp`
 * feature, so it does not 404 on an unwired build and has no `unavailable`
 * state to render.
 */
export function McpDirectoryCredentialCard({ client, company, onChanged }: Props) {
  const [load, setLoad] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<McpDirectoryCredential | null>(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"save" | "clear" | null>(null);

  // Discards a response whose company is no longer the one on screen. Same
  // guard, and the same reason, as `CompanyCredentialCard`: an admin who reads
  // "set" for a company that in fact has no key will not set one.
  const requestGeneration = useRef(0);

  const refresh = useCallback(async () => {
    const generation = ++requestGeneration.current;
    try {
      const next = await getMcpDirectoryCredential(client, company);
      if (generation !== requestGeneration.current) return;
      setStatus(next);
      setError(null);
      setLoad("ready");
    } catch (err) {
      if (generation !== requestGeneration.current) return;
      // Never swallowed into a "not set" state. The host answers 5xx when it
      // cannot read the secret store, and painting "Not set" on that would send
      // an operator to paste a key they already have.
      setError(
        err instanceof ApiError ? err.message : "Couldn't read the directory credential.",
      );
      setLoad("error");
    }
  }, [client, company]);

  useEffect(() => {
    setLoad("loading");
    setKey("");
    void refresh();
  }, [refresh]);

  async function write(value: string, mode: "save" | "clear") {
    setBusy(mode);
    const generation = ++requestGeneration.current;
    try {
      const result = await setMcpDirectoryCredential(client, company, value);
      // Unconditional: the write landed, and an admin who navigated away is
      // still owed that fact. What is conditional is the *state*, which
      // describes the company that was on screen when the write began.
      toast.success(mode === "save" ? "Smithery key saved." : "Smithery key cleared.", {
        description: result.note,
      });
      if (generation !== requestGeneration.current) return;
      setStatus(result.status);
      setKey("");
      onChanged?.();
    } catch (err) {
      // The host's own reason when it sent one — an admin-only refusal says
      // something specific, and a generic "couldn't save" throws away the only
      // actionable part.
      toast.error(
        err instanceof ApiError
          ? err.message
          : mode === "save"
            ? "Couldn't save the Smithery key."
            : "Couldn't clear the Smithery key.",
      );
    } finally {
      setBusy(null);
    }
  }

  if (load === "loading") return <Skeleton className="h-24 rounded-xl" />;

  if (load === "error") {
    return (
      <p className="text-xs text-muted-foreground" data-testid="mcp-directory-credential-error">
        {error ?? "Couldn't read the directory credential."} This is not the same as having none
        set — the host could not answer, so the current key is unknown.
      </p>
    );
  }

  const configured = status?.configured === true;
  // No company key, but the host carries one: the directory works, and it works
  // through an account shared with every other company on this instance. Saying
  // only "not set" would read as broken; saying only "set" would hide the
  // sharing.
  const sharedHostKey = !configured && status?.source === "environment";

  return (
    <section className="space-y-3 rounded-xl border border-border p-3" data-testid="mcp-directory-credential">
      <div className="flex items-center gap-2">
        <KeyRound className="size-4 text-muted-foreground" />
        <h4 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Smithery key
        </h4>
        {configured ? (
          <span
            className="inline-flex items-center gap-1 text-xs text-status-done-text"
            data-testid="mcp-directory-credential-state"
          >
            <ShieldCheck className="size-3" /> Set for this company
          </span>
        ) : sharedHostKey ? (
          <span className="text-xs text-muted-foreground" data-testid="mcp-directory-credential-state">
            Using this host&apos;s shared key
          </span>
        ) : (
          <span className="text-xs text-muted-foreground" data-testid="mcp-directory-credential-state">
            Not set
          </span>
        )}
      </div>

      {/* The host words the consequence, so the console cannot drift from what
          the host actually does. */}
      {status && (
        <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
          {status.notice}
        </p>
      )}

      <div className="space-y-1">
        <Label htmlFor="mcp-directory-key" className="text-xs">
          Smithery API key {configured ? "— set (paste a new value to rotate)" : ""}
        </Label>
        <Input
          id="mcp-directory-key"
          type="password"
          autoComplete="off"
          placeholder="paste this company's Smithery API key"
          value={key}
          onChange={(e) => setKey(e.target.value)}
        />
      </div>

      <div className="flex items-center gap-2">
        <Button
          size="sm"
          disabled={busy !== null || !key.trim()}
          onClick={() => void write(key.trim(), "save")}
        >
          {busy === "save" ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
          {configured ? "Rotate key" : "Save key"}
        </Button>
        {configured && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy !== null}
            onClick={() => void write("", "clear")}
          >
            {busy === "clear" ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Trash2 className="size-4" />
            )}
            Clear
          </Button>
        )}
      </div>
    </section>
  );
}
