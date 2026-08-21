'use client';

// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState } from 'react';
import { ArrowLeft, ChevronLeft, ChevronRight } from 'lucide-react';
import type { ToolWiki } from './agent-wiki';
import { ToolDetailCard, type DeptLite } from './KnowledgeDetail';

/**
 * The chrome around the graph in its fullscreen (only) mode: the pillar
 * selector and side paddles for stepping through departments, the vault
 * search/legend slots, and a detail panel that overlays rather than resizes
 * the canvas — so opening or closing a card never reflows the graph. Owns
 * ←/→ and Escape; typing in the vault search suppresses them so the query
 * can use those keys.
 */
export function KnowledgeGraphFullscreen({
  deptList, currentTeamId, currentDept,
  toolWiki, extraDetail, coreOpen = false, onCollapseCore, searchSlot, legendSlot, statusSlot,
  onNavDept, onBack, children,
}: {
  deptList: DeptLite[];
  currentTeamId: string | null;
  currentDept: DeptLite | null;
  toolWiki: ToolWiki | null;
  /** task / human detail card rendered by the graph (SOP chain nodes) */
  extraDetail?: React.ReactNode;
  /** the Notes vault is expanded — Escape collapses it (via
      onCollapseCore) instead of exiting fullscreen; doing both at once
      stacked two heavy transitions and glitched the exit */
  coreOpen?: boolean;
  onCollapseCore?: () => void;
  /** vault search chip, rendered top-left while the vault is open */
  searchSlot?: React.ReactNode;
  /** compact kind legend, rendered bottom-left */
  legendSlot?: React.ReactNode;
  /** the snapshot line and its Refresh control, rendered top-right */
  statusSlot?: React.ReactNode;
  onNavDept: (teamId: string) => void;
  onBack: () => void;
  children: React.ReactNode;
}) {
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  const hasDetail = !!(toolWiki || extraDetail);
  /**
   * How the right-edge chrome gets out of the detail rail's way (issue #1307).
   *
   * The rail is an absolute overlay on purpose — resizing the canvas under it
   * reflowed the graph on every open and close, which is the glitch the
   * comment on the `<aside>` below is about. So the canvas keeps its size and
   * the two controls that live on the right edge move instead: `right-2` is
   * inside the 300px rail, and the rail is `z-30`, so without this they are
   * both covered *and* unclickable rather than merely obscured.
   *
   * Above 820px the rail is that 300px column, and the offset is its width
   * plus the inset the control already had. At or below it the rail is a
   * bottom sheet (`max-h-[62vh]`, anchored bottom) — the right edge is clear
   * again, so the offset is reverted and the paddles rise into the band the
   * sheet leaves instead of staying centred underneath it.
   */
  const clearOfRail = hasDetail ? 'right-[316px] max-[820px]:right-2' : 'right-2';
  /**
   * Mid-height normally; in the band above the bottom sheet when there is one.
   *
   * The sheet is `max-h-[62vh]` anchored to the bottom, so its top edge sits at
   * 38vh and a paddle centred at 50vh is underneath it. 19vh is the middle of
   * what is left. Only applies at or below 820px, because that is the only
   * width where the rail becomes a sheet.
   */
  const paddleTop = hasDetail ? 'top-1/2 max-[820px]:top-[19vh]' : 'top-1/2';
  const idx = deptList.findIndex((d) => d.teamId === currentTeamId);
  const step = (dir: number) => {
    if (deptList.length === 0) return;
    const next = idx < 0 ? (dir > 0 ? 0 : deptList.length - 1) : (idx + dir + deptList.length) % deptList.length;
    onNavDept(deptList[next].teamId);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // typing in the vault search (or any input) must not drive navigation
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement | null)?.isContentEditable) return;
      if (e.key === 'Escape') {
        if (hasDetail) onBack();
        else if (coreOpen) onCollapseCore?.(); // close the vault, stay fullscreen
        // Nothing left to close: the graph is the page.
      } else if (e.key === 'ArrowLeft') step(-1);
      else if (e.key === 'ArrowRight') step(1);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasDetail, coreOpen, idx, deptList]);

  if (!mounted) return null;

  return (
    // Fills its container rather than covering the window: the graph IS the
    // page, so it sits inside the console's chrome instead of over it.
    <div className="flex h-full min-h-0 w-full min-w-0 bg-os-bg">
      {/* the graph fills the field — same view as the inline "demo": every
          department in its spot in the circle, the active one bloomed into its
          tree with its colour glow, the rest dimmed in the background. */}
      <div className="relative min-w-0 flex-1 overflow-hidden bg-os-surface">
        {children}

        {/* vault search — top-left while the Notes core is open */}
        {searchSlot && <div className="absolute left-5 top-5 z-10">{searchSlot}</div>}

        {/* pillar selector — compact, TOP-LEFT: convenient, not in the
            graph's way. One named chip per desk (issue #1309).

            It used to be three 10px dots at 50% opacity under the words "Pick
            a pillar", and the names existed only in each dot's `title` — so
            the control that exists to choose a desk refused to say which desk
            was which, while the graph named all three in their own colours a
            few inches away. You had to click a blind dot to learn what it was.

            The chips wrap rather than scroll or truncate the row: a company
            with ten desks gets three short lines in the corner, which is a
            legible answer, where a clipped row is not. The colour is the same
            one the desk's node and label carry, so the chip and the pillar are
            visibly the same thing. */}
        {!coreOpen && (
          <div className="absolute left-5 top-5 z-20 flex max-w-[min(34rem,45vw)] flex-col gap-1 rounded-sm-t border border-os-border-strong bg-os-bg/85 px-2.5 py-1.5 backdrop-blur">
            <span className="font-mono text-3xs uppercase tracking-[0.14em] text-os-dim">
              {/* Names the group rather than instructing. "Pick a pillar" was
                  an imperative with no visible object, and at zero desks it
                  asked for something the page made impossible. */}
              {deptList.length > 0 ? 'Pillars' : 'No desks yet'}
            </span>
            {deptList.length > 0 && (
              <div className="flex flex-wrap items-center gap-x-1 gap-y-0.5">
                {deptList.map((d) => {
                  const active = d.teamId === currentTeamId;
                  return (
                    <button
                      key={d.teamId}
                      onClick={() => onNavDept(d.teamId)}
                      title={`${d.name} — bring this pillar forward`}
                      aria-current={active ? 'true' : undefined}
                      className={`flex items-center gap-1.5 rounded-sm-t px-1.5 py-0.5 text-2xs leading-tight transition-colors duration-200 ease-standard hover:bg-os-surface hover:text-os-text ${
                        active ? 'font-bold' : 'text-os-muted'
                      }`}
                      style={active ? { color: d.color } : undefined}
                    >
                      <span
                        aria-hidden
                        className={`h-2 w-2 shrink-0 rounded-full transition-all duration-200 ${
                          active ? '' : 'opacity-60'
                        }`}
                        style={{
                          background: d.color,
                          boxShadow: active ? `0 0 8px ${d.color}` : undefined,
                        }}
                      />
                      <span className="max-w-[12rem] truncate">{d.name}</span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* the snapshot line — top-right, clear of the detail rail (issue
            #1307). `z-40` so it stays above the rail even when the offset
            above puts it beside rather than behind it, and `top-5`/`right-5`
            so it sits on the same 20px inset as the pillar selector and the
            legend rather than the 12px one it used to carry alone. */}
        {statusSlot && (
          <div
            className={`absolute top-5 z-40 flex flex-col items-end gap-1.5 transition-[right] duration-200 ease-standard ${
              hasDetail ? 'right-[316px] max-[820px]:right-5' : 'right-5'
            }`}
          >
            {statusSlot}
          </div>
        )}

        {/* compact legend — bottom-left, always on */}
        {legendSlot && <div className="absolute bottom-5 left-5 z-10">{legendSlot}</div>}

        {/* side paddles: slim, hugging the canvas edges at mid-height — you
            turn the wheel from where you're already looking, never the top.
            The right paddle steps aside when the detail panel is open — see
            `clearOfRail`, which is what finally made that sentence true
            (issue #1307). */}
        {!coreOpen && (
          <>
            <button
              onClick={() => step(-1)}
              aria-label="Previous department"
              title="Previous pillar (←)"
              className={`absolute left-2 z-40 flex h-32 w-12 -translate-y-1/2 items-center justify-center rounded-sm-t border border-os-border bg-os-bg/70 text-os-muted backdrop-blur transition-all duration-200 ease-standard hover:border-os-border-strong hover:text-os-text ${paddleTop}`}
            >
              <ChevronLeft className="h-7 w-7" />
            </button>
            <button
              onClick={() => step(1)}
              aria-label="Next department"
              title="Next pillar (→)"
              className={`absolute z-40 flex h-32 w-12 -translate-y-1/2 items-center justify-center rounded-sm-t border border-os-border bg-os-bg/70 text-os-muted backdrop-blur transition-all duration-200 ease-standard hover:border-os-border-strong hover:text-os-text ${clearOfRail} ${paddleTop}`}
            >
              <ChevronRight className="h-7 w-7" />
            </button>
          </>
        )}
      </div>

      {/* detail panel — an absolute overlay so opening/closing a card never
          resizes the graph area (that reflow was the back-and-forth glitch) */}
      {hasDetail && (
        <aside className="absolute right-0 top-0 z-30 flex h-full w-[300px] flex-col border-l border-os-border-strong bg-os-bg/95 shadow-lg backdrop-blur max-[820px]:inset-x-0 max-[820px]:bottom-0 max-[820px]:top-auto max-[820px]:max-h-[62vh] max-[820px]:w-full max-[820px]:rounded-t-lg-t max-[820px]:border-l-0 max-[820px]:border-t">
          {/* the trail: node → pillar (this) → home. Same affordance inline. */}
          <button
            onClick={onBack}
            aria-label={`Back to the ${currentDept?.name ?? 'graph'} pillar`}
            className="flex shrink-0 items-center gap-1.5 border-b border-os-border px-3 py-2 text-left font-mono text-3xs uppercase tracking-[0.14em] text-os-dim transition-colors hover:text-os-text"
          >
            <ArrowLeft className="h-3 w-3 shrink-0" />
            <span className="truncate">
              Back · <span style={currentDept ? { color: currentDept.color } : undefined}>{currentDept?.name ?? 'graph'}</span>
            </span>
          </button>
          {toolWiki ? (
            <ToolDetailCard wiki={toolWiki} onClose={onBack} />
          ) : (
            extraDetail ?? null
          )}
        </aside>
      )}
    </div>
  );
}
