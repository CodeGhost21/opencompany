// SPDX-License-Identifier: GPL-3.0-or-later

import { Info } from "lucide-react";

import { DERIVED_NOTICE } from "./adapter";

/**
 * The graph's one non-declared placement, available without a hover-only cue.
 *
 * A native disclosure gives the caveat a visible affordance and lets pointer,
 * keyboard, and touch users keep its canonical explanation open while they
 * read the wheel.
 *
 * The explanation opens ABOVE the summary and stays IN FLOW rather than
 * floating as an absolute popover. Both facts are forced by where this lives:
 * its only caller is the compact legend, which the fullscreen field pins at
 * `bottom-5 left-5` inside an `overflow-hidden` container.
 *
 * - Opening downward put the panel in the ~20px strip below the legend, where
 *   it was clipped away — the caveat stayed unreadable for exactly the
 *   pointer, keyboard, and touch users this component exists to serve.
 * - An absolute panel then had the same problem sideways: anchored to one
 *   edge of a summary whose position moves with the legend's wrapping, it ran
 *   off whichever side it was not anchored to at narrow widths.
 *
 * In flow, the panel simply grows the legend box upward into the field, which
 * is the one direction that always has room. `flex-col-reverse` renders it
 * above the summary while leaving the summary first in the DOM, so the native
 * disclosure semantics and focus order are untouched.
 */
export function WorkflowPlacementNotice() {
  return (
    <details className="group flex flex-col-reverse border-l border-os-border pl-3 font-mono text-3xs text-os-dim">
      <summary className="flex cursor-pointer list-none items-center gap-1 whitespace-nowrap rounded-sm-t px-1 py-0.5 underline decoration-dotted underline-offset-2 outline-none hover:bg-os-bg/85 hover:text-os-muted focus-visible:ring-1 focus-visible:ring-os-accent [&::-webkit-details-marker]:hidden">
        <Info className="h-3 w-3 shrink-0" aria-hidden="true" strokeWidth={2} />
        workflow placement is inferred
      </summary>
      <p className="mb-1 hidden w-72 max-w-full rounded-sm-t border border-os-border-strong bg-os-bg px-2 py-1.5 font-sans text-2xs leading-relaxed text-os-text shadow-lg group-open:block">
        {DERIVED_NOTICE}
      </p>
    </details>
  );
}
