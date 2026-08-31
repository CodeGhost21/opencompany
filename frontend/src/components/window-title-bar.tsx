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
// why 44px is the height at which that rule also lands on the traffic lights'
// centre line, which is the one item here whose position macOS owns.

import {
  WINDOW_TITLE_BAR_HEIGHT,
  WindowControlsInset,
} from "@/components/window-chrome";

export function WindowTitleBar({
  switcher,
  profile,
}: {
  /** The company/host switcher. Leads the row, right of the traffic lights. */
  switcher: React.ReactNode;
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
      {/* Last in the DOM as well as last on screen, so tab order reads
          left-to-right across the row. */}
      <div className="flex-none">{profile}</div>
    </div>
  );
}
