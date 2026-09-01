// The one row across the top of the window: which company you are in, the two
// places you jump to from anywhere, what the agents are allowed to do, and who
// you are signed in as.
//
// The switcher and the profile control used to live in the sidebar column — the
// switcher at its head, under a reserved strip for the traffic lights, and the
// profile row in its footer. That put the two facts that are *about the console
// rather than about the page* at opposite ends of a 13.5rem column, and it put
// the macOS traffic lights on top of a narrow column instead of across a bar,
// so the lights overlapped the switcher and the window had no title row to
// speak of.
//
// Now they are one row spanning the full window width, above the sidebar and
// above the content. Four rules hold it together:
//
// **It is chrome, not content.** It lives outside the sidebar's container and
// outside the scrolling content card, so it survives the sidebar collapsing and
// never scrolls away. The sidebar starts below it.
//
// **It exists in the browser too.** Only the traffic-light inset is gated on
// {@link usesOverlayTitleBar} — the row itself is not. One layout that is right
// everywhere beats a desktop layout and a web layout that drift apart, and the
// difference between them is a 72px spacer.
//
// **Everything on it is centred by one rule.** A single `items-center` on this
// flex row, and no per-item margins: see {@link WINDOW_TITLE_BAR_HEIGHT} for
// why it is the height at which that rule also lands on the traffic lights'
// centre line, which is the one item here whose position macOS owns.
//
// **Its right-hand end is three groups, not five loose items.** See
// {@link TITLE_BAR_GROUP} for what the hairlines between them are doing, and
// {@link TITLE_BAR_LADDER} for the single place that decides what the row drops
// as the window narrows.

import {
  WINDOW_TITLE_BAR_HEIGHT,
  WindowControlsInset,
} from "@/components/window-chrome";
import { cn } from "@/lib/utils";

/**
 * What the row drops as the window narrows — decided here, once, rather than by
 * each item picking its own breakpoint.
 *
 * The window's `minWidth` is 880 (`src-tauri/tauri.conf.json`) and the row's
 * contents do not fit there at their widest, so something has to go. What made
 * that a design problem rather than an arithmetic one is that every item can
 * make a locally reasonable case for surviving; the order below is the answer,
 * and it lives in one object so that reading it is reading the whole ladder
 * rather than grepping four components for `hidden`.
 *
 * | width   | what goes            |
 * |---------|----------------------|
 * | ≥ 1280  | nothing              |
 * | < 1280  | autonomy's sentence  |
 * | < 1024  | the company's name   |
 * | < 768   | the Overview glyph   |
 * | floor   | approvals + autonomy + you |
 *
 * **The autonomy sentence goes first** because it is the longest thing here and
 * the only one whose absence loses no fact: the tier's *name* stays, and the
 * host's full sentence is one hover away on the trigger's `title`.
 *
 * **The company's name goes second** because the switcher is the widest item in
 * the row and the most redundant one in it — the window already belongs to one
 * company, and the glyph, the chevron and the hover title all survive.
 *
 * **Overview goes third, and Approvals never does.** Overview is a destination
 * you *choose*; a pending count is one that *chooses you*. Between them that is
 * the whole argument for which of the two a narrow window keeps.
 *
 * **The floor is approvals, autonomy and you.** What is waiting on you and what
 * the agents may do are the two things that must survive any width — a row that
 * has silently dropped either looks identical to a company with nothing pending
 * and no policy at all.
 *
 * The tier's *name* never goes for the same reason. Nothing here wraps and
 * nothing scrolls — every item is `flex-none` except the deliberately elastic
 * middle — so the row cannot grow a second line or a horizontal scrollbar
 * however narrow the window gets.
 */
export const TITLE_BAR_LADDER = {
  /**
   * The host's leading sentence on the autonomy pill. Consumed by
   * `AutonomyPill`; `hidden` rather than truncated, because half a sentence
   * about what the agents may do would still read as a complete claim.
   */
  autonomySentence: "hidden xl:inline",
  /**
   * The company's name beside the switcher's glyph. Consumed by `HostSwitcher`'s
   * `titlebar` variant, which keeps the glyph, the status dot and the chevron.
   */
  companyName: "hidden lg:flex",
  /** The Overview glyph. Applied by the row itself, to the slot it sits in. */
  overview: "hidden md:inline-flex",
} as const;

/**
 * The shape both title-row glyph buttons take — Overview and Approvals.
 *
 * Exported so the two are one decision rather than two copies that drift. It is
 * `relative` because the approvals chip is positioned against it, and it has no
 * fill at rest for the reason the switcher's trigger does not: these stand on
 * the window chrome rather than in a card, and announce themselves on hover and
 * on focus.
 */
export const TITLE_BAR_ICON_BUTTON = cn(
  "relative inline-flex size-8 flex-none items-center justify-center rounded-lg",
  "text-muted-foreground transition",
  "hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
  "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
  // The view you are already on. Keyed off `aria-current` so the appearance and
  // the announced state cannot disagree.
  "aria-[current=page]:bg-sidebar-accent aria-[current=page]:text-sidebar-accent-foreground",
);

/**
 * One of the row's three right-hand groups.
 *
 * `empty:hidden` is load-bearing and not tidiness. Both of the last two groups
 * hold a control that renders **nothing** in a real state — the autonomy pill
 * when the host has not said what the tier is, the profile control on a host
 * with no sign-in — and the element handed to this row is truthy either way, so
 * the layout cannot ask. An empty wrapper would otherwise hold one `gap-2` of
 * dead space open, and, now that a group carries its own divider, would leave a
 * hairline standing beside nothing.
 */
const TITLE_BAR_GROUP = "flex flex-none items-center gap-2 empty:hidden";

/**
 * The hairline that opens a group.
 *
 * Three things after the company name is an accumulation; the rules are what
 * make it a design. They say *these are different kinds of thing* — where you
 * are going, what the agents may do, and who you are — so the eye reads three
 * objects rather than five loose controls, and a gap alone cannot say that at
 * any width that also fits.
 *
 * A `::before` rather than an element of its own, because that is what ties it
 * to {@link TITLE_BAR_GROUP}'s `empty:hidden`: a pseudo-element does not make
 * its host non-empty, so the rule vanishes with the group it introduces instead
 * of surviving it as a stray mark. `--chrome-border` is the border token for
 * this layer — the row stands on `--chrome`.
 */
const TITLE_BAR_DIVIDER = "before:mr-2 before:h-5 before:w-px before:bg-chrome-border before:content-['']";

export function WindowTitleBar({
  switcher,
  overview,
  approvals,
  autonomy,
  profile,
}: {
  /** The company/host switcher. Leads the row, right of the traffic lights. */
  switcher: React.ReactNode;
  /**
   * The Overview jump — first half of the right-hand row's first group, and the
   * first thing the row drops as it narrows. See {@link TITLE_BAR_LADDER}.
   */
  overview?: React.ReactNode;
  /**
   * The Approvals jump, carrying the pending count. Second half of the first
   * group and the one item in the row that ever demands attention, which is why
   * it is the last thing that would ever go.
   */
  approvals?: React.ReactNode;
  /**
   * The standing autonomy policy, the second group.
   *
   * Optional, and absent is a real state rather than a gap: the pill renders
   * nothing when the host has not said what the tier is, and the group closes up
   * around it — hairline included. See `AutonomyPill`.
   */
  autonomy?: React.ReactNode;
  /** The profile / account control, the third group and the far right. */
  profile: React.ReactNode;
}) {
  return (
    // `data-tauri-drag-region` is opt-in per element, not inherited: Tauri
    // starts a drag only when the pressed element is itself marked. So the
    // switcher and the profile control keep their clicks without opting out of
    // anything, and the empty middle has to opt *in* on its own — which is what
    // the spacer below does.
    <div
      data-tauri-drag-region
      data-testid="window-title-bar"
      className="flex w-full flex-none items-center gap-2 px-3"
      style={{ height: WINDOW_TITLE_BAR_HEIGHT }}
    >
      {/* Renders nothing off the macOS desktop, where the lights do not float
          over the page and there is nothing to clear. */}
      <WindowControlsInset />
      {/* Capped rather than stretched. The trigger was sized for a sidebar
          column, and left to itself in a 1280px row it would run halfway across
          the window naming a company whose name is three words long. It already
          truncates; this gives it something to truncate against. */}
      <div className="min-w-0 max-w-72">{switcher}</div>
      {/* The draggable middle. `self-stretch` so the grabbable area is the full
          height of the row rather than a hairline through its centre. */}
      <div data-tauri-drag-region aria-hidden="true" className="min-w-0 flex-1 self-stretch" />
      {/* Group one — where you are going. The two places you jump to from
          anywhere, held tighter to each other (`gap-1`) than to the groups
          beside them, so they read as one object. No divider: it is the first
          group, and the elastic spacer already separates it from the switcher.

          It carries no `data-tauri-drag-region`, and neither do the groups
          below. The attribute is opt-in per element, so a control simply does
          not have it — marking one would hand its presses to the window drag
          instead of to the thing it opens. The `flex-1 self-stretch` spacer
          above is the row's only elastic member, so that spacer, and not any of
          these, is what keeps the band grabbable. */}
      <div
        data-testid="title-bar-group-go"
        className={cn(TITLE_BAR_GROUP, "gap-1")}
      >
        {/* The ladder's third rung, applied by the row rather than by the button
            — one place decides what goes, and the button decides what it is. */}
        <span
          data-testid="title-bar-overview-slot"
          className={cn(TITLE_BAR_LADDER.overview, "empty:hidden")}
        >
          {overview}
        </span>
        {approvals}
      </div>
      {/* Group two — what the agents may do. Rendered inside a wrapper on
          purpose, unlike the bare slot this used to be: the wrapper is what
          carries the hairline, and `empty:hidden` is what stops the wrapper
          surviving a pill that returned `null`. */}
      <div
        data-testid="title-bar-group-state"
        className={cn(TITLE_BAR_GROUP, TITLE_BAR_DIVIDER)}
      >
        {autonomy}
      </div>
      {/* Group three — you. Last in the DOM as well as last on screen, so tab
          order reads left-to-right across the row. */}
      <div
        data-testid="title-bar-group-you"
        className={cn(TITLE_BAR_GROUP, TITLE_BAR_DIVIDER)}
      >
        {profile}
      </div>
    </div>
  );
}
