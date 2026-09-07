import { appendFileSync, mkdirSync, realpathSync, rmSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { MANAGED_HOST_HOME } from "./host-identity";

/**
 * Seeding for the company's financial ledger.
 *
 * The Finance surface folds `ledger.jsonl` and nothing else, and nothing in this
 * suite could put a line in it — the ledger fills from the inference cost hook,
 * which needs a real billed turn. So every Finance assertion until now ran
 * against an empty ledger, where the tile and the transaction list agree because
 * both are zero. That is the gap that let a whole-dollar tile ship over a
 * cent-precision list.
 *
 * This writes the same file the cost hook appends to, in the same shape
 * (`LedgerEntry` in `src/ports/types.rs`), so the host reads it through its own
 * projection with no test-only route in the way. `GET …/finances` loads the
 * record per request, so a seed lands on the next navigation.
 */

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../..");

/**
 * Whether `home` is the suite's own disposable scratch data root
 * (`target/e2e`) or somewhere inside it — the same boundary `host.sh` checks
 * before it decides to wipe a data root rather than reuse it.
 *
 * A root outside `target/e2e` reaches this file only when a caller supplied
 * `PW_HOST_DATA_DIR` themselves, and `host.sh` deliberately leaves such a root
 * untouched so it can be reused across runs. Seeding or clearing a ledger
 * there would create or destroy state in a root this suite does not own, so
 * every write below is gated on this check first.
 */
export function isDisposableRoot(home: string): boolean {
  const scratch = join(repoRoot, "target/e2e");
  mkdirSync(scratch, { recursive: true });
  mkdirSync(home, { recursive: true });
  const canonicalScratch = realpathSync(scratch);
  const canonicalHome = realpathSync(home);
  return (
    canonicalHome !== canonicalScratch &&
    canonicalHome.startsWith(canonicalScratch + sep)
  );
}

/** The harness company's id, as the host registers `companies/e2e_harness`. */
export const HARNESS_COMPANY_ID = "e2e-harness-co";

/** One line of `ledger.jsonl`. Outflows are negative, inflows positive. */
export interface SeedEntry {
  atMillis: number;
  kind: string;
  amountUsd: number;
  memo: string;
}

/**
 * Where this run's ledger lives, or `null` when this file has no business
 * writing there at all: the suite did not bring the host up (`PW_BASE_URL`),
 * or it did but was pointed at a data root outside its own scratch area — one
 * `host.sh` is reusing rather than wiping, and so is not this file's to touch.
 */
export function ledgerPath(companyId = HARNESS_COMPANY_ID): string | null {
  if (!MANAGED_HOST_HOME) return null;
  if (!isDisposableRoot(MANAGED_HOST_HOME)) return null;
  return join(MANAGED_HOST_HOME, "companies", companyId, "ledger.jsonl");
}

/** Appends entries to the company ledger. Returns false when unreachable. */
export function seedLedger(
  entries: SeedEntry[],
  companyId = HARNESS_COMPANY_ID,
): boolean {
  const path = ledgerPath(companyId);
  if (!path) return false;
  mkdirSync(join(MANAGED_HOST_HOME!, "companies", companyId), { recursive: true });
  const lines = entries
    .map((e) =>
      JSON.stringify({
        at_millis: e.atMillis,
        kind: e.kind,
        amount_usd: e.amountUsd,
        memo: e.memo,
      }),
    )
    .join("\n");
  appendFileSync(path, lines + "\n");
  return true;
}

/**
 * Removes the seeded ledger.
 *
 * The whole file, because this suite is the only thing that writes it on the
 * disposable data root it seeds into — a surgical removal would need to
 * re-read and rewrite append-only data the host may be holding open. Goes
 * through {@link ledgerPath}, so a data root this suite does not own is left
 * alone rather than wiped.
 */
export function clearLedger(companyId = HARNESS_COMPANY_ID): void {
  const path = ledgerPath(companyId);
  if (!path) return;
  rmSync(path, { force: true });
}

/**
 * This month's spend as a handful of realistic lines, totalling $0.163.
 *
 * The amounts are the point. $0.12 and $0.04 are ordinary sub-dollar inference
 * turns; $0.003 is a metered search, which is real spend smaller than a cent.
 * Together they are the case a whole-dollar tile reported as "$0" while the list
 * beneath it showed three outflows.
 */
export function subDollarMonth(now = Date.now()): SeedEntry[] {
  return [
    {
      atMillis: now - 3_600_000,
      kind: "inference.spend",
      amountUsd: -0.12,
      memo: "Model turn — drafting reply",
    },
    {
      atMillis: now - 1_800_000,
      kind: "inference.spend",
      amountUsd: -0.04,
      memo: "Model turn — summarising thread",
    },
    {
      atMillis: now - 900_000,
      kind: "tools.search",
      amountUsd: -0.003,
      memo: "Metered web search",
    },
  ];
}

/**
 * A single outflow at exactly half a cent, negative.
 *
 * `balance_usd` (`finances_from`) is the bookkeeping sum across the whole
 * ledger, passed to `usd` as a raw signed float rather than pre-`abs`'d —
 * unlike a transaction row's `amount_usd`, which the host already sends
 * non-negative. That is the one call site where a sign/magnitude boundary bug
 * in the shared formatter reaches the screen.
 */
export function halfCentDebit(now = Date.now()): SeedEntry[] {
  return [
    {
      atMillis: now - 60_000,
      kind: "inference.spend",
      amountUsd: -0.005,
      memo: "Half-cent debit",
    },
  ];
}
