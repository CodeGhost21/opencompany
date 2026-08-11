import { describe, expect, it } from "vitest";

import type { OpenCompanyClient } from "@/api/client";
import {
  isDetached,
  isDryRun,
  runWorkflow,
  type WorkflowRunResponse,
} from "@/api/workflows";

/**
 * Dry run / test mode on the console API surface (issue #542).
 *
 * Two pure-logic contracts carry the whole feature on the client:
 *
 * - `runWorkflow({ dryRun: true })` must post `dry_run: true` (snake_case, the
 *   wire spelling) and NOTHING when the option is absent — a real run's body
 *   must stay byte-for-byte what it always was;
 * - `isDryRun` must discriminate on the RESPONSE, never on what was asked. An
 *   older host ignores the request flag and runs FOR REAL, answering with a body
 *   that has no `dryRun` key — and the console leans on `isDryRun` returning
 *   `false` there to warn the operator the run was real.
 */

/** A fake client that records the last POST body. */
function capturingClient(sink: { path?: string; body?: unknown }): OpenCompanyClient {
  return {
    scopeFor: (company: string | null) => `/api/v1/${company ?? "company"}`,
    post: async <T>(path: string, body: unknown): Promise<T> => {
      sink.path = path;
      sink.body = body;
      return { output: {}, pendingApprovals: [] } as unknown as T;
    },
  } as unknown as OpenCompanyClient;
}

describe("runWorkflow dry-run payload", () => {
  it("posts dry_run: true when the dryRun option is set", async () => {
    const sink: { path?: string; body?: unknown } = {};
    await runWorkflow(capturingClient(sink), "acme", "demo", { request: "x" }, { dryRun: true });
    expect(sink.path).toBe("/api/v1/acme/workflows/demo/run");
    expect(sink.body).toEqual({ input: { request: "x" }, dry_run: true });
  });

  it("posts NO dry_run key for an ordinary run — the real-run body is unchanged", async () => {
    const sink: { path?: string; body?: unknown } = {};
    await runWorkflow(capturingClient(sink), "acme", "demo");
    expect(sink.body).toEqual({ input: {} });
    expect((sink.body as Record<string, unknown>).dry_run).toBeUndefined();
  });

  it("composes dry_run with detach without dropping either", async () => {
    const sink: { path?: string; body?: unknown } = {};
    await runWorkflow(capturingClient(sink), null, "demo", {}, { detach: true, dryRun: true });
    expect(sink.body).toEqual({ input: {}, detach: true, dry_run: true });
  });
});

describe("isDryRun discriminates on the response shape", () => {
  it("is true only when the host echoes dryRun: true", () => {
    const dry: WorkflowRunResponse = { output: {}, pendingApprovals: [], dryRun: true };
    expect(isDryRun(dry)).toBe(true);
  });

  it("is false when the response omits dryRun — the old-host 'ran for real' signal", () => {
    // This is the case that raises the loud warning: the console asked for a dry
    // run, the host ignored it, and the absent marker is the only tell.
    const real: WorkflowRunResponse = { output: {}, pendingApprovals: [] };
    expect(isDryRun(real)).toBe(false);
  });

  it("is false on a detached acceptance, which carries no output or marker", () => {
    const accepted: WorkflowRunResponse = { runId: "r1", detached: true };
    expect(isDetached(accepted)).toBe(true);
    expect(isDryRun(accepted)).toBe(false);
  });
});
