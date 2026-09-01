// The Settings section's sub-page table, and the helpers that read it.
//
// It lives in its own module rather than beside the section that renders it so
// that anything *pointing at* a sub-page can name one without importing the
// section — which imports every view under it, and would import itself back
// through them. `device-pairing.tsx` is the case that forced it: it tells a
// desktop user where to go, and for one release it told them to go somewhere
// that did not exist (issue #1476). Directions read from this table cannot say
// that again — a page id that is not here is a type error.

import {
  BrainCircuit,
  ChartColumnBig,
  Globe,
  Search,
  type LucideIcon,
  Settings2,
  Sparkles,
  UserCog,
} from "lucide-react";

/** The sub-pages that live under Settings. The id is the hash's second segment. */
export const SETTINGS_PAGES = [
  {
    id: "general",
    label: "General",
    icon: Settings2,
    hint: "Approvals, connection, lifecycle, domain, mail",
    group: "identity",
  },
  {
    id: "people",
    label: "People",
    icon: UserCog,
    hint: "Who can sign in, and as what",
    group: "identity",
  },
  // One question per page. "Connections" carried five — third-party accounts,
  // MCP servers, inference, channels, repositories — so each was something an
  // operator scrolled past on the way to another. The first three became pages;
  // the last two left the product.
  //
  // Two of those three then left this rail entirely: third-party accounts (as
  // **Apps**) and MCP servers are the Connections section now, at
  // `#/connections/apps` and `#/connections/mcp`, because what the company can
  // act through is something an operator reads repeatedly rather than
  // configures once. Both `#/settings/oauth` and `#/settings/mcp` still
  // resolve, rewritten onto the section by `console-route-rewrites.ts`, so
  // every link minted while they lived here works.
  //
  // Inference stayed, and so did Hosting and Search below it, for the reason
  // stated twice in this file: a credential form belongs beside the one thing
  // it unlocks. The model, the deploy target and the search provider are three
  // such things; filing them under a section named for the act of connecting
  // would separate each credential from what it is for.
  { id: "inference", label: "Inference", icon: BrainCircuit, hint: "The model teammates think with", group: "integrations" },
  // A credential form belongs beside what it unlocks. An operator looking for
  // "where do I put my Vercel token" searches for hosting, so it sits here
  // rather than inside a third-party-accounts drawer.
  { id: "hosting", label: "Hosting", icon: Globe, hint: "Where this company's sites go live", group: "integrations" },
  // Beside Hosting for the same reason: a credential form belongs beside what
  // it unlocks, and an operator looking for "where do I put my Brave key"
  // searches for search.
  { id: "search", label: "Search", icon: Search, hint: "Where teammates look things up", group: "integrations" },
  // "What this company knows how to do" read as capability the company performs
  // — the implication issue #569 exists to remove, set here *before* the tab
  // gets a chance to correct it. The siblings describe their content; so does
  // this now.
  { id: "skills", label: "Skills", icon: Sparkles, hint: "Playbooks your teammates read", group: "capability" },
  // Brain is NOT here: it has its own nav row (`#/brain`). It was the one page
  // on this rail an operator came to *read* rather than to change — settings
  // are configuration, and what the company remembers is not configuration.
  // `#/settings/brain` still resolves, rewritten onto the row by
  // `console-route-rewrites.ts`, so every link minted while it lived here works.
  { id: "usage", label: "Usage", icon: ChartColumnBig, hint: "What this company is spending", group: "spend" },
] as const satisfies readonly { id: string; label: string; icon: LucideIcon; hint: string; group: string }[];

export type SettingsPage = (typeof SETTINGS_PAGES)[number]["id"];

/** The settings rail groups related sub-pages without changing their routes. */
export const SETTINGS_PAGE_GROUPS = [
  { id: "identity", label: "Identity & lifecycle" },
  { id: "integrations", label: "Integrations" },
  { id: "capability", label: "Capability" },
  { id: "spend", label: "Spend" },
] as const satisfies readonly { id: (typeof SETTINGS_PAGES)[number]["group"]; label: string }[];

export const DEFAULT_SETTINGS_PAGE: SettingsPage = "general";

/** Whether a hash segment names a real sub-page. */
export function isSettingsPage(sub: string | null): sub is SettingsPage {
  return SETTINGS_PAGES.some((page) => page.id === sub);
}

/** Whether a hash segment names a real sub-page. */
export function resolveSettingsPage(sub: string | null): SettingsPage {
  return isSettingsPage(sub) ? sub : DEFAULT_SETTINGS_PAGE;
}

/**
 * What the sub-nav calls a page, for prose that sends someone to it.
 *
 * Typed to `SettingsPage`, so directions written against this cannot outlive
 * the page they name: renaming a page rewrites the sentence, and removing one
 * stops the build.
 */
export function settingsPageLabel(page: SettingsPage): string {
  return SETTINGS_PAGES.find((p) => p.id === page)!.label;
}

/**
 * The console hash a link to one Settings sub-page needs.
 *
 * Typed for the same reason `settingsPageLabel` is: a link written against this
 * cannot outlive the page it points at.
 *
 * `#/settings/connections` was the standing counter-example: hard-coded in four
 * places across `SetupController` and `SetupDialog`, naming a page that stopped
 * existing when Connections was split into OAuth / MCP / Inference, and so
 * repaired onto General — a dead link that looked like a working one, which is
 * the failure issue #1476 was filed for a release earlier. All four are typed
 * calls to this function now, and all four turned out to mean Inference: every
 * one of them is reached because the company has no usable model. The address
 * itself also answers again, rewritten onto the Connections section.
 */
export function settingsHref(page: SettingsPage): string {
  return `#/settings/${page}`;
}
