// Where a view lives in the address, as opposed to what a view is.
//
// `console-routes.ts` answers "does this surface route at all". This answers
// "what does its address look like", and the two are deliberately separate: a
// view's id is its identity and never changes, while its *path* is a statement
// about how the console is organised and has changed twice already.
//
// # The prefix
//
// Company's surfaces used to be top-level addresses — `#/ledgers`, `#/brain`,
// `#/workspace`, `#/finances` — while being drawn as Company's pages in both
// the sidebar and the content rail. The navigation said one thing and the
// address bar said another, so a link nobody could place ("what is
// `#/ledgers`?") named a page every operator knows as Company → Work.
//
// They are `#/company/<page>` now. The view ids are untouched: `ledgers` is
// still the view, `work` is only what it is called in a URL, exactly as "Work"
// is only what it is called on a row. See `COMPANY_PAGES`.
//
// # Old addresses still resolve
//
// Not through a rewrite table — through {@link parseConsolePath} returning
// `null` for anything that is not prefixed, which drops the address into the
// router's ordinary head/sub resolution, which is what it always did. So every
// `#/ledgers/goals` and `#/team/<id>` link ever minted keeps working, and the
// router's `canonicalize` quietly replaces it with the prefixed form on
// arrival. Nothing has to enumerate the old spellings.

import type { View } from "@/lib/console-routes";

/** The first segment every Company surface is filed under. */
export const COMPANY_SEGMENT = "company";

/**
 * The slug each Company page takes in the address, and the view it resolves to.
 *
 * The slug is the word on the row wherever the two can agree — `work` for the
 * `ledgers` view, because "Work" is what the row says and `#/company/ledgers`
 * would be the third name for one thing. Where the row's word and the view id
 * already match, so does the slug.
 *
 * `team` and `tasks` are here without rows of their own: they are the
 * deep-link surfaces `isNavigationActive` already files under Company (a
 * teammate is a seat on the org chart, a task is a card on the board), so their
 * addresses belong under it too.
 */
export const COMPANY_PAGES: Readonly<Record<string, View>> = {
  work: "ledgers",
  workspace: "workspace",
  brain: "brain",
  finances: "finances",
  team: "team",
  // Two slugs, one view, and the order matters: `SLUG_FOR_VIEW` inverts this
  // map and the last entry wins, so `team` resolves *out* to `agent` while
  // `#/company/team/<id>` still resolves *in*. That is what retires the old
  // address without breaking it — an old link lands, and `canonicalize`
  // rewrites the bar to `#/company/agent/<id>` on arrival.
  //
  // Singular deliberately: this address is one teammate. The plural
  // `#/company/agents` is the roster, and it is a different page.
  agent: "team",
  tasks: "tasks",
};

/** The slug a Company view takes, or `undefined` if it is not one. */
const SLUG_FOR_VIEW: Readonly<Partial<Record<View, string>>> = Object.fromEntries(
  Object.entries(COMPANY_PAGES).map(([slug, view]) => [view, slug]),
);

/**
 * The canonical address for a resolved route, without the leading `#/`.
 *
 * One place decides, so the router's `navigate`, its `canonicalize` and every
 * `href` helper cannot disagree about what a view's address is — which is the
 * failure mode a prefix invites: a link built by hand in one file that keeps
 * pointing at the flat form long after nothing else does.
 */
export function formatConsolePath(view: View, sub: string | null): string {
  const slug = SLUG_FOR_VIEW[view];
  const head = slug ? `${COMPANY_SEGMENT}/${slug}` : view;
  return sub ? `${head}/${sub}` : head;
}

/**
 * Resolve a prefixed address, or `null` to leave it to the ordinary rules.
 *
 * `null` is the answer for every address that is not `#/company/…`, which is
 * what keeps this additive: an unprefixed hash reaches the router's existing
 * head/sub resolution untouched, so every old link still lands.
 *
 * **A `company` head whose next segment is not a page slug is Company's own
 * sub-page**, not a broken prefix. `#/company/graph` is the knowledge graph and
 * `#/company/<deskId>` is the org chart focused on a desk — both predate this
 * and both still mean what they meant. The cost is that a desk whose id is
 * literally `work`, `brain`, `team`, `tasks`, `workspace` or `finances` would
 * be shadowed by the page of that name; six reserved words against a prefix
 * that makes every other address legible is the trade, and it is stated here
 * rather than discovered.
 */
export function parseConsolePath(segments: readonly string[]): [View, string | null] | null {
  if (segments[0] !== COMPANY_SEGMENT) return null;
  const second = segments[1];
  if (second === undefined) return ["company", null];
  const view = COMPANY_PAGES[second];
  if (!view) return ["company", second];
  return [view, segments[2] ?? null];
}

/**
 * A ready-made `href` for a view and an optional second segment.
 *
 * Every link to a Company surface goes through this rather than composing
 * `#/tasks/${id}` by hand. Hand-composed addresses are what a prefix breaks:
 * they keep resolving — the router still accepts the old spelling — so nothing
 * fails, and the console quietly ships two spellings of the same page, one of
 * which the address bar rewrites the instant it is followed.
 *
 * The segment is percent-encoded here and NOT in `formatConsolePath`, which is
 * the router's own formatter and is handed a `sub` that has already come off
 * the address. Encoding there would double-encode a task id on every
 * `canonicalize`; not encoding here would break the first workspace node whose
 * id contains a slash.
 */
export function consoleHref(view: View, sub?: string | null): string {
  return `#/${formatConsolePath(view, sub == null ? null : encodeURIComponent(sub))}`;
}
