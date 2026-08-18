/**
 * The QA harness's judgements, pinned (issue #987).
 *
 * `qa/oc-qa.js` is pasted into a browser console, so nothing imports it and
 * nothing type-checks it. Two things about it are worth a gate anyway:
 *
 * 1. **It parses.** A syntax error is discovered by an operator mid-incident,
 *    which is the worst possible moment and the only moment it is ever run.
 *
 * 2. **Its run verdict agrees with the console's.** The harness owns a
 *    transcription of `run-health.ts`, and a second definition of "did this run
 *    succeed" is precisely the defect issue #981 filed against the product —
 *    where the console's TypeScript held the only verdict and every API client
 *    folding `nodes[].status` scored a dropped report as green. The harness made
 *    exactly that mistake and scored a delivery-failure run as PASS. Pinning the
 *    two together is what stops a change to the console's reading from silently
 *    re-greening a bad run in the harness.
 *
 * The script is evaluated in a `vm` sandbox rather than imported: it is an IIFE
 * that assigns `globalThis.OCQA`, with no `export`, because an `export` would
 * make it unpasteable — which is the one property it must not lose.
 */

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createContext, runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

import { runTone } from "@/views/workflows/run-health";
import type {
  DeliveryReport,
  WorkflowBlockedNode,
  WorkflowRunOutcome,
} from "@/api/workflows";

const here = dirname(fileURLToPath(import.meta.url));
const SCRIPT = resolve(here, "../../../qa/oc-qa.js");

/** One reported check, as the script emits it. */
interface Row {
  check: string;
  verdict: "PASS" | "WARN" | "FAIL" | "SKIP";
  value: string;
  note: string;
}

/** A minimal `Response` stand-in — the four things `http()` reads. */
function response(status: number, body: unknown, headers: Record<string, string> = {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: async () => (body === undefined ? "" : JSON.stringify(body)),
    headers: new Headers(headers),
  };
}

/** Loads `oc-qa.js` into a sandbox and hands back its `_internals`. */
function loadHarness(fetchImpl?: (path: string) => Promise<unknown>) {
  const source = readFileSync(SCRIPT, "utf8");
  const sandbox: Record<string, unknown> = {
    console: { log: () => {}, table: () => {} },
    fetch:
      fetchImpl ??
      (async () => {
        throw new Error("no network in this test");
      }),
    setTimeout,
    clearTimeout,
    AbortController,
    Headers: globalThis.Headers,
    location: { host: "test" },
  };
  createContext(sandbox);
  runInNewContext(source, sandbox);
  const ocqa = sandbox.OCQA as {
    version: string;
    read: (options?: { company?: string }) => Promise<Row[]>;
    probe: unknown;
    report: unknown;
    _internals: {
      runVerdict: (run: unknown) => string;
      undeliveredCount: (d: DeliveryReport[]) => number;
      pendingCount: (d: DeliveryReport[]) => number;
      awaitingCount: (run: unknown) => number;
      isBlocked: (run: unknown) => boolean;
      judgeCacheHeader: (kind: string, header: string | null) => { verdict: string; note: string };
      age: (atMillis: number, now?: number) => string;
      notWired: (res: { body: unknown }) => boolean;
      secs: (ms: number) => string;
    };
  };
  return ocqa;
}

/** A run outcome with only the fields the verdict reads. */
function run(overrides: Partial<WorkflowRunOutcome>): WorkflowRunOutcome {
  return {
    seq: 1,
    atMillis: 1_700_000_000_000,
    workflowId: "daily-release-readiness",
    scheduled: false,
    deliveries: [],
    pendingApprovals: [],
    ...overrides,
  } as WorkflowRunOutcome;
}

function delivery(status: DeliveryReport["status"]): DeliveryReport {
  return { node: "report", kind: "channel", target: "operator", status, detail: "" };
}

/** A node the run stopped short at, waiting on a person (issue #881). */
function blocked(nodeId: string): WorkflowBlockedNode {
  return { nodeId, tools: ["send_email"] };
}

describe("oc-qa.js loads", () => {
  it("parses and exposes both entry points", () => {
    const ocqa = loadHarness();
    expect(typeof ocqa.read).toBe("function");
    expect(typeof ocqa.probe).toBe("function");
    expect(typeof ocqa.report).toBe("function");
    expect(ocqa.version).toMatch(/^\d+\.\d+\.\d+$/);
  });
});

describe("read() against a host whose surfaces do not answer", () => {
  /**
   * Rule 2 of the harness, end to end: **unreadable is never PASS.**
   *
   * The 2026-08-18 pass wrote three checks up as green that had never run, and
   * a pure-function test cannot catch that — the mistake lives in the plumbing,
   * where a 404 body decays to `{}` and every field read off it comes back
   * `undefined`. So this drives the real `read()` over a host that answers
   * `/healthz`, `/spec` and the company status and 404s everything else, and
   * asserts nothing downstream claims to have passed.
   */
  const reachable: Record<string, unknown> = {
    "/healthz": { status: "ok" },
    "/spec": {
      name: "opencompany",
      version: "0.1.0",
      capabilities: ["rest", "graphql"],
      storage: "memory",
      instance_id: "abcdef0123456789",
    },
    "/api/v1/company": { id: "acme", name: "Acme", lifecycle: "running", pending_approvals: 0 },
  };

  async function runRead(): Promise<Row[]> {
    const ocqa = loadHarness(async (path: string) =>
      path in reachable ? response(200, reachable[path]) : response(404, { error: "not found" }),
    );
    return ocqa.read();
  }

  it("emits exactly the 22 checks the docs claim", async () => {
    // Pins the number in `qa/README.md` and `qa/MASTER-QA.md` to the code. The
    // cache-header check expands to three rows on a host that actually serves a
    // console; here the shell 404s, so it contributes its single SKIP.
    const rows = await runRead();
    expect(rows).toHaveLength(22);
    expect(new Set(rows.map((r) => r.check)).size).toBe(22);
  });

  it("passes only what it could actually read", async () => {
    const rows = await runRead();
    const passed = rows.filter((r) => r.verdict === "PASS").map((r) => r.check);
    expect(passed.sort()).toEqual(["company-lifecycle", "host"]);
  });

  it("reports every unreadable surface as untested rather than as passed", async () => {
    const rows = await runRead();
    const unreadable = rows.filter((r) => r.check !== "host" && r.check !== "company-lifecycle");
    // `approval-tier` and `repo-binding` are SKIP by construction, not by 404 —
    // the tier has no read surface at all and the build binding needs a human.
    for (const r of unreadable) {
      expect(r.verdict, `${r.check} judged a surface it never read`).toBe("SKIP");
    }
  });

  it("never renders an absent value as a confident zero", async () => {
    // A 404 body decays to `{}`, so a check that formats `body.count` prints
    // `undefined` or `0` and reads as a real, healthy reading. Every SKIP row
    // must say why instead.
    const rows = await runRead();
    for (const r of rows.filter((x) => x.verdict === "SKIP")) {
      expect(r.note, `${r.check} skipped without saying why`).not.toBe("");
    }
  });
});

describe("runVerdict agrees with the console's runTone", () => {
  /**
   * Every arm of the precedence order, including the two that only exist
   * because leaving them out reads as success: a run still in flight and a run
   * somebody stopped both fall through to green without their own arm.
   */
  const cases: Array<{ name: string; run: WorkflowRunOutcome; label: string }> = [
    { name: "still running", run: run({ running: true }), label: "running" },
    {
      name: "running outranks a failure it has not hit yet",
      run: run({ running: true, error: "boom" }),
      label: "running",
    },
    { name: "errored", run: run({ error: "node exploded" }), label: "failed" },
    {
      name: "stopped, judged before its deliveries",
      run: run({ cancelled: true, deliveries: [delivery("failed")] } as Partial<WorkflowRunOutcome>),
      label: "stopped",
    },
    {
      // #881: no error, not cancelled, not running, and no report routed — so
      // before this arm existed it fell through to green and told the operator
      // a pipeline that delivered nothing had succeeded.
      name: "stopped short at a gate",
      run: run({ blockedNodes: [blocked("approve")] }),
      label: "blocked",
    },
    {
      name: "blocked is judged before the delivery rows",
      run: run({ blockedNodes: [blocked("approve")], deliveries: [delivery("failed")] }),
      label: "blocked",
    },
    {
      name: "every node ok but the report was dropped (#981)",
      run: run({ deliveries: [delivery("failed")] }),
      label: "not delivered",
    },
    {
      name: "skipped counts as undelivered",
      run: run({ deliveries: [delivery("skipped")] }),
      label: "not delivered",
    },
    {
      name: "denied counts as undelivered",
      run: run({ deliveries: [delivery("denied")] }),
      label: "not delivered",
    },
    {
      name: "undelivered outranks pending",
      run: run({ deliveries: [delivery("pending"), delivery("failed")] }),
      label: "not delivered",
    },
    {
      name: "parked for approval is not a failure",
      run: run({ deliveries: [delivery("pending")] }),
      label: "awaiting approval",
    },
    {
      // #846: the gated shape. It never reached an output node, so `deliveries`
      // is empty and a delivery-only read scored it as a clean run.
      name: "paused at a gate, having routed no report at all",
      run: run({ pendingApprovals: ["approve"] }),
      label: "awaiting approval",
    },
    { name: "delivered", run: run({ deliveries: [delivery("sent")] }), label: "ok" },
    { name: "nothing to deliver", run: run({}), label: "ok" },
  ];

  /** The console's label for a run, mapped to the harness's verdict word. */
  const TONE_TO_VERDICT: Record<string, string> = {
    running: "running",
    failed: "failed",
    stopped: "stopped",
    blocked: "blocked",
    "not delivered": "undelivered",
    "awaiting approval": "awaiting-approval",
    ok: "ok",
  };

  for (const c of cases) {
    it(c.name, () => {
      const { runVerdict } = loadHarness()._internals;
      // Both halves asserted: the console still reads it the way this table
      // says, AND the harness agrees with the console. Asserting only the
      // second would pass vacuously if both drifted together.
      expect(runTone(c.run).label).toBe(c.label);
      expect(runVerdict(c.run)).toBe(TONE_TO_VERDICT[c.label]);
    });
  }

  it("folds gates and parked reports into one waiting-on-a-person count (#846)", () => {
    const { awaitingCount } = loadHarness()._internals;
    // Either half alone is a reading that scored a waiting run as finished.
    expect(awaitingCount(run({ pendingApprovals: ["a"], deliveries: [] }))).toBe(1);
    expect(awaitingCount(run({ pendingApprovals: [], deliveries: [delivery("pending")] }))).toBe(1);
    expect(
      awaitingCount(run({ pendingApprovals: ["a", "b"], deliveries: [delivery("pending")] })),
    ).toBe(3);
    expect(awaitingCount(run({}))).toBe(0);
  });

  it("counts deliveries the same way the console does", () => {
    const { undeliveredCount, pendingCount } = loadHarness()._internals;
    const deliveries = [
      delivery("sent"),
      delivery("pending"),
      delivery("failed"),
      delivery("skipped"),
    ];
    expect(undeliveredCount(deliveries)).toBe(2);
    expect(pendingCount(deliveries)).toBe(1);
  });

  it("reports an unknown run as unknown rather than ok", () => {
    const { runVerdict } = loadHarness()._internals;
    expect(runVerdict(null)).toBe("unknown");
  });
});

describe("judgeCacheHeader — the #979 reading", () => {
  it("fails an HTML response with no cache-control at all", () => {
    // The bug exactly: no header means heuristic caching, so the returning
    // browser keeps yesterday's shell. An absent header must not be a WARN.
    const { judgeCacheHeader } = loadHarness()._internals;
    expect(judgeCacheHeader("html", null).verdict).toBe("FAIL");
    expect(judgeCacheHeader("html", "").verdict).toBe("FAIL");
  });

  it("fails an HTML response the browser is allowed to reuse", () => {
    const { judgeCacheHeader } = loadHarness()._internals;
    expect(judgeCacheHeader("html", "public, max-age=3600").verdict).toBe("FAIL");
  });

  it("passes an HTML response that revalidates", () => {
    const { judgeCacheHeader } = loadHarness()._internals;
    for (const header of ["no-cache", "no-store", "public, max-age=0, must-revalidate"]) {
      expect(judgeCacheHeader("html", header).verdict, header).toBe("PASS");
    }
  });

  it("passes a hashed asset cached long, and only warns when it is not", () => {
    const { judgeCacheHeader } = loadHarness()._internals;
    expect(judgeCacheHeader("asset", "public, max-age=31536000, immutable").verdict).toBe("PASS");
    expect(judgeCacheHeader("asset", "public, max-age=86400").verdict).toBe("PASS");
    // Never a FAIL: a short-lived hashed asset costs a revalidation, not a
    // white screen. Only the shell can break the app by being cached.
    expect(judgeCacheHeader("asset", null).verdict).toBe("WARN");
    expect(judgeCacheHeader("asset", "public, max-age=60").verdict).toBe("WARN");
  });
});

describe("notWired — a feature absent from the build is untested, not failed", () => {
  /**
   * Found by running the harness against a real default-feature host: it
   * answered `POST …/workflows/{id}/run` with
   * `404 {"code":"not_wired"}` — "this deployment has no workflow runner" —
   * and the probe scored it FAIL, which would send somebody chasing a graph
   * that was fine.
   */
  it("recognises the typed code", () => {
    const { notWired } = loadHarness()._internals;
    expect(notWired({ body: { error: "workflow execution is not wired", code: "not_wired" } })).toBe(
      true,
    );
  });

  it("does not match on the prose, only the code (#248)", () => {
    // The message is free to be reworded; a check that grepped it would go
    // quiet the day somebody did, and silently start scoring absent features
    // as failures again.
    const { notWired } = loadHarness()._internals;
    expect(notWired({ body: { error: "workflow execution is not wired in this deployment" } })).toBe(
      false,
    );
    expect(notWired({ body: { error: "boom", code: "internal" } })).toBe(false);
    expect(notWired({ body: null })).toBe(false);
  });
});

describe("secs", () => {
  it("keeps a sub-second reading legible instead of rounding it to 0.0s", () => {
    // The live run printed every probe as "0.0s", which reads as a broken
    // clock rather than a fast host — and latency is one of the values the
    // chat probe's verdict is formed from.
    const { secs } = loadHarness()._internals;
    expect(secs(0)).toBe("0ms");
    expect(secs(42)).toBe("42ms");
    expect(secs(999)).toBe("999ms");
    expect(secs(1000)).toBe("1.0s");
    expect(secs(6040)).toBe("6.0s");
    expect(secs(121_000)).toBe("121.0s");
  });
});

describe("age", () => {
  it("reports absent timestamps as n/a rather than as 'just now'", () => {
    // `memoryStats.factsUpdatedAtMillis` and friends are `0` when nothing has
    // been written. Rendering that as a fresh timestamp would report an empty
    // company as an active one.
    const { age } = loadHarness()._internals;
    expect(age(0)).toBe("n/a");
  });

  it("scales from seconds to days", () => {
    const { age } = loadHarness()._internals;
    const now = 1_700_000_000_000;
    expect(age(now - 30_000, now)).toBe("30s");
    expect(age(now - 5 * 60_000, now)).toBe("5m");
    expect(age(now - 4 * 3_600_000, now)).toBe("4h");
    expect(age(now - 3 * 86_400_000, now)).toBe("3d");
  });
});
