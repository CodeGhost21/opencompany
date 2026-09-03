/**
 * The one renderer for a real-money USD amount held as a dollar float.
 *
 * Cent precision, always — a tile and the rows beneath it read the same number
 * because they run the same formatter, not because two call sites happen to
 * agree on an option. An amount too small to show at that precision reads as a
 * bound rather than as zero, so a non-zero total never renders as free.
 *
 * Sibling formatters, deliberately not merged into this one:
 * - `lib/cost.ts` — LLM token cost, which is legitimately sub-cent and carries
 *   its own four-digit line precision. Same floor vocabulary as here.
 * - `views/finance/money.ts` — integer minor units in an arbitrary currency,
 *   where the digit count is asked of `Intl` per currency.
 */

/** Cent precision, pinned at both ends so a whole amount still shows `.00`. */
const CENT_DIGITS = {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
} as const;

/** The smallest amount cent precision can state. */
const CENT = "$0.01";

/** Nothing, stated exactly. Also the answer for `-0`. */
const ZERO = "$0.00";

/**
 * Whether cent precision would render `amount` as zero.
 *
 * Mirrors the rounding `Intl` applies rather than testing a hand-picked
 * threshold, so the bound and the rendered string can never disagree about
 * which side of a cent a value falls on. Decided from the magnitude, so a
 * debit and its matching credit land on the same side of the boundary.
 */
function roundsToZero(amount: number): boolean {
  return Math.round(Math.abs(amount) * 100) === 0;
}

/**
 * Renders a USD dollar float at cent precision.
 *
 * `—` for a value that is not a number, `$0.00` for exactly nothing, and a
 * bound (`<$0.01`, or `>-$0.01` below zero) for an amount that is not nothing
 * but is smaller than a cent.
 */
export function usd(amount: number): string {
  if (!Number.isFinite(amount)) return "—";
  if (amount === 0) return ZERO;
  if (roundsToZero(amount)) return amount < 0 ? `>-${CENT}` : `<${CENT}`;
  return amount.toLocaleString("en-US", CENT_DIGITS);
}

/**
 * Renders the size of an amount, leaving the sign to the caller.
 *
 * For the two places that pair their own `+`/`−` glyph with a figure — a
 * transaction row and the net tile — where `usd`'s own minus would double up.
 */
export function usdMagnitude(amount: number): string {
  return usd(Math.abs(amount));
}
