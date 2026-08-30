// The card every first-run screen is drawn in.
//
// Lifted out of `views/setup/SetupWizard.tsx`, which owned it privately, when
// "Add a host" stopped being a dialog and became a screen of the same flow
// (`views/setup/AddHostPage.tsx`). The two are the same moment from an
// operator's point of view — a host, then a company — and two moments that read
// as one flow have to be built out of one object rather than two that happen to
// look alike.

import type React from "react";

/**
 * A card, centred — not an application shell.
 *
 * An earlier version put edge-to-edge header and footer rules across the
 * viewport, which is the chrome of an app you live in. This is a task you pass
 * through once, and dressing it as an admin panel made a five-field flow look
 * like a broken settings page: rules running off both edges, a narrow column
 * marooned in the middle of them, and — because the content band was `flex-1` —
 * most of a thousand pixels of nothing between the last field and the buttons.
 *
 * One bounded card instead, centred on both axes, with its own header and its
 * own actions. It grows with its content and stops at 88vh, after which the
 * middle scrolls and the header and actions stay put. Nothing floats, nothing
 * stretches, and the eye has one object to land on.
 *
 * **The height cap is on the card, and the scroll is on the middle band.** That
 * is the whole reason a screen is not a dialog here: `DialogContent` scrolls
 * *itself*, so its own title scrolls away and its width is the dialog's rather
 * than the content's — which is how the add-host chooser ended up clipped
 * inside a 24rem popup (issue #1531).
 */
export function OnboardingShell({
  header,
  footer,
  children,
  testId,
}: {
  header?: React.ReactNode;
  footer?: React.ReactNode;
  children: React.ReactNode;
  /** Names the card itself, so a spec can wait on the screen and not a field. */
  testId?: string;
}) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-background p-4 sm:p-6">
      <div
        data-testid={testId}
        className="flex max-h-[88vh] w-full max-w-[34rem] flex-col overflow-hidden rounded-2xl border bg-card shadow-xl"
      >
        {header && <div className="shrink-0 border-b px-6 py-5">{header}</div>}
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">{children}</div>
        {footer && <div className="shrink-0 border-t px-6 py-4">{footer}</div>}
      </div>
    </div>
  );
}
