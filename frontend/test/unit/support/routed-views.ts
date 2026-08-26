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
export type Names =
  /** The file that renders this view's `<PageHeader>`. */
  | { pageHeader: string }
  /**
   * The file that names it some other way. Only legal for a file already in
   * `HAND_ROLLED` above — the reason lives there, in one place, rather than
   * being restated here and drifting.
   */
  | { handRolled: string };

export const NAMED_BY: Record<View, Names> = {
  overview: { pageHeader: "OperatorOverview.tsx" },
  /** Company and Team are two tabs of one page; `TeamView` draws its header. */
  company: { pageHeader: "TeamView.tsx" },
  team: { pageHeader: "TeamView.tsx" },
  chat: { handRolled: "chat/ChatHeader.tsx" },
  conversation: { pageHeader: "Conversation.tsx" },
  inbox: { pageHeader: "InboxView.tsx" },
  /** `#/tasks/<id>` is the card detail pane, not the board. */
  tasks: { handRolled: "TaskDetailView.tsx" },
  ledgers: { pageHeader: "LedgersView.tsx" },
  workspace: { pageHeader: "WorkspaceView.tsx" },
  approvals: { pageHeader: "ApprovalsView.tsx" },
  workflows: { pageHeader: "WorkflowsView.tsx" },
  observatory: { pageHeader: "observatory/ObservatoryView.tsx" },
  pages: { pageHeader: "PagesView.tsx" },
  finances: { pageHeader: "FinancesView.tsx" },
  /** `SettingsSection` is the tab frame; `SettingsView` is the page. */
  settings: { pageHeader: "SettingsView.tsx" },
  feedback: { pageHeader: "FeedbackView.tsx" },
  setup: { handRolled: "setup/SetupWizard.tsx" },
  "not-found": { pageHeader: "UnknownRouteView.tsx" },
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
