import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { VIEWS as ROUTED_VIEWS, type View } from "@/lib/console-routes";

/**
 * Every page's title comes from `PageHeader`. A view that hand-rolls an `<h1>`
 * fails here (issue #1763).
 *
 * # Why this is a test rather than a convention
 *
 * The console reached twelve distinct heading styles without anyone deciding
 * to have twelve. Each one was reasonable where it was written — `text-xl`
 * because the page felt smaller, `text-lg font-medium` because it was a
 * sub-page, `text-sm` because it lived in a toolbar — and none of them is
 * visible as drift until you put four screens side by side, which no reviewer
 * of any single PR ever does.
 *
 * So this guards the *mechanism* rather than the values: a page cannot invent a
 * thirteenth style, because a page cannot write a heading at all. It is the
 * same argument `scripts/ci/assert-design-tokens.sh` makes about raw hex —
 * a grep is cheaper than the argument, and it does not get tired.
 *
 * # What it does not check
 *
 * Nothing about how `PageHeader` looks. Its type scale, its bar and its
 * hairline are decided in one file; if they are wrong they are wrong once,
 * which is the entire point of moving them there.
 *
 * `h2` and below are `page-section-heading-level.test.ts`'s business.
 */

const VIEWS = new URL("../../src/views", import.meta.url).pathname;

/**
 * Files allowed to open an `<h1>` of their own, and how many.
 *
 * The count is load-bearing. A bare allowlist would let `WorkflowsView` — which
 * legitimately keeps one — quietly grow a second, which is exactly the drift
 * this test exists to stop. Every entry below is a heading that names *the open
 * item* or lives *outside the console shell*, not a page title that could have
 * been a `PageHeader` and was not.
 *
 * Adding a row is a design decision, not a formality: say why the heading
 * cannot be a page header, in the same register as the rows already here.
 */
const HAND_ROLLED: Record<string, { count: number; why: string }> = {
  "Login.tsx": {
    count: 1,
    why:
      "Sign-in, outside the console shell. A hero heading centred in a `max-w-md` " +
      "column with no page around it — there is no bar for a bar-shaped header to sit in.",
  },
  "setup/SetupWizard.tsx": {
    count: 2,
    why:
      "The first-run flow, outside the console shell. These head a wizard *step* " +
      "and its completion screen, neither of which is a page an address reaches.",
  },
  "setup/AddHostPage.tsx": {
    count: 1,
    why: "Also the first-run flow, for the same reason.",
  },
  "WorkflowsView.tsx": {
    count: 1,
    why:
      "The workflow detail identity row (#1135/#1138), pinned by " +
      "`workflow-toolbar-layout.test.ts`: two rows, because identity-and-state and " +
      "act-on-it are different questions. It names the open workflow, not the page — " +
      "the page's own header is the index's, and that one is a `PageHeader`.",
  },
  "chat/ChatHeader.tsx": {
    count: 1,
    why:
      "The channel bar. It names the open channel and changes as you switch, and its " +
      "title sits inside a `group/title` whose hover reveals the copy control beside " +
      "it — an affordance that only works while the heading and the button share a " +
      "parent this file owns.",
  },
  "TaskDetailView.tsx": {
    count: 1,
    why:
      "The card's title inside the Work detail pane, above the compressed metadata " +
      "row #1347/#1348/#1349 cut 190px of preamble down to. A bar with a hairline " +
      "over it is the chrome those issues removed.",
  },
  "team/AgentDetailView.tsx": {
    count: 1,
    why:
      "The teammate profile block: a 56px avatar that is itself the control for " +
      "changing it (#1181), the name, the role, and a row of desk and tier badges. " +
      "It also renders only once the teammate has loaded, so it cannot be hoisted " +
      "to a header that has to exist through the loading and error states too.",
  },
};

/** Every `.tsx` under `src/views`, as paths relative to it. */
function views(dir = VIEWS, prefix = ""): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) return views(join(dir, entry.name), rel);
    return entry.name.endsWith(".tsx") ? [rel] : [];
  });
}

/**
 * `<h1` outside comments.
 *
 * A doc comment that *names* the anti-pattern it is warning about must not
 * count as the anti-pattern — the same trap `assert-design-tokens.sh` documents
 * having fallen into. Block comments are stripped whole (they span lines);
 * `//` only when it opens the line, so a `//` inside a string literal on a line
 * of real code cannot blind the scan to an `<h1>` before it.
 */
function handRolledCount(source: string): number {
  const code = source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^[ \t]*\/\/.*$/gm, "");
  return code.match(/<h1[\s>]/g)?.length ?? 0;
}

const SOURCES = new Map(views().map((rel) => [rel, readFileSync(join(VIEWS, rel), "utf8")]));

describe("page headers come from PageHeader (#1763)", () => {
  it("finds views to check at all, so a broken glob cannot pass silently", () => {
    expect(SOURCES.size).toBeGreaterThan(20);
  });

  it("has no view hand-rolling a page heading outside the allowlist", () => {
    const offenders = [...SOURCES]
      .filter(([rel, src]) => !(rel in HAND_ROLLED) && handRolledCount(src) > 0)
      .map(([rel, src]) => `${rel} opens ${handRolledCount(src)} <h1> of its own`);

    expect(
      offenders,
      `Use <PageHeader> instead — src/components/page-header.tsx.\n` +
        `A heading that genuinely cannot be one needs a row in HAND_ROLLED ` +
        `saying why.\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("holds every allowlisted file to exactly the count it is allowed", () => {
    const offenders = [...Object.entries(HAND_ROLLED)]
      .map(([rel, { count }]) => {
        const src = SOURCES.get(rel);
        if (src === undefined) return `${rel} is allowlisted but no longer exists`;
        const found = handRolledCount(src);
        return found === count ? null : `${rel} opens ${found} <h1>, allowed ${count}`;
      })
      .filter((line): line is string => line !== null);

    expect(
      offenders,
      `An allowlisted file grew or lost a heading. If it grew one, it is a new ` +
        `page header and belongs in <PageHeader>; if it lost one, drop the row.\n` +
        `${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has every view with a visible page header importing the component", () => {
    // A view that renders a title without importing PageHeader is a view that
    // found some other way to draw one — which is the thing being prevented.
    const drawn = [...SOURCES].filter(([, src]) => src.includes("<PageHeader"));
    const missing = drawn
      .filter(([, src]) => !src.includes('from "@/components/page-header"'))
      .map(([rel]) => rel);

    expect(missing, `render <PageHeader> without importing it: ${missing.join(", ")}`).toEqual([]);
    expect(drawn.length).toBeGreaterThan(15);
  });
});

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
 */
type Names =
  /** The file that renders this view's `<PageHeader>`. */
  | { pageHeader: string }
  /**
   * The file that names it some other way. Only legal for a file already in
   * `HAND_ROLLED` above — the reason lives there, in one place, rather than
   * being restated here and drifting.
   */
  | { handRolled: string };

const NAMED_BY: Record<View, Names> = {
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

describe("every routed view is named by something (#1763)", () => {
  it("has a row for every view the router can reach, and no stale ones", () => {
    // The compile-time `Record<View, …>` already forbids a missing row. This
    // is the runtime half: `VIEWS` (imported as `ROUTED_VIEWS`, since this
    // file already has a `VIEWS` of its own) is derived from `ROUTABLE`, so if the two
    // ever disagree the disagreement is visible here rather than silent.
    expect([...ROUTED_VIEWS].sort()).toEqual([...(Object.keys(NAMED_BY) as View[])].sort());
    expect(ROUTED_VIEWS.length).toBeGreaterThan(15);
  });

  it("has every routed view's named file actually exist", () => {
    const missing = (Object.entries(NAMED_BY) as [View, Names][])
      .map(([view, how]) => ["pageHeader" in how ? how.pageHeader : how.handRolled, view] as const)
      .filter(([file]) => !SOURCES.has(file))
      .map(([file, view]) => `${view} names ${file}, which is not under src/views`);

    expect(missing, missing.join("\n")).toEqual([]);
  });

  it("has every routed view without a documented exception rendering PageHeader", () => {
    const offenders = (Object.entries(NAMED_BY) as [View, Names][])
      .filter((entry): entry is [View, { pageHeader: string }] => "pageHeader" in entry[1])
      .filter(([, how]) => !(SOURCES.get(how.pageHeader) ?? "").includes("<PageHeader"))
      .map(([view, how]) => `${view} is named by ${how.pageHeader}, which renders no <PageHeader>`);

    expect(
      offenders,
      `A routed view lost its page header. A page with no header is a page a ` +
        `screen reader cannot announce — which is the state Workspace and the ` +
        `unknown-route page were in before #1763.\n` +
        `Render <PageHeader> there (use hidden if the page is its own content), ` +
        `or move the row to handRolled and add the file to HAND_ROLLED with a ` +
        `reason.\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("has every handRolled exception carrying its reason in HAND_ROLLED", () => {
    // One reason, in one place. A second copy here is a second thing to keep
    // true, and the whole argument of this file is that nobody notices when a
    // second copy stops being true.
    const undocumented = (Object.entries(NAMED_BY) as [View, Names][])
      .filter((entry): entry is [View, { handRolled: string }] => "handRolled" in entry[1])
      .filter(([, how]) => !(how.handRolled in HAND_ROLLED))
      .map(([view, how]) => `${view} names ${how.handRolled} as hand-rolled, but it has no HAND_ROLLED row`);

    expect(undocumented, undocumented.join("\n")).toEqual([]);
  });
});
