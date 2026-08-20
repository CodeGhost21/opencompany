// The pure half of the Work tab strip's responsive overflow (issue #1284):
// given how wide every tab actually measures and how much room there is,
// decide how many fit before the rest have to go behind "More ▾". Pulled out
// of the component so the decision is testable without a DOM, the same
// reasoning `filteredEmptyNotice` and `lib/ledger-wizard.ts` already use for
// their own "the decision is the whole thing worth getting right" logic.
//
// This exists because a hardcoded "show the first 4" was tried and rejected:
// it reads fine at whatever width someone happened to test on and wrong at
// every other one, and it is exactly as wrong whether a company holds 5
// lists or the 12-declared cap. Measuring is the only version of this that is
// correct at both a wide desktop window and a narrow one.

/**
 * How many of `widths` (in strip order) fit within `available` pixels, given
 * the "More" trigger itself costs `moreWidth` once anything has to sit behind
 * it.
 *
 * The common case — everything fits — never pays for the More trigger at
 * all: only once the full strip's width would exceed what is available does
 * the budget shrink to make room for it. Always keeps at least the first tab
 * (Tasks, by construction — see `WorkView`) visible even if it alone
 * overflows `available`, since a strip with nothing selected on screen is a
 * worse failure than one that overflows by a few pixels.
 */
export function chooseVisibleCount(
  widths: readonly number[],
  moreWidth: number,
  available: number,
): number {
  if (widths.length === 0) return 0;
  const total = widths.reduce((sum, w) => sum + w, 0);
  if (total <= available) return widths.length;

  const budget = available - moreWidth;
  let used = 0;
  let count = 0;
  for (const width of widths) {
    if (used + width > budget) break;
    used += width;
    count += 1;
  }
  return Math.max(count, 1);
}
