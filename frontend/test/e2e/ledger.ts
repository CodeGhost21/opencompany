import { appendFileSync, mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";

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
 * Where this run's ledger lives, or `null` when the suite did not bring the
 * host up and so cannot reach its disk (`PW_BASE_URL`).
 */
export function ledgerPath(companyId = HARNESS_COMPANY_ID): string | null {
  if (!MANAGED_HOST_HOME) return null;
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
 * The whole file, because this suite is the only thing that writes it on an
 * offline harness host — a surgical removal would need to re-read and rewrite
 * append-only data the host may be holding open.
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
