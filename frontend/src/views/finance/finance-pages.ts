// The Finance section's sub-page table, and the helpers that read it.
//
// A leaf module, exactly as `views/connection-pages.ts` and
// `views/settings-pages.ts` are, and for the same reason: anything *pointing
// at* a sub-page has to name one without importing the section, which imports
// every view under it.
//
// The case that forces it here is the console's sub-navigation. Finance's three
// pages are rows on the Company section's content rail now
// (`components/section-rail.tsx`, driven by `NAV_SECTIONS` in
// `components/sidebar-navigation.tsx`), so the nav table reads this list — and
// a static import of `FinanceSection` from the nav table would pull
// `InvoicingView`, `WalletView` and the lazy `FinancesView` in behind it, into
// a module the sidebar renders on every route.
//
// `FinanceSection.tsx` re-exports everything below, so the imports written
// while the table lived there keep resolving.

import { CreditCard, LayoutDashboard, Wallet, type LucideIcon } from "lucide-react";

/**
 * The sub-pages under Finance. The id is the hash's second segment.
 *
 * Three, not two. Overview is the ledger projection (`GET …/finances`, folded by
 * `metering::finances_from` from the company's own ledger and its manifest
 * `[budget]`) — the company's internal accounting, which owes nothing to either
 * provider. It leads because it is the one page that has something to show on a
 * host where nothing is connected yet, so the section is never an empty shell.
 */
export const FINANCE_PAGES = [
  {
    id: "overview",
    label: "Overview",
    icon: LayoutDashboard,
    hint: "Balance, budget and spend from the ledger",
  },
  {
    id: "invoicing",
    label: "Invoicing",
    icon: CreditCard,
    hint: "What customers owe, through Chargebee",
  },
  {
    id: "wallet",
    label: "Wallet",
    icon: Wallet,
    hint: "The PayPal balance and what moved through it",
  },
] as const satisfies readonly { id: string; label: string; icon: LucideIcon; hint: string }[];

export type FinancePage = (typeof FINANCE_PAGES)[number]["id"];

export const DEFAULT_FINANCE_PAGE: FinancePage = "overview";

/** Whether a hash segment names a real sub-page. */
export function isFinancePage(sub: string | null): sub is FinancePage {
  return FINANCE_PAGES.some((page) => page.id === sub);
}

/** The sub-page a hash segment resolves to, defaulting to Overview. */
export function resolveFinancePage(sub: string | null): FinancePage {
  return isFinancePage(sub) ? sub : DEFAULT_FINANCE_PAGE;
}

/**
 * The console hash a link to one Finance sub-page needs.
 *
 * Typed for the same reason `settingsHref` and `connectionsHref` are: a link
 * written against this cannot outlive the page it points at.
 */
export function financeHref(page: FinancePage): string {
  return `#/finances/${page}`;
}
