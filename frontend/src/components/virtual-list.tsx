import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { computeWindow, rowOffsets, totalHeight } from "@/components/virtual-window";

export interface VirtualListProps<T> {
  items: T[];
  /**
   * The scroll container the rows live inside. The list windows against this
   * element's scroll position rather than owning its own scrollbar, so a page
   * with header content above the list keeps a single scrollbar.
   */
  scrollElement: HTMLElement | null;
  renderItem: (item: T, index: number) => ReactNode;
  getKey: (item: T, index: number) => string;
  /** Columns per row. Rows stay CSS-grid aligned across lanes. Default 1. */
  lanes?: number;
  /** Row height assumed before a row has been measured. Default 120. */
  estimateRowHeight?: number;
  /** Rows rendered beyond the viewport on each side. Default 4. */
  overscan?: number;
  /** Gap in px between rows and lanes. Default 12. */
  gap?: number;
  className?: string;
  "data-testid"?: string;
}

export function VirtualList<T>({
  items,
  scrollElement,
  renderItem,
  getKey,
  lanes = 1,
  estimateRowHeight = 120,
  overscan = 4,
  gap = 12,
  className,
  "data-testid": testid,
}: VirtualListProps<T>) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const rowEls = useRef(new Map<string, HTMLDivElement>());
  const rowKeys = useRef(new WeakMap<HTMLDivElement, string>());
  const heights = useRef(new Map<string, number>());
  const [, bump] = useReducer((n: number) => n + 1, 0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(0);
  const [scrollMargin, setScrollMargin] = useState(0);
  const [resizeObserver] = useState<ResizeObserver | null>(() =>
    typeof ResizeObserver === "undefined"
      ? null
      : new ResizeObserver((entries) => {
          let changed = false;
          for (const entry of entries) {
            const key = rowKeys.current.get(entry.target as HTMLDivElement);
            if (!key) continue;
            const measured = entry.contentRect.height;
            if (measured > 0 && Math.abs((heights.current.get(key) ?? -1) - measured) > 1) {
              heights.current.set(key, measured);
              changed = true;
            }
          }
          if (changed) bump();
        }),
  );

  useEffect(() => () => resizeObserver?.disconnect(), [resizeObserver]);

  const laneCount = Math.max(1, lanes);
  const rowCount = Math.ceil(items.length / laneCount);

  const rowKeyAt = useCallback(
    (row: number): string => {
      const index = row * laneCount;
      const first = items[index];
      return first ? getKey(first, index) : `row-${row}`;
    },
    [items, laneCount, getKey],
  );

  const rowHeights = new Array<number>(rowCount);
  for (let r = 0; r < rowCount; r++) {
    rowHeights[r] = heights.current.get(rowKeyAt(r)) ?? estimateRowHeight;
  }
  const offsets = rowOffsets(rowHeights, gap);
  const height = totalHeight(offsets, gap);

  const win = computeWindow({
    scrollTop: scrollTop - scrollMargin,
    viewportHeight: viewport,
    offsets,
    count: rowCount,
    overscan,
  });

  const sync = useCallback(() => {
    const el = scrollElement;
    const wrap = wrapperRef.current;
    if (!el || !wrap) return;
    const nextTop = el.scrollTop;
    const nextView = el.clientHeight;
    const nextMargin =
      wrap.getBoundingClientRect().top - el.getBoundingClientRect().top + el.scrollTop;
    setScrollTop((p) => (p !== nextTop ? nextTop : p));
    setViewport((p) => (p !== nextView ? nextView : p));
    setScrollMargin((p) => (Math.abs(p - nextMargin) > 1 ? nextMargin : p));
  }, [scrollElement]);

  useEffect(() => {
    const el = scrollElement;
    if (!el) return;
    sync();
    el.addEventListener("scroll", sync, { passive: true });
    window.addEventListener("resize", sync);
    return () => {
      el.removeEventListener("scroll", sync);
      window.removeEventListener("resize", sync);
    };
  }, [scrollElement, sync]);

  useLayoutEffect(() => {
    sync();
    let changed = false;
    for (const [key, el] of rowEls.current) {
      const measured = el.getBoundingClientRect().height;
      if (measured > 0 && Math.abs((heights.current.get(key) ?? -1) - measured) > 1) {
        heights.current.set(key, measured);
        changed = true;
      }
    }
    if (changed) bump();
  });

  const setRowEl = useCallback(
    (key: string, el: HTMLDivElement | null) => {
      if (el) {
        rowEls.current.set(key, el);
        rowKeys.current.set(el, key);
        resizeObserver?.observe(el);
      } else {
        const prev = rowEls.current.get(key);
        if (prev) {
          resizeObserver?.unobserve(prev);
          rowKeys.current.delete(prev);
        }
        rowEls.current.delete(key);
      }
    },
    [resizeObserver],
  );

  const rows: ReactNode[] = [];
  for (let r = win.startIndex; r <= win.endIndex; r++) {
    const key = rowKeyAt(r);
    const cells: ReactNode[] = [];
    for (let l = 0; l < laneCount; l++) {
      const index = r * laneCount + l;
      if (index >= items.length) break;
      const item = items[index];
      cells.push(<Fragment key={getKey(item, index)}>{renderItem(item, index)}</Fragment>);
    }
    rows.push(
      <div
        key={key}
        ref={(el) => setRowEl(key, el)}
        style={{
          position: "absolute",
          top: offsets[r],
          left: 0,
          right: 0,
          display: "grid",
          gridTemplateColumns: `repeat(${laneCount}, minmax(0, 1fr))`,
          gap,
        }}
      >
        {cells}
      </div>,
    );
  }

  return (
    <div
      ref={wrapperRef}
      className={className}
      style={{ position: "relative", height }}
      data-testid={testid}
    >
      {rows}
    </div>
  );
}
