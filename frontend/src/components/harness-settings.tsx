// Coding harnesses this machine can drive over ACP (issue #1245's UI half).
//
// Local only, like `DevicePairing`: readiness is a property of this machine,
// not of the company or host the console happens to be pointed at, so this
// renders nothing outside the desktop build and takes neither `client` nor
// `company`. It has no "add a harness" flow yet — `acp::discovery`'s catalogue
// is a fixed list (claude, codex, goose), each installed from its own
// upstream, so there is nothing here to add to yet, only to check on.

import { useCallback, useEffect, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";

import { isDesktopRuntime } from "@/api/transport";
import {
  acpHarnesses,
  type AcpHarnessStatus,
  type AcpReadiness,
} from "@/api/transport/desktop";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { cn } from "@/lib/utils";

/**
 * How a harness's readiness reads, and what colour says so.
 *
 * `label` carries the meaning; the dot is the shorthand — same rule
 * `host-switcher.tsx`'s `STATUS_COPY` follows, for the same reason: hue alone
 * tells an operator that something differs without saying what.
 */
const READINESS_COPY: Record<AcpReadiness["state"], { label: string; dot: string }> = {
  ready: { label: "Ready", dot: "bg-status-done" },
  notSignedIn: { label: "Not signed in", dot: "bg-status-blocked" },
  notInstalled: { label: "Not installed", dot: "bg-muted-foreground/50" },
  spawnFailed: { label: "Won't start", dot: "bg-destructive" },
};

function ReadinessPill({ readiness }: { readiness: AcpReadiness }) {
  const copy = READINESS_COPY[readiness.state];
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full border bg-card px-2.5 py-0.5 text-xs font-medium">
      <span className={cn("size-1.5 rounded-full", copy.dot)} />
      {copy.label}
    </span>
  );
}

/** The second line under a harness's name: why it's not ready, or where it is. */
function HarnessDetail({ harness }: { harness: AcpHarnessStatus }) {
  if (harness.readiness.state === "spawnFailed") {
    return (
      <p className="mt-0.5 truncate text-xs text-muted-foreground" title={harness.readiness.reason}>
        {harness.readiness.reason}
      </p>
    );
  }
  if (harness.readiness.state === "notInstalled") {
    return <p className="mt-0.5 text-xs text-muted-foreground">Not found on PATH.</p>;
  }
  if (harness.readiness.state === "notSignedIn") {
    return <p className="mt-0.5 text-xs text-muted-foreground">Installed, but not signed in yet.</p>;
  }
  return harness.path ? (
    <p className="mt-0.5 truncate font-mono text-xs text-muted-foreground" title={harness.path}>
      {harness.path}
    </p>
  ) : null;
}

/**
 * Lists every harness `acp::discovery` knows about, and whether each is
 * spawnable right now — the "harness management" settings surface, modeled on
 * `buzz-agent`'s `HarnessesSettingsPanel` but without its install/add-runtime
 * catalog: OpenCompany has nothing to add yet, only a fixed set to check on.
 */
export function HarnessSettings() {
  const [harnesses, setHarnesses] = useState<AcpHarnessStatus[] | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setHarnesses(await acpHarnesses());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // A browser has no local harnesses to speak of — there is nothing this
  // console could spawn from a webview.
  if (!isDesktopRuntime()) return null;
  // `null` means a shell built before `oc_acp_harnesses` existed. Degrading to
  // nothing here matches `DevicePairing`/`localInstances`'s own precedent: an
  // older shell should look like it never had the feature, not like a broken
  // one.
  if (!loading && harnesses === null) return null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">Coding harnesses</CardTitle>
        <CardDescription>
          External coding CLIs a teammate can run through, on this machine.
        </CardDescription>
        <CardAction>
          <Button
            variant="outline"
            size="sm"
            disabled={loading}
            onClick={() => void load()}
          >
            <RefreshCw className={cn("size-4", loading && "animate-spin")} />
            Check again
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="space-y-0 divide-y">
        {loading && !harnesses ? (
          <div className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
            <Loader2 className="size-4 animate-spin" /> Checking installed harnesses…
          </div>
        ) : (
          harnesses?.map((harness) => (
            <div
              key={harness.id}
              className="flex items-center justify-between gap-4 py-3 first:pt-0 last:pb-0"
            >
              <div className="min-w-0">
                <p className="text-sm font-medium">{harness.label}</p>
                <HarnessDetail harness={harness} />
              </div>
              <ReadinessPill readiness={harness.readiness} />
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}
