export interface RowWindow {
  startIndex: number;
  endIndex: number;
}

export function rowOffsets(rowHeights: number[], gap = 0): number[] {
  const offsets = new Array<number>(rowHeights.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < rowHeights.length; i++) {
    offsets[i + 1] = offsets[i] + rowHeights[i] + gap;
  }
  return offsets;
}

export function totalHeight(offsets: number[], gap = 0): number {
  const count = offsets.length - 1;
  return count > 0 ? offsets[count] - gap : 0;
}

function rowAtOffset(offsets: number[], value: number, count: number): number {
  let lo = 0;
  let hi = count - 1;
  let ans = 0;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid] <= value) {
      ans = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return ans;
}

/**
 * The `[startIndex, endIndex]` rows that intersect the viewport
 * `[scrollTop, scrollTop + viewportHeight]`, padded by `overscan` rows on each
 * side and clamped to the population. Returns an empty window
 * (`endIndex < startIndex`) only when `count` is 0.
 */
export function computeWindow(params: {
  scrollTop: number;
  viewportHeight: number;
  offsets: number[];
  count: number;
  overscan: number;
}): RowWindow {
  const { scrollTop, viewportHeight, offsets, count, overscan } = params;
  if (count <= 0) return { startIndex: 0, endIndex: -1 };
  const top = Math.max(0, scrollTop);
  const bottom = top + Math.max(0, viewportHeight);
  const first = rowAtOffset(offsets, top, count);
  const last = rowAtOffset(offsets, bottom, count);
  return {
    startIndex: Math.max(0, first - overscan),
    endIndex: Math.min(count - 1, last + overscan),
  };
}
