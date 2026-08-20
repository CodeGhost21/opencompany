import type { View } from "@/components/app-shell";

/**
 * Whether the address names a surface that renders edge-to-edge instead of on
 * the inset content card (issue #1178).
 *
 * The test is "is this page a canvas or a document", and it is asked of the
 * whole address rather than the view alone, because Workflows is both:
 *
 *   - `overview` is always the force-directed knowledge graph, laid out
 *     against `h-svh` and clipped by its own `overflow-hidden`. Framed, it
 *     would measure the window and be drawn into a box 24px smaller in both
 *     axes.
 *   - `workflows/<id>` is the React Flow canvas, whose viewport transform and
 *     minimap viewbox are both computed from the container's measured rect —
 *     precisely the surface #1259 and #1261 just finished un-cropping.
 *   - `workflows` with no id is the browse list: cards, a count and a toolbar.
 *     A document, and it looked like a page missing its edges when it was
 *     lumped in with the canvas.
 *
 * `sub` is the hash's second segment, and the canvas keeps it current — opening
 * a workflow writes `#/workflows/<id>` and leaving one clears it — so this
 * tracks what is actually on screen rather than a second copy of that state.
 */
export function isFullBleed(view: View, sub: string | null): boolean {
  if (view === "overview") return true;
  return view === "workflows" && sub !== null;
}
