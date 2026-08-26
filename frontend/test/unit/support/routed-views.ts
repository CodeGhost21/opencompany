/**
 * Which file names each routed view — shared by the two tests that ask about
 * page headings, so neither can be true of a list the other has not heard of.
 *
 * Not a `.test.ts`, so the unit runner does not collect it (`include` in
 * `vitest.config.ts` matches `*.test.ts` only).
 */

import type { View } from "@/lib/console-routes";
import type { SettingsPage } from "@/views/settings-pages";

/**
 * Which file names each routed view, and how (codex review, #1785).
 *
 * # Why the rest of this file was not enough
 *
 * Everything above is a rule about *headings that exist*: it stops a view
 * inventing a thirteenth style. It says nothing about a view with **no**
 * heading at all, and a floor of "more than 15 files draw a `PageHeader`"
 * cannot: delete the header from `WorkspaceView` and sixteen others still
 * draw one, so the suite stays green. Verified rather than argued — with
 * `<PageHeader …/>` cut out of `WorkspaceView` entirely, this file passed
 * 4 of 4.
 *
 * That is not a hypothetical regression. It is the exact state Workspace was
 * in *before* #1763: no header, no `h1`, a page a screen reader could not
 * announce. A guard that would not have caught the defect it was written for
 * is worth exactly as much as the count it asserts.
 *
 * # The shape
 *
 * `Record<View, …>` over the router's own union, the same trick `ROUTABLE` in
 * `lib/console-routes.ts` uses: a view added to the union with no row here is
 * a **compile error**, caught by `npm run typecheck:unit`, so a new route
 * cannot be added without someone deciding what names it. `VIEWS` is then
 * iterated at runtime, so the two cannot drift apart either.
 *
 * The mapping is by hand and cannot be derived: `app-shell.tsx` renders a
 * wrapper for several of these (`CompanyView`, `SettingsSection`,
 * `TaskDetailRoute`) and the header lives one level down, which is a fact
 * about the component tree that no grep over route names can see.
 *
 * # What this still cannot see, and what covers it
 *
 * **Control flow.** Everything in this file reads source text, so a view
 * satisfies it by *containing* `<PageHeader` — not by rendering one. A file
 * with an early `return` for a loading or error state above its header passes
 * here while shipping a state with no `h1` at all, which is exactly what
 * `SearchView`, `HostingView`, `WalletView` and `InvoicingView` were doing
 * (codex review on #1785).
 *
 * Two files close that gap, and neither is the adoption scan:
 *
 *   - `page-header-precedes-every-return.test.ts` asks a strictly weaker,
 *     decidable question — is there *any* JSX `return` textually above the
 *     header? — over every routed view and every settings page, so a new route
 *     is covered the day it is added.
 *   - `settings-page-named-in-every-state.test.ts` renders six of those pages
 *     in their loading and error states and asks the DOM for the `h1`, which
 *     is the only evidence that actually proves it.
 *
 * Do not extend the adoption scan to try to reach either. A scan that worked
 * out which branch runs would be wrong in a way nobody could see, which is the
 * failure mode it exists to prevent.
 */
/**
 * One thing a route can render. A route that dispatches gets several.
 *
 * The single-leaf version of this was wrong three times over, all found by the
 * same review: `company` was mapped to `TeamView` alone while
 * `CompanyView.tsx:116-129` sends `#/company/graph` to `Overview` and every
 * other segment to `OrgChartView`; and `chat` and `team` named components that
 * are simply **absent** in some of their own states. Each wrong leaf switched
 * the guard off for a whole route while reporting green — the exact failure
 * these tests exist to catch.
 *
 * So the rule is: **a route may only name a file that carries a heading in
 * every state that file is on screen for.** If the route dispatches, enumerate
 * the leaves. If a state cannot carry one, that is a per-state decision written
 * into the view, not a row here — a route-level exemption also hides every
 * future regression under that route.
 */
export type Leaf =
  /** The file renders a `<PageHeader>` — visible, or `hidden` for a page that is its own content. */
  | { pageHeader: string }
  /**
   * The file names the page some other way. Only legal for a file already in
   * `HAND_ROLLED` in `page-header-adoption.test.ts` — the reason lives there,
   * in one place, rather than being restated here and drifting.
   */
  | { handRolled: string };

/** Every leaf a route can render. Order is irrelevant; completeness is not. */
export type Names = readonly Leaf[];

export const NAMED_BY: Record<View, Names> = {
  overview: [{ pageHeader: "OperatorOverview.tsx" }],
  /**
   * Three leaves, from `CompanyView`'s own dispatch: `#/company/graph` is the
   * knowledge graph, any other segment is the org chart focused on that desk,
   * and the bare route is the roster.
   */
  company: [
    { pageHeader: "TeamView.tsx" },
    { pageHeader: "company/OrgChartView.tsx" },
    { pageHeader: "Overview.tsx" },
  ],
  /**
   * `#/team/<id>` opens the teammate profile; the bare route is the roster.
   *
   * `AgentDetailView` is a `pageHeader` leaf even though its *loaded* heading
   * is the hand-rolled one `HAND_ROLLED` allows: it also renders a `hidden`
   * header for the four states `Identity` does not mount for, and that is the
   * half a guard has to hold it to. Listing it as `handRolled` asked nothing
   * of it at all — deleting that header passed every check.
   */
  team: [{ pageHeader: "TeamView.tsx" }, { pageHeader: "team/AgentDetailView.tsx" }],
  /**
   * The channel bar names the loaded pane. The three channel-less states —
   * desks failed, desks pending, no channel — are `ChatView`'s own panes, and
   * each carries a `hidden` header so the page is named before a channel is.
   */
  chat: [{ handRolled: "chat/ChatHeader.tsx" }, { pageHeader: "ChatView.tsx" }],
  conversation: [{ pageHeader: "Conversation.tsx" }],
  inbox: [{ pageHeader: "InboxView.tsx" }],
  /**
   * `#/tasks/<id>` is the card detail pane, not the board. A `pageHeader` leaf
   * for the same reason `team/AgentDetailView.tsx` is: its loaded heading is
   * the card's own title, which `HAND_ROLLED` allows, and it also renders a
   * `hidden` header for the deleted-card state, where there is no title to
   * show. As `handRolled` the guard asked nothing of it.
   */
  tasks: [{ pageHeader: "TaskDetailView.tsx" }],
  ledgers: [{ pageHeader: "LedgersView.tsx" }],
  workspace: [{ pageHeader: "WorkspaceView.tsx" }],
  approvals: [{ pageHeader: "ApprovalsView.tsx" }],
  workflows: [{ pageHeader: "WorkflowsView.tsx" }],
  observatory: [{ pageHeader: "observatory/ObservatoryView.tsx" }],
  pages: [{ pageHeader: "PagesView.tsx" }],
  finances: [{ pageHeader: "FinancesView.tsx" }],
  /** `SettingsSection` is the tab frame; `SettingsView` is the page. */
  settings: [{ pageHeader: "SettingsView.tsx" }],
  feedback: [{ pageHeader: "FeedbackView.tsx" }],
  /**
   * The first-run flow, outside the console shell. `pageHeader` because its two
   * pre-wizard states (cannot read the instance, still reading it) carry a
   * `hidden` header — "outside the shell" is a reason not to paint a bar, not a
   * reason to have no name.
   */
  setup: [{ pageHeader: "setup/SetupWizard.tsx" }],
  "not-found": [{ pageHeader: "UnknownRouteView.tsx" }],
};

/**
 * The same question one level down: Settings is a single routed view whose
 * `sub` segment picks one of ten pages, each of which draws its own
 * `PageHeader`. `#/settings/people` is an address an operator can bookmark, so
 * "the routed views are covered" is not the whole answer — `PeopleView`'s
 * loading state had no `h1` and no routed-view check could have seen it.
 *
 * `Record<SettingsPage, …>` over the table in `settings-pages.ts`, so a new
 * settings page with no row is a compile error, for the same reason
 * `NAMED_BY` is a `Record<View, …>`.
 */
export const SETTINGS_NAMED_BY: Record<SettingsPage, string> = {
  general: "SettingsView.tsx",
  people: "PeopleView.tsx",
  oauth: "OAuthView.tsx",
  mcp: "McpServersView.tsx",
  inference: "InferenceView.tsx",
  hosting: "HostingView.tsx",
  search: "SearchView.tsx",
  skills: "SkillsView.tsx",
  brain: "MemoryView.tsx",
  usage: "UsageView.tsx",
};
