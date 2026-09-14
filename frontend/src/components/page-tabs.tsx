// The console's one page-level sub-tab strip, and the panel that answers it.
//
// # What this is for
//
// Some pages are one subject seen several ways rather than several pages. MCP
// is the case that prompted this: its rows and its `mcp.json` are the *same*
// configuration read through the same host routes, so splitting them into two
// sidebar destinations would put one source of truth behind two addresses —
// which is what issue #414 was. A tab keeps them one page while letting each
// have an address.
//
// The test for reaching for this rather than a sub-page: would a sidebar row
// for each be a lie about how many things the page is? Brain's Overview,
// Upload and Settings are three jobs that change at three different rates and
// each earned a row. MCP's two are one job in two notations.
//
// # Why the strip lives in the header
//
// The tabs sit at the bottom of `PageHeader`'s bar, flush with its hairline, so
// the active tab's underline lands *on* the rule that separates the page's name
// from the page. That is what makes them read as slices of this page rather
// than as a control the page happens to draw first — the shape OpenHuman's
// Connections page uses, and GitHub's and Linear's, for the same reason.
//
// They were in the body on MCP, under a permissions alert, which put a
// page-level navigation control below page-level content and left the header
// naming a page whose two halves it said nothing about.
//
// # Why not `components/ui/tabs.tsx`
//
// Base UI's `Tabs` needs its list and its panels inside one root. The strip
// renders into the header and the panels into the body — different subtrees,
// with the page's scroll container in between — so a single root would have to
// wrap the whole page and thread `value` back down anyway. This keeps the ARIA
// wiring (`role`, `aria-selected`, `aria-controls`) and drops the constraint.
//
// The state lives in the URL via `useHashTab`, so a tab is linkable and answers
// the Back button. The strip itself is controlled and knows nothing about that.

import type { ReactNode } from "react";

import type { LucideIcon } from "lucide-react";

import { cn } from "@/lib/utils";

/** One tab. `id` is what `?tab=` carries, so keep it URL-shaped. */
export type PageTab<T extends string> = {
  id: T;
  label: string;
  icon?: LucideIcon;
  /** Inline with the label, for a tab that is a list of things. */
  count?: number;
  /** `title` on the trigger, for a tab whose label cannot say enough. */
  hint?: string;
};

/**
 * The ids that tie a trigger to its panel.
 *
 * Exported because the two halves render in different subtrees and must agree
 * on the strings without either owning them — a panel that invents its own id
 * is a panel `aria-controls` points past.
 */
export function pageTabIds(base: string, id: string) {
  return { tab: `${base}-tab-${id}`, panel: `${base}-panel-${id}` };
}

/**
 * `-mb-3` cancels `PageHeader`'s own `pb-3` so the strip reaches the bar's
 * bottom edge; each trigger then carries that padding itself, which is what
 * puts the 2px underline on the hairline instead of 12px above it.
 *
 * `overflow-x-auto` with no scrollbar: a page with more tabs than fit must
 * still reach the last one, and nothing in the title bar may grow a visible
 * horizontal bar.
 */
const STRIP =
  "-mb-3 mt-2.5 flex items-stretch gap-4 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden";

const TRIGGER =
  "relative inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap border-b-2 pt-1 pb-3 " +
  "text-sm font-medium transition-colors focus-visible:ring-2 focus-visible:ring-ring " +
  "focus-visible:outline-none [&_svg]:size-4 [&_svg]:shrink-0";

export function PageTabs<T extends string>({
  tabs,
  value,
  onChange,
  idBase,
  className,
  "aria-label": ariaLabel,
}: {
  tabs: readonly PageTab<T>[];
  value: T;
  onChange: (next: T) => void;
  /** Namespaces the trigger/panel ids. One per page. */
  idBase: string;
  className?: string;
  /** Names the strip, since it has no visible label of its own. */
  "aria-label"?: string;
}) {
  return (
    <div role="tablist" aria-label={ariaLabel} className={cn(STRIP, className)}>
      {tabs.map((tab) => {
        const active = tab.id === value;
        const ids = pageTabIds(idBase, tab.id);
        const Icon = tab.icon;
        return (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={ids.tab}
            aria-selected={active}
            aria-controls={ids.panel}
            // The inactive triggers leave the tab order: a tablist is one stop,
            // and arrow keys move within it. Without this a five-tab page costs
            // five tabs to step past.
            tabIndex={active ? 0 : -1}
            title={tab.hint}
            data-testid={ids.tab}
            onClick={() => onChange(tab.id)}
            onKeyDown={(e) => {
              const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
              if (!step) return;
              e.preventDefault();
              const at = tabs.findIndex((t) => t.id === value);
              // Wraps, which is what a tablist does — and what stops the last
              // tab from being a dead end for a keyboard.
              const next = tabs[(at + step + tabs.length) % tabs.length];
              onChange(next.id);
              document.getElementById(pageTabIds(idBase, next.id).tab)?.focus();
            }}
            className={cn(
              TRIGGER,
              active
                ? "border-primary text-foreground"
                : "border-transparent text-muted-foreground hover:border-border hover:text-foreground",
            )}
          >
            {Icon && <Icon aria-hidden="true" />}
            {tab.label}
            {tab.count !== undefined && (
              <span className="rounded-full bg-muted px-1.5 py-0.5 text-xs text-muted-foreground tabular-nums">
                {tab.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The body half. Renders nothing unless it is the active tab — the panels are
 * whole sections of a page (MCP's rows are 1300 lines of them), so mounting the
 * inactive one to hide it would run its fetches for a surface nobody is looking
 * at.
 */
export function PageTabPanel({
  idBase,
  id,
  value,
  className,
  children,
}: {
  idBase: string;
  id: string;
  value: string;
  className?: string;
  children: ReactNode;
}) {
  if (id !== value) return null;
  const ids = pageTabIds(idBase, id);
  return (
    <div role="tabpanel" id={ids.panel} aria-labelledby={ids.tab} className={className}>
      {children}
    </div>
  );
}
