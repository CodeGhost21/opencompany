// Turning the host's Composio status into the provider rows the console renders
// (issue #397).
//
// The host now answers open mode with the backend's live catalog — roughly a
// hundred toolkits — rather than a hardcoded eight. A hundred rows is its own
// usability problem, and dumping them unsorted and unfiltered would trade one
// broken page for another. These helpers own the two decisions that fixes:
//
//  1. **Order**, so the answer to "where is Gmail" is "at the top", not "row
//     thirty-one". Connected providers first (an operator scanning this panel is
//     usually checking what is live), then the handful everyone reaches for,
//     then the long tail alphabetically.
//  2. **Reach**, so the tail is discoverable. The list collapses to a preview
//     with an explicit "show all N" and a search box that matches on both slug
//     and display name. Before this, reaching a provider outside the eight meant
//     knowing its Composio slug and typing it into a free-text field — which is
//     a fine escape hatch and a terrible discovery mechanism.
//
// Pure functions on purpose: `vitest.config.ts` scopes the unit runner to
// helpers with no document and no host, and this is exactly that. What the
// component does with the rows belongs in `test/e2e`.

import type { ComposioStatus } from "@/api/composio";

/** Friendly display labels for the common toolkits; slug-cased fallback otherwise. */
const TOOLKIT_LABELS: Record<string, string> = {
  gmail: "Gmail",
  slack: "Slack",
  github: "GitHub",
  googlecalendar: "Google Calendar",
  googledrive: "Google Drive",
  googlesheets: "Google Sheets",
  googledocs: "Google Docs",
  notion: "Notion",
  linear: "Linear",
  discord: "Discord",
  hubspot: "HubSpot",
  jira: "Jira",
  asana: "Asana",
  trello: "Trello",
  zendesk: "Zendesk",
  salesforce: "Salesforce",
  dropbox: "Dropbox",
  airtable: "Airtable",
  calendly: "Calendly",
  stripe: "Stripe",
  shopify: "Shopify",
  twitter: "X (Twitter)",
  linkedin: "LinkedIn",
  youtube: "YouTube",
  outlook: "Outlook",
  onedrive: "OneDrive",
  sharepoint: "SharePoint",
  supabase: "Supabase",
  gitlab: "GitLab",
  bitbucket: "Bitbucket",
};

/**
 * A display label for a toolkit slug.
 *
 * The table is a nicety, not a catalog — it exists so `googlecalendar` reads as
 * "Google Calendar" rather than "Googlecalendar". A slug missing from it is
 * title-cased and still renders; nothing here can hide a provider the host sent.
 * That is the distinction #397 turns on: the *list* comes from the backend, and
 * only its *typography* is local.
 */
export function toolkitLabel(slug: string): string {
  const key = slug.trim().toLowerCase();
  return TOOLKIT_LABELS[key] ?? (key ? key.charAt(0).toUpperCase() + key.slice(1) : slug);
}

/**
 * The providers a company reaches for first, in the order they are surfaced.
 *
 * Purely an ordering hint for the rendered list — it never adds a row, never
 * removes one, and is not what the host offers. A curated slug absent from the
 * host's list is simply absent.
 */
export const CURATED_TOOLKITS: readonly string[] = [
  "gmail",
  "googlecalendar",
  "googledrive",
  "github",
  "slack",
  "notion",
  "linear",
  "discord",
];

/** How many rows show before the operator asks for the rest. */
export const PREVIEW_COUNT = 12;

/** One provider row as the section renders it. */
export interface ProviderRow {
  /** The Composio toolkit slug — the key every host call is made with. */
  slug: string;
  /** What the operator reads. */
  label: string;
  /** Whether the company has at least one active connection for it. */
  connected: boolean;
  /** Whether it is one of the {@link CURATED_TOOLKITS}. */
  curated: boolean;
}

/**
 * Build the ordered provider rows.
 *
 * `effective` is the host's answer (manifest list, backend catalog, or flagged
 * fallback — this function does not care which, and must not: re-deciding here
 * what the host already decided is how the two drifted apart in the first
 * place). `extra` carries slugs the operator connected through the free-text
 * field this session, so they keep a row instead of vanishing. `connected` maps
 * lowercased slug to connected state.
 *
 * Order: connected, then curated, then the rest alphabetically. Duplicates
 * collapse; blank slugs are dropped.
 */
export function buildProviderRows(
  effective: readonly string[],
  extra: readonly string[],
  connected: Readonly<Record<string, boolean>>,
): ProviderRow[] {
  const seen = new Set<string>();
  const rows: ProviderRow[] = [];
  for (const raw of [...effective, ...extra]) {
    const slug = raw.trim().toLowerCase();
    if (!slug || seen.has(slug)) continue;
    seen.add(slug);
    rows.push({
      slug,
      label: toolkitLabel(slug),
      connected: connected[slug] === true,
      curated: CURATED_TOOLKITS.includes(slug),
    });
  }
  const rank = (row: ProviderRow) => (row.connected ? 0 : row.curated ? 1 : 2);
  return rows.sort((a, b) => {
    if (rank(a) !== rank(b)) return rank(a) - rank(b);
    if (rank(a) === 1) return CURATED_TOOLKITS.indexOf(a.slug) - CURATED_TOOLKITS.indexOf(b.slug);
    return a.label.localeCompare(b.label);
  });
}

/**
 * Narrow rows to a search query, matched case-insensitively against both the
 * slug and the display label.
 *
 * Both halves matter: an operator who knows the product types "google calendar"
 * and an operator who knows Composio types `googlecalendar`, and neither should
 * come up empty.
 */
export function filterProviderRows(rows: readonly ProviderRow[], query: string): ProviderRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...rows];
  return rows.filter((row) => row.slug.includes(q) || row.label.toLowerCase().includes(q));
}

/**
 * The rows to actually render, and whether anything is being held back.
 *
 * A search shows every match — someone who typed a query wants the answer, not
 * the first twelve answers. Without one, a long list collapses to
 * {@link PREVIEW_COUNT} unless the operator has expanded it.
 */
export function visibleProviderRows(
  rows: readonly ProviderRow[],
  query: string,
  expanded: boolean,
): { visible: ProviderRow[]; hidden: number } {
  const matched = filterProviderRows(rows, query);
  if (query.trim() || expanded || matched.length <= PREVIEW_COUNT) {
    return { visible: matched, hidden: 0 };
  }
  return { visible: matched.slice(0, PREVIEW_COUNT), hidden: matched.length - PREVIEW_COUNT };
}

/**
 * The warning to show the operator when the list they are looking at is not the
 * backend's real catalog, or `null` when it is trustworthy.
 *
 * This is the console half of the host's honesty contract. The host marks a
 * fallback as `catalogSource: "fallback"` and says why; if the console then
 * rendered it identically to a fetched catalog, the marking would have bought
 * nothing — the operator would still be looking at eight providers with no way
 * to know the other ninety exist.
 *
 * `manifest` is not a degradation: a company that narrowed its own allowlist is
 * seeing exactly what it chose, and telling it the list "may be incomplete"
 * would be false.
 */
export function catalogWarning(status: Pick<ComposioStatus, "catalogSource" | "catalogNotice">): string | null {
  if (status.catalogSource !== "fallback") return null;
  return (
    status.catalogNotice ??
    "Composio's provider catalog could not be fetched, so this is a built-in starter list and may be incomplete."
  );
}
