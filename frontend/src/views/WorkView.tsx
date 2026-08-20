// Work (issue #1284, Rule 2's final shape): the single nav row for every list
// a company holds. Tasks is the hero — first tab, selected by default, on
// screen with zero extra clicks — and every other list (`goals`, `decisions`,
// whatever the company declared) is a tab beside it, each carrying its own
// open count. Tabs that do not fit the strip's measured width collapse behind
// "More ▾" rather than a hardcoded count — see `lib/overflow-tabs.ts` for why
// and `hooks/use-overflow-tabs.ts` for how.
//
// This wraps `LedgersView` rather than replacing it: the tab strip decides
// which list is selected, `LedgersView` still renders that one list's rows
// exactly as it always has (search, board/list toggle, compose, delete —
// none of that changed). See `docs/spec/runtime/ledgers-console-ia.md`'s
// Rule 2 for the two shapes this superseded and why.

import { useMemo } from "react";
import { ChevronDown } from "lucide-react";

import type { OpenCompanyClient } from "@/api/client";
import type { ApprovalSummary } from "@/api/types";
import type { LedgerSummary } from "@/api/ledgers";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useOverflowTabs } from "@/hooks/use-overflow-tabs";
import { BOARD_LEDGER } from "@/lib/board-columns";
import { cn } from "@/lib/utils";
import { LedgersView } from "@/views/LedgersView";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  ledgers: LedgerSummary[];
  ledgersLoading: boolean;
  /** The list named in `#/ledgers/<slug>`, or `null`/`undefined` for the bare
   * `#/ledgers` address — which resolves to the Tasks tab (Rule 2). */
  sub?: string | null;
  onOpenLedger?: (slug: string | null) => void;
  onOpenCard?: (id: string) => void;
  taskEventTick?: number;
  approvals?: readonly ApprovalSummary[];
  now?: number;
  onReviewApprovals?: (taskId: string) => void;
}

function tabLabel(held: LedgerSummary): string {
  return held.open > 0 ? `${held.title} ${held.open}` : held.title;
}

export function WorkView({
  client,
  company,
  ledgers,
  ledgersLoading,
  sub,
  onOpenLedger,
  onOpenCard,
  taskEventTick,
  approvals,
  now,
  onReviewApprovals,
}: Props) {
  /** Tasks first, always — the hero tab, then every other list in the order
   * the host declared them. */
  const ordered = useMemo(() => {
    const board = ledgers.find((held) => held.slug === BOARD_LEDGER);
    const rest = ledgers.filter((held) => held.slug !== BOARD_LEDGER);
    return board ? [board, ...rest] : rest;
  }, [ledgers]);

  /** A bare `#/ledgers` resolves to Tasks — the hero, on screen with zero
   * extra clicks (Rule 2). A named slug resolves to that tab even if the
   * list read has not landed yet, so the strip does not flicker Tasks-then-
   * elsewhere on a direct link to Goals. */
  const activeSlug = sub ?? BOARD_LEDGER;

  const { containerRef, measureRef, moreRef, visibleCount } = useOverflowTabs(
    ordered.map((held) => held.slug),
  );
  const visible = ordered.slice(0, visibleCount);
  const overflow = ordered.slice(visibleCount);

  const openLedger = (slug: string) => {
    if (slug !== activeSlug) onOpenLedger?.(slug);
  };

  const tabClass = (active: boolean) =>
    cn(
      "shrink-0 rounded-md px-3 py-1.5 text-sm font-medium transition-colors whitespace-nowrap",
      active
        ? "bg-accent text-accent-foreground"
        : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
    );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {ordered.length > 0 && (
        <div
          ref={containerRef}
          role="tablist"
          aria-label="Work"
          data-testid="work-tab-strip"
          className="relative flex min-w-0 items-center gap-1.5 overflow-hidden border-b px-4 py-2"
        >
          {visible.map((held) => (
            <button
              key={held.slug}
              type="button"
              role="tab"
              aria-selected={held.slug === activeSlug}
              data-testid={`work-tab-${held.slug}`}
              onClick={() => openLedger(held.slug)}
              className={tabClass(held.slug === activeSlug)}
            >
              {tabLabel(held)}
            </button>
          ))}
          {overflow.length > 0 && (
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <button
                    type="button"
                    data-testid="work-tab-more"
                    className={tabClass(overflow.some((held) => held.slug === activeSlug))}
                  >
                    More
                    <ChevronDown className="ml-1 inline size-3.5" />
                  </button>
                }
              />
              <DropdownMenuContent align="start">
                {overflow.map((held) => (
                  <DropdownMenuItem
                    key={held.slug}
                    data-testid={`work-tab-more-${held.slug}`}
                    onClick={() => openLedger(held.slug)}
                  >
                    {tabLabel(held)}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          )}

          {/* A hidden measuring row: same content and classes as the real
              tabs, so each child's natural width is what a real tab would
              take, whether or not it is currently rendered visibly. */}
          <div
            ref={measureRef}
            aria-hidden
            className="pointer-events-none absolute -left-[9999px] top-0 flex items-center gap-1.5"
          >
            {ordered.map((held) => (
              <span key={held.slug} className={tabClass(false)}>
                {tabLabel(held)}
              </span>
            ))}
          </div>
          <div
            ref={moreRef}
            aria-hidden
            className="pointer-events-none absolute -left-[9999px] top-0"
          >
            <span className={tabClass(false)}>
              More
              <ChevronDown className="ml-1 inline size-3.5" />
            </span>
          </div>
        </div>
      )}

      <div className="min-h-0 flex-1">
        <LedgersView
          client={client}
          company={company}
          ledgers={ledgers}
          ledgersLoading={ledgersLoading}
          sub={activeSlug}
          onOpenLedger={onOpenLedger}
          onOpenCard={onOpenCard}
          taskEventTick={taskEventTick}
          approvals={approvals}
          now={now}
          onReviewApprovals={onReviewApprovals}
        />
      </div>
    </div>
  );
}
