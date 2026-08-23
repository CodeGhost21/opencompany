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
 * The panel opens UPWARD (`bottom-full`). Its only caller is the compact
 * legend, which the fullscreen field pins at `bottom-5` inside an
 * `overflow-hidden` container — a downward panel would be clipped after a few
 * pixels, which is exactly the unreadable caveat this component exists to fix.
 */
export function WorkflowPlacementNotice() {
  return (
    <details className="group relative border-l border-os-border pl-3 font-mono text-3xs text-os-dim">
      <summary className="flex cursor-pointer list-none items-center gap-1 rounded-sm-t px-1 py-0.5 underline decoration-dotted underline-offset-2 outline-none hover:bg-os-bg/85 hover:text-os-muted focus-visible:ring-1 focus-visible:ring-os-accent [&::-webkit-details-marker]:hidden">
        <Info className="h-3 w-3 shrink-0" aria-hidden="true" strokeWidth={2} />
        workflow placement is inferred
      </summary>
      <p className="absolute bottom-full right-0 z-50 mb-1 hidden w-72 rounded-sm-t border border-os-border-strong bg-os-bg px-2 py-1.5 font-sans text-2xs leading-relaxed text-os-text shadow-lg group-open:block">
        {DERIVED_NOTICE}
      </p>
    </details>
  );
}
