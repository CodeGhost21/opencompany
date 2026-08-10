// One connection's console: discovery, company selection, and the app shell.
//
// This is the phase machine `App` used to own. Moving it here is what makes a
// second host an *instance* rather than a second code path — and it is what
// makes failure local. `App`'s version set a single global phase, so any host
// being unreachable blanked the whole console and any expired session dropped
// the whole app to a login screen. With N connections that is the wrong shape:
// one host being down has to redden one row and leave the others working.

import { useCallback, useEffect, useMemo, useState } from "react";
import { Loader2 } from "lucide-react";

import { OpenCompanyClient } from "@/api/client";
import { ApiError, type CompanyStatus } from "@/api/types";
import { AppShell } from "@/components/app-shell";
import { CompanyPicker } from "@/components/company-picker";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { ConnectionScopeProvider } from "@/connections/ConnectionContext";
import { probe, useConnection } from "@/connections/registry";
import type { ConnectionId } from "@/connections/types";
import { Login } from "@/views/Login";

type Phase =
  | { kind: "loading" }
  | { kind: "error"; message: string; hint?: string }
  | { kind: "login"; company: string | null; notice?: string }
  | { kind: "picker"; companies: CompanyStatus[] }
  | {
      kind: "console";
      company: string | null;
      status: CompanyStatus;
      companies: CompanyStatus[];
      canGoBack: boolean;
    };

interface Props {
  connectionId: ConnectionId;
  client: OpenCompanyClient;
  /** The company this console was configured to open, if any. */
  defaultCompany: string | null;
  /** A notice from a sign-in attempt that happened before this mounted. */
  notice?: string;
  /** Forces the sign-in view — a magic link that failed to redeem, say. */
  forceLogin?: boolean;
}

export function ConnectionConsole({
  connectionId,
  client,
  defaultCompany,
  notice,
  forceLogin,
}: Props) {
  const [phase, setPhase] = useState<Phase>(
    forceLogin ? { kind: "login", company: defaultCompany, notice } : { kind: "loading" },
  );
  // Incremented to re-run discovery on demand. The boot effect's dependencies
  // (`client`, `defaultCompany`, `forceLogin`) never change when a sign-in
  // completes, so without an explicit epoch a successful re-login would either
  // hang on "Connecting…" (nothing re-ran the boot) or reload the document
  // (every other connection's stream died with it).
  const [bootEpoch, setBootEpoch] = useState(0);
  const connection = useConnection(connectionId);

  // The registry marks this connection `unauthenticated` when its client sees a
  // 401 — that is the *only* 401 handler, so one host refusing a credential
  // cannot reach into another host's view. Deriving the sign-in screen from
  // that status rather than from a second callback is what keeps it that way.
  const refused = connection?.status === "unauthenticated";

  // Re-probe this connection and re-run the boot effect. A reload would tear
  // down every *other* connection's stream to recover this one; bumping the
  // epoch keeps the recovery local.
  const reBoot = useCallback(() => {
    void probe(connectionId).then(() => {
      setPhase({ kind: "loading" });
      setBootEpoch((n) => n + 1);
    });
  }, [connectionId]);

  useEffect(() => {
    // `forceLogin` forces the sign-in view until the *first* boot; a sign-in
    // bumps the epoch, so the forced-login recover cannot re-enter the boot.
    if (forceLogin && bootEpoch === 0) return;
    let cancelled = false;
    const set = (p: Phase) => !cancelled && setPhase(p);

    async function boot() {
      // Explicit company wins: go straight to its console.
      if (defaultCompany) {
        try {
          const status = await client.status(defaultCompany);
          set({
            kind: "console",
            company: defaultCompany,
            status,
            companies: [status],
            canGoBack: false,
          });
        } catch (err) {
          set(connectionError(client, err, defaultCompany));
        }
        return;
      }

      try {
        const companies = await client.listCompanies();
        if (companies.length === 1) {
          const c = companies[0];
          set({ kind: "console", company: c.id, status: c, companies, canGoBack: false });
        } else if (companies.length > 1) {
          set({ kind: "picker", companies });
        } else {
          set({
            kind: "error",
            message: "No companies are running on this host.",
            hint: "Start one with `opencompany serve --company <dir>`.",
          });
        }
      } catch (listErr) {
        // Fall back to the single-company alias (prosumer serve).
        try {
          const status = await client.status(null);
          set({ kind: "console", company: null, status, companies: [], canGoBack: false });
        } catch {
          set(connectionError(client, listErr, defaultCompany));
        }
      }
    }

    void boot();
    return () => {
      cancelled = true;
    };
  }, [client, defaultCompany, forceLogin, bootEpoch]);

  const switchCompany = useCallback(
    async (id: string, companies: CompanyStatus[]) => {
      try {
        const status = await client.status(id);
        setPhase({ kind: "console", company: id, status, companies, canGoBack: true });
      } catch (err) {
        setPhase(connectionError(client, err, id));
      }
    },
    [client],
  );

  const backToPicker = useCallback(() => {
    void client
      .listCompanies()
      .then((companies) => setPhase({ kind: "picker", companies }))
      .catch((err: unknown) => setPhase(connectionError(client, err, null)));
  }, [client]);

  const consoleCompany = phase.kind === "console" ? phase.company : null;
  const scope = useMemo(
    () => ({ connection: connectionId, company: consoleCompany }),
    [connectionId, consoleCompany],
  );

  if (refused && phase.kind !== "login") {
    return (
      <Login
        client={client}
        company={defaultCompany}
        notice={notice}
        onSignedIn={reBoot}
      />
    );
  }

  switch (phase.kind) {
    case "loading":
      return (
        <FullScreen>
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Connecting…
          </div>
        </FullScreen>
      );

    case "login":
      return (
        <Login
          client={client}
          company={phase.company}
          notice={phase.notice}
          onSignedIn={reBoot}
        />
      );

    case "error":
      return (
        <FullScreen>
          <div className="w-full max-w-md space-y-4" data-testid="connection-error">
            <Alert variant="destructive">
              <AlertTitle>Can&apos;t connect</AlertTitle>
              <AlertDescription>
                {phase.message}
                {phase.hint && (
                  <span className="mt-1 block font-mono text-xs opacity-80">{phase.hint}</span>
                )}
              </AlertDescription>
            </Alert>
            <Button className="w-full" onClick={() => location.reload()}>
              Retry
            </Button>
          </div>
        </FullScreen>
      );

    case "picker":
      return (
        <CompanyPicker
          companies={phase.companies}
          onPick={(id) => void switchCompany(id, phase.companies)}
        />
      );

    case "console":
      return (
        // The remount key carries the connection as well as the company. It was
        // already the isolation primitive — a company switch throws away every
        // piece of in-flight view state rather than reconciling it — and the
        // comments in `app-shell.tsx` about "another company's channel ids are
        // another namespace" are only true again once the host is in the key
        // too. Two hosts each serving an `acme` are two namespaces.
        <ConnectionScopeProvider scope={scope}>
          <AppShell
            key={`${connectionId}:${phase.company ?? "single"}`}
            client={client}
            company={phase.company}
            initialStatus={phase.status}
            companies={phase.companies}
            onSwitchCompany={(id) => void switchCompany(id, phase.companies)}
            onBackToPicker={phase.canGoBack ? backToPicker : undefined}
          />
        </ConnectionScopeProvider>
      );
  }
}

function FullScreen({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid min-h-svh place-items-center bg-background p-6 text-center">
      {children}
    </div>
  );
}

function connectionError(
  client: OpenCompanyClient,
  err: unknown,
  company: string | null,
): Phase {
  const where = client.baseUrl || "this origin";
  if (err instanceof ApiError && err.status === 401) {
    // A 401 now usually means "no session", not "no operator token" — humans
    // sign in. Offering the login view is right for a user and harmless for an
    // operator, who can still pass ?token=.
    return { kind: "login", company };
  }
  return {
    kind: "error",
    message: `Couldn't reach a company host at ${where}.`,
    hint: "Set the host with ?api=<url>, or run `opencompany serve`.",
  };
}
