import { lazy, Suspense } from "react";

import type { OpenCompanyClient } from "@/api/client";
import { RouteLoading } from "@/components/route-loading";
import { resolveFinancePage } from "@/views/finance/finance-pages";
import { InvoicingView } from "@/views/finance/InvoicingView";
import { WalletView } from "@/views/finance/WalletView";

// Recharts-backed and only used here — load the ledger overview on demand, as
// it was loaded when it hung off the shell directly.
const FinancesView = lazy(() =>
  import("@/views/FinancesView").then((m) => ({ default: m.FinancesView })),
);

// The table itself lives in `finance-pages.ts` so that anything pointing at a
// sub-page — the console's nav table, a route rewrite — can name one without
// importing this section and the three views under it. Re-exported here so the
// imports written while it lived in this file keep resolving.
export {
  DEFAULT_FINANCE_PAGE,
  FINANCE_PAGES,
  financeHref,
  isFinancePage,
  resolveFinancePage,
  type FinancePage,
} from "@/views/finance/finance-pages";

interface Props {
  client: OpenCompanyClient;
  company: string | null;
  /** The hash's second segment, e.g. `wallet` in `#/finances/wallet`. */
  sub: string | null;
}

/**
 * Finance, as a section rather than a page.
 *
 * Each sub-page is its own route (`#/finances/wallet`), so it is linkable and
 * survives a refresh exactly as a top-level view does.
 *
 * # Why this replaced Settings → Billing
 *
 * Chargebee and PayPal were configured at `#/settings/billing`, which is the
 * right home for a credential and the wrong home for everything else about
 * money: a settings tab is a place an operator visits once, and invoices and a
 * balance are read repeatedly. "Billing" was also ambiguous in a product that is
 * itself billed — an operator reading it reasonably expects *what OpenCompany
 * charges me*, which is Settings → Usage. The credential forms now sit in a
 * collapsible panel at the top of the page whose data they unlock.
 *
 * # Where the rail went
 *
 * This section drew a `w-60` rail of its own, and Settings copied it. Finance is
 * a row *under Company* (`NAV_SECTIONS`), and Company draws a section rail of
 * its own now (`components/section-rail.tsx`, issue #2130) — so a rail here
 * would be the second one in the same viewport, which is exactly the 768–1023px
 * two-rail band issue #1383 was filed about and `SettingsSection.tsx` still
 * carries a comment about. One rail per section: Finance's three pages are
 * nested rows on Company's rail, shown while Finance is the open row.
 *
 * What is left is the dispatch, which is all this component does besides draw
 * the rail — the same shape `connections/ConnectionsSection.tsx` has, and for
 * the same reason.
 *
 * # The `key` on each provider page is load-bearing
 *
 * Both hold typed-but-unsaved credentials. `key={company}` remounts on a company
 * switch rather than re-running against carried-over state — without it, an
 * operator who typed a Chargebee key for one company, switched, and pressed Save
 * writes that credential into the other company's secret store. Clearing fields
 * by hand covers the ones somebody remembered; a remount covers all of them, and
 * also makes `company` constant for the instance's lifetime, so a slow response
 * from a previous company cannot land on a later one's view.
 */
export function FinanceSection({ client, company, sub }: Props) {
  const page = resolveFinancePage(sub);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {page === "overview" && (
        <Suspense fallback={<RouteLoading title="Finances" label="Loading finances…" />}>
          <FinancesView client={client} company={company} />
        </Suspense>
      )}
      {page === "invoicing" && (
        <InvoicingView key={company ?? "self"} client={client} company={company} />
      )}
      {page === "wallet" && (
        <WalletView key={company ?? "self"} client={client} company={company} />
      )}
    </div>
  );
}
