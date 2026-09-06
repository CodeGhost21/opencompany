// The Connections section's sub-page table, and the helpers that read it.
//
// A leaf module, exactly as `settings-pages.ts` is, and for the same reason:
// anything *pointing at* a sub-page — prose, a route rewrite — has to name one
// without importing the section, which imports every view under it. The route
// rewrites in `lib/console-route-rewrites.ts` are the case that forces it here:
// they run on the router's own path, so a static import of the section from
// there would pull `OAuthView` and `McpServersView` in behind them.
//
// Modelled on `finance/FinanceSection.tsx`'s `FINANCE_PAGES`, which keeps its
// table inside the section because nothing outside needs to read it. This one
// is read from two other modules, so it lives on its own.

import { Blocks, LayoutGrid, type LucideIcon } from "lucide-react";

/**
 * The sub-pages that live under Connections. The id is the hash's second
 * segment.
 *
 * Two, not five. A single "Connections" page once carried third-party
 * accounts, MCP servers, inference, channels and repositories, and was
 * deliberately broken apart because each was something an operator scrolled
 * past on the way to another (see the comment above the `oauth` entry in
 * `settings-pages.ts`, and `OAuthView`'s own header). That decision was about
 * one-question-per-page, and it stands: these are still two pages. What is new
 * is that they have a parent, which is a different thing from being merged
 * back together.
 *
 * Inference, Hosting and Search deliberately did **not** move here. Each is a
 * credential form that belongs beside the one thing it unlocks — the model, the
 * deploy target, the search provider — which is the argument `settings-pages.ts`
 * makes twice, and filing them under a section named for the act of connecting
 * would undo it.
 */
export const CONNECTION_PAGES = [
  {
    id: "apps",
    label: "Apps",
    icon: LayoutGrid,
    hint: "The apps your teammates act through",
  },
  {
    id: "mcp",
    label: "MCP Servers",
    icon: Blocks,
    hint: "Tool servers and their tools",
  },
] as const satisfies readonly { id: string; label: string; icon: LucideIcon; hint: string }[];

export type ConnectionPage = (typeof CONNECTION_PAGES)[number]["id"];

export const DEFAULT_CONNECTION_PAGE: ConnectionPage = "apps";

/** Whether a hash segment names a real sub-page. */
export function isConnectionPage(sub: string | null): sub is ConnectionPage {
  return CONNECTION_PAGES.some((page) => page.id === sub);
}

/** The sub-page a hash segment resolves to, defaulting to Apps. */
export function resolveConnectionPage(sub: string | null): ConnectionPage {
  return isConnectionPage(sub) ? sub : DEFAULT_CONNECTION_PAGE;
}

/**
 * The console hash a link to one Connections sub-page needs.
 *
 * Typed for the same reason `settingsHref` is: a link written against this
 * cannot outlive the page it points at. `#/settings/connections` — hard-coded
 * in four places, pointing at a page that stopped existing when Connections was
 * split — is what happens without it.
 */
export function connectionsHref(page: ConnectionPage): string {
  return `#/connections/${page}`;
}
