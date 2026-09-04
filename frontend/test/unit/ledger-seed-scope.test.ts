import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * `test/e2e/ledger.ts` seeds and clears a company's `ledger.jsonl` on the data
 * root the managed e2e host is serving. That root is only ever the suite's own
 * disposable scratch area (`target/e2e/...`) when `PW_HOST_DATA_DIR` is unset —
 * a caller who sets it to a root they keep across runs gets that root reused
 * as-is by `test/e2e/host.sh`, not wiped.
 *
 * `clearLedger()` used to `rmSync` unconditionally, so a Finance spec run
 * against a reused root deleted the company's entire pre-existing ledger,
 * including entries that had nothing to do with the test. The property under
 * test: seeding and clearing only ever touch a root this suite created, never
 * one a caller supplied.
 */

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../..");
const scratchRoot = join(repoRoot, "target/e2e");

const originalEnv = {
  PW_BASE_URL: process.env.PW_BASE_URL,
  PW_HOST_DATA_DIR: process.env.PW_HOST_DATA_DIR,
  PW_FIRST_RUN: process.env.PW_FIRST_RUN,
  PW_EULER: process.env.PW_EULER,
};

let disposableHome: string;
let persistentHome: string;

beforeEach(() => {
  delete process.env.PW_BASE_URL;
  delete process.env.PW_FIRST_RUN;
  delete process.env.PW_EULER;

  mkdirSync(scratchRoot, { recursive: true });
  disposableHome = join(scratchRoot, `ledger-seed-scope-disposable-${process.pid}`);
  persistentHome = mkdtempSync(join(tmpdir(), "ledger-seed-scope-persistent-"));
});

afterEach(() => {
  rmSync(disposableHome, { recursive: true, force: true });
  rmSync(persistentHome, { recursive: true, force: true });

  for (const [name, value] of Object.entries(originalEnv)) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
});

async function freshLedger() {
  vi.resetModules();
  return await import("../e2e/ledger");
}

describe("ledger seeding scope", () => {
  it("seeds and clears normally on the suite's own disposable data root", async () => {
    process.env.PW_HOST_DATA_DIR = disposableHome;
    const { clearLedger, ledgerPath, seedLedger } = await freshLedger();

    const path = ledgerPath();
    expect(path).not.toBeNull();

    seedLedger([{ atMillis: Date.now(), kind: "inference.spend", amountUsd: -0.12, memo: "test" }]);
    expect(readFileSync(path!, "utf8")).toContain("test");

    clearLedger();
    expect(() => readFileSync(path!, "utf8")).toThrow();
  });

  it("never deletes a caller-supplied root's pre-existing ledger entries", async () => {
    const companyDir = join(persistentHome, "companies", "e2e-harness-co");
    mkdirSync(companyDir, { recursive: true });
    const preExistingLedgerPath = join(companyDir, "ledger.jsonl");
    const realEntry = JSON.stringify({
      at_millis: Date.now() - 86_400_000,
      kind: "inference.spend",
      amount_usd: -4.2,
      memo: "real spend unrelated to this test",
    });
    writeFileSync(preExistingLedgerPath, realEntry + "\n");

    process.env.PW_HOST_DATA_DIR = persistentHome;
    const { clearLedger, ledgerPath, seedLedger } = await freshLedger();

    // The helper must refuse to operate on a root it did not create.
    expect(ledgerPath()).toBeNull();

    expect(seedLedger([{ atMillis: Date.now(), kind: "inference.spend", amountUsd: -0.005, memo: "seed" }])).toBe(
      false,
    );

    // The destructive path this guards against: clearing must not touch a
    // pre-existing file on a root the suite does not own.
    clearLedger();

    const survived = readFileSync(preExistingLedgerPath, "utf8");
    expect(survived).toContain("real spend unrelated to this test");
    expect(survived).not.toContain("\"memo\":\"seed\"");
  });
});
