// Turning the host's Composio status into the provider tiles the console renders
// (issues #397, #556, #600).
//
// ## What changed, and why the shape of this file changed with it
//
// #397 taught the host to serve the backend's live catalog instead of a
// hardcoded eight, and #556 un-capped it — so this surface went from 8 providers
// to 123. It kept rendering them as one flat vertical list, twelve at a time
// behind a "show all" button, because a flat list was all the data allowed: the
// host reduced every catalog entry to a bare slug, and there is nothing to group
// 123 slugs by.
//
// #600 widened that wire (`effectiveCatalog`), so these helpers now own four
// decisions instead of two:
//
//  1. **Category**, so 123 providers are eight browsable buckets. Derived from
//     Composio's OWN `categories[]` strings by substring, never from a list
//     maintained here — which is the whole point: a Composio integration added
//     tomorrow lands in the right bucket with no code change. A slug-only entry
//     (manifest list, degraded fallback, backend predating the dynamic catalog)
//     falls through to a slug/name keyword heuristic rather than vanishing.
//  2. **Order**, so the answer to "where is Gmail" is "at the top", not "row
//     thirty-one". Connected first — an operator scanning this panel is usually
//     checking what is live — then the handful everyone reaches for, then the
//     tail alphabetically.
//  3. **Reach**, so the tail is discoverable: search over slug, display name AND
//     description, composed with the category filter rather than replacing it.
//     Someone who does not know a provider's name can still find it by what it
//     does.
//  4. **Typography and branding**, so a tile reads as a product rather than a
//     slug. The backend's name/description/logo win wherever it published them;
//     the local tables below fill the gaps.
//
// Pure functions on purpose: `vitest.config.ts` scopes the unit runner to
// helpers with no document and no host, and this is exactly that. What the
// component does with the rows — the grid, the chips, the images — belongs in
// `test/e2e`.

import type { ComposioStatus, ComposioToolkitEntry } from "@/api/composio";

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
 * Title-case a slug the label table does not cover: `capsule_crm` → `Capsule
 * Crm`, `digital-ocean` → `Digital Ocean`.
 *
 * Splitting on `_` and `-` matters more than it looks. Composio slugs are
 * routinely compound, and the previous rendering — upper-case the first letter
 * and stop — turned `capsule_crm` into `Capsule_crm`, which reads as a database
 * column rather than a product.
 */
function prettifySlug(slug: string): string {
  return slug
    .split(/[_-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

/**
 * A display label for a toolkit slug, with no catalog entry to hand.
 *
 * The table is a nicety, not a catalog — it exists so `googlecalendar` reads as
 * "Google Calendar" rather than "Googlecalendar". A slug missing from it is
 * title-cased and still renders; nothing here can hide a provider the host sent.
 * That is the distinction #397 turns on: the *list* comes from the backend, and
 * only its *typography* is local.
 *
 * Kept as a slug-only entry point because the sign-in-by-slug path has nothing
 * else — the operator typed a slug the catalog does not carry. Everywhere a
 * catalog entry exists, {@link providerLabel} prefers the backend's own name.
 */
export function toolkitLabel(slug: string): string {
  const key = slug.trim().toLowerCase();
  return TOOLKIT_LABELS[key] ?? (key ? prettifySlug(key) : slug);
}

/**
 * The display label for a catalog entry: the backend's published name when it
 * has one, else local typography.
 *
 * The backend wins deliberately. It is describing the provider it will actually
 * connect you to, and it learns about a rename before this file does.
 */
export function providerLabel(entry: ComposioToolkitEntry): string {
  return entry.name.trim() || toolkitLabel(entry.slug);
}

/**
 * Composio's own logo CDN, keyed by slug.
 *
 * Used only when the entry published no `logo` — which is every slug-only
 * entry, i.e. every manifest list and every degraded fallback. The URL is a
 * guess by construction, so the tile that renders it must tolerate a 404 and
 * fall back to a monogram rather than showing a broken image.
 */
export function composioLogoUrl(slug: string): string {
  return `https://logos.composio.dev/api/${slug}`;
}

/** The buckets the category chips offer, in the order they are shown. */
export type ProviderCategory =
  | "All"
  | "Chat"
  | "Productivity"
  | "Platform"
  | "Social"
  | "Tools & Automation";

/**
 * Chip order. Fixed rather than derived so the chips do not reshuffle when a
 * company connects a provider or the backend adds one — a filter row that moves
 * under the cursor is worse than a filter row in an imperfect order.
 */
export const PROVIDER_CATEGORY_ORDER: readonly ProviderCategory[] = [
  "All",
  "Chat",
  "Productivity",
  "Platform",
  "Social",
  "Tools & Automation",
];

/**
 * Map Composio's catalog category strings onto the fixed buckets above.
 *
 * Composio's category names are free-form (`"productivity"`, `"crm"`,
 * `"developer-tools"`, `"project management"`), so this matches on substrings
 * and returns the first hit. `null` when nothing matches, so the caller can fall
 * through to the slug/name heuristic.
 *
 * **This is the piece that makes the feature maintainable**, and it is why the
 * host forwards `categories[]` verbatim instead of bucketing server-side: 123
 * providers bucket themselves, and provider number 124 does too, with no edit
 * here. A hand-maintained slug→category table would be stale the week it
 * shipped — which is precisely the trap `TOOLKIT_LABELS` above is allowed to
 * fall into only because its failure mode is cosmetic.
 *
 * Ported from OpenHuman's `mapComposioCategory` (`toolkitMeta.tsx`), which
 * drives the same catalog from the same strings. Keeping the two in step is
 * cheaper than diverging.
 */
export function mapComposioCategory(categories: readonly string[]): ProviderCategory | null {
  if (categories.length === 0) return null;
  const haystack = categories.join(" ").toLowerCase();
  const has = (...needles: string[]) => needles.some((n) => haystack.includes(n));

  if (has("chat", "messaging", "communication")) return "Chat";
  if (has("social", "marketing")) return "Social";
  if (
    has(
      "productivity",
      "document",
      "calendar",
      "scheduling",
      "project management",
      "project-management",
      "note",
      "task",
      "storage",
      "email",
    )
  ) {
    return "Productivity";
  }
  if (has("crm", "developer", "devtool", "analytics", "payment", "finance", "database", "cloud")) {
    return "Platform";
  }
  return null;
}

const CHAT_KEYWORDS = ["discord", "slack", "teams", "webex", "whatsapp", "dialpad", "lark", "feishu"];
const SOCIAL_KEYWORDS = ["facebook", "instagram", "linkedin", "reddit", "youtube", "twitter", "x_"];
const PRODUCTIVITY_KEYWORDS = [
  "gmail",
  "calendar",
  "drive",
  "docs",
  "doc",
  "sheets",
  "slides",
  "tasks",
  "todoist",
  "trello",
  "notion",
  "box",
  "dropbox",
  "sharepoint",
  "one_drive",
  "onedrive",
  "outlook",
  "miro",
  "mural",
  "monday",
  "clickup",
  "linear",
  "jira",
  "confluence",
  "asana",
  "basecamp",
  "wrike",
  "cal",
  "calendly",
  "typeform",
  "excel",
  "figma",
  "google",
];
const PLATFORM_KEYWORDS = [
  "github",
  "gitlab",
  "bitbucket",
  "digital_ocean",
  "contentful",
  "supabase",
  "convex",
  "prisma",
  "sentry",
  "stripe",
  "salesforce",
  "hubspot",
  "quickbooks",
  "zendesk",
  "zoho",
];

/**
 * Last-resort bucketing from the slug and name alone.
 *
 * Only reached when the backend published no categories — a manifest allowlist,
 * a degraded fallback, or a backend predating the dynamic catalog. Those lists
 * are short and weighted towards exactly these providers, so a keyword pass
 * covers most of them; anything it misses lands in "Tools & Automation", which
 * is a real bucket rather than a hole.
 */
function guessCategory(slug: string, name: string): ProviderCategory {
  const key = `${slug} ${name}`.toLowerCase();
  if (CHAT_KEYWORDS.some((k) => key.includes(k))) return "Chat";
  if (SOCIAL_KEYWORDS.some((k) => key.includes(k))) return "Social";
  if (PRODUCTIVITY_KEYWORDS.some((k) => key.includes(k))) return "Productivity";
  if (PLATFORM_KEYWORDS.some((k) => key.includes(k))) return "Platform";
  return "Tools & Automation";
}

/**
 * A short "what you are authorising" hint, derived from the bucket.
 *
 * Deliberately vague, and honest about it: Composio decides the real scopes at
 * consent time and does not publish them here. A per-provider scope list would
 * be a claim this console cannot back, so this says the shape of the access
 * rather than pretending to enumerate it.
 */
export function permissionHint(category: ProviderCategory): string {
  switch (category) {
    case "Chat":
      return "Messages, channels, and communication data";
    case "Social":
      return "Posts, profiles, and social content";
    case "Productivity":
      return "Docs, files, tasks, and workspace data";
    case "Platform":
      return "Repos, records, tickets, and system data";
    default:
      return "Connected account data";
  }
}

/**
 * The providers a company reaches for first, in the order they are surfaced.
 *
 * Purely an ordering hint for the rendered tiles — it never adds a tile, never
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

/** One provider tile as the section renders it. */
export interface ProviderRow {
  /** The Composio toolkit slug — the key every host call is made with. */
  slug: string;
  /** What the operator reads. The backend's name when it published one. */
  label: string;
  /** One line about what the provider is for. Empty when nothing is known. */
  description: string;
  /** Logo to render, published or derived. May 404 — the tile must cope. */
  logoUrl: string;
  /** Which chip this tile lives under. */
  category: ProviderCategory;
  /** Whether the company has at least one active connection for it. */
  connected: boolean;
  /** Whether it is one of the {@link CURATED_TOOLKITS}. */
  curated: boolean;
}

/**
 * Build the ordered provider tiles.
 *
 * `catalog` is the host's answer (manifest list, backend catalog, or flagged
 * fallback — this function does not care which, and must not: re-deciding here
 * what the host already decided is how the two drifted apart in the first
 * place). `extra` carries slugs the operator connected through the free-text
 * field this session, so they keep a tile instead of vanishing. `connected` maps
 * lowercased slug to connected state.
 *
 * Order: connected, then curated, then the rest alphabetically. Duplicates
 * collapse; blank slugs are dropped.
 */
export function buildProviderRows(
  catalog: readonly ComposioToolkitEntry[],
  extra: readonly string[],
  connected: Readonly<Record<string, boolean>>,
): ProviderRow[] {
  const seen = new Set<string>();
  const rows: ProviderRow[] = [];
  const entries: ComposioToolkitEntry[] = [
    ...catalog,
    // A slug typed into the sign-in-by-slug field has no catalog entry by
    // definition — it is the escape hatch for a provider the catalog omitted.
    // It still gets a tile, rendered entirely from local metadata.
    ...extra.map((slug) => ({ slug, name: "", description: "", logo: null, categories: [] })),
  ];
  for (const entry of entries) {
    const slug = entry.slug.trim().toLowerCase();
    if (!slug || seen.has(slug)) continue;
    seen.add(slug);
    const label = providerLabel({ ...entry, slug });
    rows.push({
      slug,
      label,
      description: entry.description.trim(),
      logoUrl: entry.logo?.trim() || composioLogoUrl(slug),
      category: mapComposioCategory(entry.categories) ?? guessCategory(slug, label),
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
 * The categories actually present in `rows`, in {@link PROVIDER_CATEGORY_ORDER},
 * always led by `All`.
 *
 * Only offering buckets that have something in them is the difference between a
 * filter and a set of traps: a chip that reliably yields an empty grid teaches
 * the operator to distrust the whole row. A manifest list of two providers gets
 * two chips plus `All`, not six.
 */
export function availableCategories(rows: readonly ProviderRow[]): ProviderCategory[] {
  const present = new Set(rows.map((row) => row.category));
  return PROVIDER_CATEGORY_ORDER.filter((c) => c === "All" || present.has(c));
}

/**
 * Narrow rows to a category. `All` is the identity.
 *
 * Composes with {@link filterProviderRows} rather than replacing it — the two
 * are `AND`, so an operator can search inside a bucket. Search that silently
 * cleared the chip would be the more common design and the more annoying one:
 * it throws away half of what the operator already told us.
 */
export function filterByCategory(
  rows: readonly ProviderRow[],
  category: ProviderCategory,
): ProviderRow[] {
  if (category === "All") return [...rows];
  return rows.filter((row) => row.category === category);
}

/**
 * Narrow rows to a search query, matched case-insensitively against the slug,
 * the display label, and the description.
 *
 * All three matter. An operator who knows the product types "google calendar";
 * one who knows Composio types `googlecalendar`; and one who knows neither
 * types "invoices" and should still reach Stripe. That third case is new with
 * #600 — before it, there was no description on the wire to match against.
 */
export function filterProviderRows(rows: readonly ProviderRow[], query: string): ProviderRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...rows];
  return rows.filter(
    (row) =>
      row.slug.includes(q) ||
      row.label.toLowerCase().includes(q) ||
      row.description.toLowerCase().includes(q),
  );
}

/**
 * The tiles to actually render: the category filter and the search composed, in
 * that order.
 *
 * There is no preview cut here any more, and that is the point of #600. The old
 * list collapsed to twelve rows behind a "Show all 123 providers" button
 * because 123 full-width rows were unreadable — the cut was a workaround for the
 * layout, not a feature. A dense tile grid shows all of them at a glance, so the
 * button that stood between the operator and the catalog is gone rather than
 * relabelled.
 */
export function visibleProviderRows(
  rows: readonly ProviderRow[],
  category: ProviderCategory,
  query: string,
): ProviderRow[] {
  return filterProviderRows(filterByCategory(rows, category), query);
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
export function catalogWarning(
  status: Pick<ComposioStatus, "catalogSource" | "catalogNotice">,
): string | null {
  if (status.catalogSource !== "fallback") return null;
  return (
    status.catalogNotice ??
    "Composio's provider catalog could not be fetched, so this is a built-in starter list and may be incomplete."
  );
}
