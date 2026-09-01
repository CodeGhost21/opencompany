// The one row across the top of the window: which company you are in, and who
// you are signed in as.
//
// Both controls used to live in the sidebar column — the switcher at its head,
// under a reserved strip for the traffic lights, and the profile row in its
// footer. That put the two facts that are *about the console rather than about
// the page* at opposite ends of a 13.5rem column, and it put the macOS traffic
// lights on top of a narrow column instead of across a bar, so the lights
// overlapped the switcher and the window had no title row to speak of.
//
// Now they are one row spanning the full window width, above the sidebar and
// above the content. Three rules hold it together:
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
// **It narrows by dropping whole facts, never by scrolling.** The window's
// `minWidth` is 880 (`src-tauri/tauri.conf.json`), and the row's contents do
// not fit there at their widest. What goes, in order:
//
//   1. the autonomy pill's **sentence** (below `lg`, 1024px), leaving the
//      shield and the tier's name;
//   2. the company switcher's width, which is capped at `max-w-72` and
//      truncates its label — it has always done this.
//
// The tier's *name* never goes: it is the fact the pill exists to carry, and a
// row that has silently dropped it looks identical to a company with no policy
// at all. Nothing here wraps and nothing scrolls — every item is `flex-none`
// except the deliberately elastic middle, so the row cannot grow a second line
// or a horizontal scrollbar however narrow the window gets.

import {
  WINDOW_TITLE_BAR_HEIGHT,
  WindowControlsInset,
} from "@/components/window-chrome";

export function WindowTitleBar({
  switcher,
  autonomy,
  profile,
}: {
  /** The company/host switcher. Leads the row, right of the traffic lights. */
  switcher: React.ReactNode;
  /**
   * The standing autonomy policy, right-aligned before the profile control.
   *
   * Optional, and absent is a real state rather than a gap: the pill renders
   * nothing when the host has not said what the tier is, and the row closes up
   * around it. See `AutonomyPill`.
   */
  autonomy?: React.ReactNode;
  /** The profile / account control, at the far right. */
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
      {/* The standing policy, immediately before the profile control. It is a
          control rather than static chrome — the tier can be changed from here
          — so it deliberately does NOT carry `data-tauri-drag-region`: the
          attribute is opt-in per element, and marking a button would hand its
          presses to the window drag instead of to the menu it opens. The
          `flex-1 self-stretch` spacer above is the row's only elastic member,
          so that spacer — not the pill — is what keeps the band between the
          switcher and here grabbable. See `AutonomyPill`.
          Rendered without a wrapper on purpose: the pill returns `null` when
          the tier is unknown, and a wrapper would survive it as an empty box
          holding one `gap-2` of dead space open in the middle of the row. */}
      {autonomy}
      {/* Last in the DOM as well as last on screen, so tab order reads
          left-to-right across the row. */}
      <div className="flex-none">{profile}</div>
    </div>
  );
}
