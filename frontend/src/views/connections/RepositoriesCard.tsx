import { useCallback, useEffect, useState } from "react";
import { Check, GitBranch, Info, Loader2, Trash2 } from "lucide-react";
import { toast } from "sonner";

import type { OpenCompanyClient } from "@/api/client";
import { bindRepo, listRepos, revokeRepo, type Repo } from "@/api/repos";
import { ApiError } from "@/api/types";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /**
   * Whether this viewer may change what the company reads (issue #403).
   * Courtesy only — the host answers 403 regardless.
   */
  canManage: boolean;
}

type Load = "loading" | "ready" | "unavailable";

/**
 * Bind real repositories to the company.
 *
 * The host keeps a bare mirror of each one and fetches it with the operator's
 * credential; nothing an agent runs ever sees the token, and no read on this
 * page returns it — a stored credential shows only as a fingerprint.
 *
 * The card is deliberately honest about two things the operator cannot see
 * from here:
 *
 * - **Nothing reads these yet.** Binding a repository in this release stages
 *   the code on the host. The tools that let an agent open it are a follow-up,
 *   and saying so beats an operator wiring a token and waiting for a capability
 *   that has not shipped.
 * - **A classic token will be refused**, so the field says which kind to make
 *   *before* they paste one, rather than after.
 */
export function RepositoriesCard({ client, company, canManage }: Props) {
  const [load, setLoad] = useState<Load>("loading");
  const [repos, setRepos] = useState<Repo[]>([]);
  const [diffsAvailable, setDiffsAvailable] = useState(false);
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [branches, setBranches] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await listRepos(client, company);
      setRepos(list.repos);
      setDiffsAvailable(list.pullRequestsAvailable);
      setLoad("ready");
    } catch {
      // No repository surface on this host — hide the card entirely rather
      // than showing an error for a capability that was never offered.
      setLoad("unavailable");
    }
  }, [client, company]);

  useEffect(() => {
    setLoad("loading");
    void refresh();
  }, [refresh]);

  async function bind() {
    if (busy) return;
    if (!url.trim() || !token.trim()) {
      toast.error("A repository URL and a token are both needed.");
      return;
    }
    setBusy("bind");
    try {
      const parsed = branches
        .split(",")
        .map((b) => b.trim())
        .filter(Boolean);
      await bindRepo(client, company, {
        url: url.trim(),
        token: token.trim(),
        branches: parsed,
      });
      // Cleared on success only. A refused bind keeps the URL and branches so
      // the operator can fix the one field the host complained about instead
      // of retyping the form — but the token is dropped either way, because
      // holding a live credential in a component's state after the request is
      // over buys nothing.
      setUrl("");
      setBranches("");
      setToken("");
      toast.success("Repository bound. The host has mirrored it.");
      await refresh();
    } catch (err) {
      setToken("");
      toast.error(
        err instanceof ApiError ? err.message : "Couldn't bind that repository.",
      );
    } finally {
      setBusy(null);
    }
  }

  async function revoke(key: string) {
    if (busy) return;
    setBusy(key);
    try {
      await revokeRepo(client, company, key);
      toast.success("Repository unbound. Its mirror and credential are gone.");
      await refresh();
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Couldn't unbind that repository.",
      );
    } finally {
      setBusy(null);
    }
  }

  if (load === "unavailable") return null;

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Repositories
        </h3>
        {load === "ready" && repos.length > 0 && (
          <Badge variant="secondary" className="gap-1">
            <Check className="size-3" /> {repos.length} bound
          </Badge>
        )}
      </div>

      <Card>
        <CardContent className="space-y-4 pt-6">
          <div className="space-y-1">
            <p className="text-sm font-medium">GitHub repositories</p>
            <p className="text-sm text-muted-foreground">
              Bind a repository and this host keeps its own copy, refreshed with the token you
              give it. The token stays on the host — it is never written into a workspace, an
              agent&apos;s environment, or any file your agents can read.
            </p>
          </div>

          <Alert>
            <Info className="size-4" />
            <AlertTitle>Nothing reads these yet</AlertTitle>
            <AlertDescription>
              This release stages the code on the host and stops there. The tools that let a
              teammate open a bound repository, and read a pull request&apos;s diff, arrive in a
              follow-up — binding now just means they will have something to read when they do.
              {!diffsAvailable &&
                " Pull-request diffs also need a build of this host with GitHub access compiled in; this one does not have it."}
            </AlertDescription>
          </Alert>

          {load === "loading" ? (
            <p className="text-sm text-muted-foreground">Loading…</p>
          ) : repos.length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing bound yet.</p>
          ) : (
            <ul className="divide-y rounded-md border">
              {repos.map((repo) => (
                <li key={repo.key} className="flex items-start gap-3 p-3">
                  <GitBranch className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                      {repo.owner}/{repo.repo}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {repo.branches.join(", ")} · {formatSize(repo.sizeBytes)} ·{" "}
                      {repo.lastFetchedMillis
                        ? `updated ${formatWhen(repo.lastFetchedMillis)}`
                        : "never fetched"}
                    </p>
                    <p className="font-mono text-2xs text-muted-foreground">
                      token {repo.tokenFingerprint}
                    </p>
                  </div>
                  {canManage && (
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={busy !== null}
                      onClick={() => void revoke(repo.key)}
                      aria-label={`Unbind ${repo.owner}/${repo.repo}`}
                    >
                      {busy === repo.key ? (
                        <Loader2 className="size-4 animate-spin" />
                      ) : (
                        <Trash2 className="size-4" />
                      )}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          )}

          {!canManage ? (
            <p className="rounded-md bg-muted/40 p-2 text-xs text-muted-foreground">
              A repository credential belongs to the company, so an admin manages it. The list
              above is the whole picture; ask an admin to add or remove one.
            </p>
          ) : (
            <div className="space-y-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label htmlFor="repo-url" className="text-xs">
                    Repository URL
                  </Label>
                  <Input
                    id="repo-url"
                    autoComplete="off"
                    placeholder="https://github.com/acme/widgets"
                    value={url}
                    onChange={(e) => setUrl(e.target.value)}
                  />
                </div>
                <div className="space-y-1">
                  <Label htmlFor="repo-branches" className="text-xs">
                    Branches (optional)
                  </Label>
                  <Input
                    id="repo-branches"
                    autoComplete="off"
                    placeholder="main, release — blank uses the default"
                    value={branches}
                    onChange={(e) => setBranches(e.target.value)}
                  />
                </div>
              </div>
              <div className="space-y-1">
                <Label htmlFor="repo-token" className="text-xs">
                  Fine-grained access token
                </Label>
                <Input
                  id="repo-token"
                  type="password"
                  autoComplete="off"
                  placeholder="github_pat_…"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                />
                {/* Said before they paste, not after the host refuses: a
                    classic token is the one mistake this form can prevent. */}
                <p className="text-xs text-muted-foreground">
                  GitHub → Settings → Developer settings → Personal access tokens → Fine-grained.
                  Scope it to this one repository and give it read-only access to Contents,
                  Metadata and Pull requests. A classic <code className="font-mono">ghp_</code>{" "}
                  token is refused — it can read everything the account can.
                </p>
              </div>
              <Button size="sm" disabled={busy !== null} onClick={() => void bind()}>
                {busy === "bind" && <Loader2 className="size-4 animate-spin" />}
                Bind repository
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    </section>
  );
}

/** Renders a byte count the way the host does, so the two agree. */
function formatSize(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit + 1 < units.length) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

/** A short relative time, or a date once it stops being useful as one. */
function formatWhen(millis: number): string {
  const seconds = Math.max(0, Math.round((Date.now() - millis) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(millis).toLocaleDateString();
}
