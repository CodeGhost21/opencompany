import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, Info, Loader2, RotateCcw, Save } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { getMcpConfig, putMcpConfig, type McpConfigDoc } from "@/api/mcp-config";
import { ApiError } from "@/api/types";
import { formatMcpConfig, mcpConfigChanged, parseMcpConfig } from "@/lib/mcp-json";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** Whether this viewer may change the company's tool servers (issue #403). */
  canManage: boolean;
  /** Called after a successful save, so the rows re-read what was written. */
  onSaved?: () => void;
}

type Load = "loading" | "ready" | "unavailable" | "error";

/**
 * `mcp.json` — the company's declared MCP servers as one editable document.
 *
 * The second half of the MCP page, beside the connection rows. Both write the
 * *same* store through the same host, so this is not an import/export format
 * that drifts from the real configuration: it is the configuration, in the
 * spelling an operator already knows from `claude_desktop_config.json`. Pasting
 * a block of servers is one action here and N form submissions on the rows,
 * which is the whole reason a text surface earns its place.
 *
 * Three things this deliberately does *not* do:
 *
 * - **Guess at the host's rules.** Only JSON-ness and the document's shape are
 *   checked locally (see `lib/mcp-json.ts`); a refusal from the host is shown
 *   verbatim, because the host is the authority on what it will dial and a
 *   console paraphrase is one more thing to keep in step.
 * - **Show a credential.** The read carries `authConfigured`, never a token, so
 *   an entry with no `headers` is not a server with no credential — it is a
 *   credential this surface cannot show. Saving such an entry leaves the stored
 *   value alone rather than clearing it, which is stated on screen because the
 *   opposite guess is the destructive one.
 * - **Reformat as you type.** The text is the operator's until they press
 *   Revert. An editor that rewrites the buffer mid-edit loses cursor position
 *   and, worse, decides what a half-typed entry meant.
 */
export function McpJsonEditor({ client, company, canManage, onSaved }: Props) {
  const [load, setLoad] = useState<Load>("loading");
  const [loaded, setLoaded] = useState<McpConfigDoc | null>(null);
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  // The host's own refusal, kept until the next save rather than toasted: it
  // names an entry in a document still on screen, and a toast that names
  // `notion` is gone by the time the operator finds `notion`.
  const [refusal, setRefusal] = useState<string | null>(null);
  // Which company's answer is still wanted. A scope change mid-flight must not
  // let one company's servers land in another company's editor.
  const scope = useRef(0);

  const refresh = useCallback(async () => {
    const mine = scope.current;
    setLoad("loading");
    try {
      const doc = await getMcpConfig(client, company);
      if (scope.current !== mine) return;
      setLoaded(doc);
      setText(formatMcpConfig(doc));
      setLoad("ready");
    } catch (err) {
      if (scope.current !== mine) return;
      // A 404 is a host with no MCP config route — a fact about the build, not
      // a failure. Anything else means we do not know what this company has,
      // and an empty editor would invite a save that wipes it.
      setLoad(err instanceof ApiError && err.status === 404 ? "unavailable" : "error");
    }
  }, [client, company]);

  useEffect(() => {
    scope.current += 1;
    void refresh();
  }, [refresh]);

  const parsed = useMemo(() => parseMcpConfig(text), [text]);
  const changed = useMemo(() => mcpConfigChanged(text, loaded), [text, loaded]);

  async function save() {
    if (saving || !parsed.ok) return;
    setSaving(true);
    setRefusal(null);
    try {
      const res = await putMcpConfig(client, company, parsed.doc);
      toast.success(`Saved mcp.json. ${res.note}`);
      onSaved?.();
      await refresh();
    } catch (err) {
      setRefusal(err instanceof ApiError ? err.message : "The host didn't accept that file.");
    } finally {
      setSaving(false);
    }
  }

  if (load === "unavailable") {
    return (
      <Alert data-testid="mcp-json-unavailable">
        <Info className="size-4" />
        <AlertTitle>This host doesn&apos;t serve mcp.json</AlertTitle>
        <AlertDescription>
          The document route isn&apos;t on this build. The connections above are still the whole
          configuration — edit them there.
        </AlertDescription>
      </Alert>
    );
  }

  if (load === "loading") return <Skeleton className="h-72 rounded-xl" />;

  if (load === "error") {
    return (
      <Alert variant="destructive" data-testid="mcp-json-error">
        <AlertTriangle className="size-4" />
        <AlertTitle>Couldn&apos;t read this company&apos;s mcp.json</AlertTitle>
        <AlertDescription>
          The host didn&apos;t answer, so what is configured is unknown — and an empty editor here
          would invite a save that replaces it. Reload to try again.
        </AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="space-y-3" data-testid="mcp-json-editor">
      <p className="text-sm text-muted-foreground">
        Every declared server in one file, in the shape you already use elsewhere. Saving replaces
        the set: a server you delete here is removed, and one declared in{" "}
        <code className="font-mono text-xs">company.toml</code> is disabled with{" "}
        <code className="font-mono text-xs">&quot;enabled&quot;: false</code> rather than deleted.
      </p>

      <Textarea
        data-testid="mcp-json-text"
        aria-label="mcp.json"
        spellCheck={false}
        readOnly={!canManage}
        value={text}
        onChange={(e) => setText(e.target.value)}
        className="min-h-72 font-mono text-xs leading-relaxed"
      />

      {/* Said beside the buffer rather than in the docs: `headers` is absent
          from every entry the host sends, and the reading an operator would
          otherwise take from that — "this server has no credential" — is one
          keystroke away from pasting a token that was never missing. */}
      <p className="text-xs text-muted-foreground">
        Credentials aren&apos;t shown. An entry reports{" "}
        <code className="font-mono">&quot;authConfigured&quot;</code> only; add{" "}
        <code className="font-mono">
          &quot;headers&quot;: {"{"} &quot;Authorization&quot;: &quot;Bearer …&quot; {"}"}
        </code>{" "}
        to set or rotate one. Leaving it out keeps the stored credential.
      </p>

      {!parsed.ok && changed && (
        <Alert variant="destructive" data-testid="mcp-json-invalid">
          <AlertTriangle className="size-4" />
          <AlertTitle>This file can&apos;t be saved yet</AlertTitle>
          <AlertDescription>{parsed.message}</AlertDescription>
        </Alert>
      )}

      {refusal && (
        <Alert variant="destructive" data-testid="mcp-json-refused">
          <AlertTriangle className="size-4" />
          <AlertTitle>The host refused this file</AlertTitle>
          <AlertDescription>{refusal}</AlertDescription>
        </Alert>
      )}

      {canManage && (
        <div className="flex items-center gap-2">
          <Button
            data-testid="mcp-json-save"
            disabled={saving || !changed || !parsed.ok}
            onClick={() => void save()}
          >
            {saving ? <Loader2 className="size-4 animate-spin" /> : <Save className="size-4" />}
            Save
          </Button>
          <Button
            variant="ghost"
            data-testid="mcp-json-revert"
            disabled={saving || !changed}
            onClick={() => setText(loaded ? formatMcpConfig(loaded) : "")}
          >
            <RotateCcw className="size-4" /> Revert
          </Button>
          {changed && parsed.ok && (
            <span className="text-xs text-muted-foreground">Unsaved changes</span>
          )}
        </div>
      )}
    </div>
  );
}
