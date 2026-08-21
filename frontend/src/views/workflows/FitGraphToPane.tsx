// Keeps a long pipeline readable — and pointed at its start — when it opens
// (issue #1361).
//
// Renders nothing. Like `RevealSelectedNode` and `WorkflowMiniMap` it is a
// child of `<ReactFlow>` purely so it can reach the canvas store — that is the
// only provider `useReactFlow` can find in this view.
//
// WHAT IT CORRECTS. `<ReactFlow fitView>` calls `getViewportForBounds`, which
// picks the zoom that makes the graph fit and then **centres** it. Both halves
// break on the shape this product actually ships: `layout` places one node per
// depth layer, so a ten-node pipeline is 45:1 and the unbounded fit zooms out
// to 0.353 — far below the point where a 14px node title is still words.
//
// The zoom half is fixed on the `<ReactFlow>` element itself, by giving
// `fitViewOptions` a `minZoom` of `LEGIBLE_FIT_ZOOM` (see `graph.ts` for where
// that number comes from). That is deliberately where it lives: it is declared
// on the fit, it applies from the very first painted frame, and it does not
// depend on this component running at all.
//
// The centring half is what this component is for. Once the zoom stops going
// down, "centred" means the operator lands in the middle of a pipeline with
// neither end on screen — measured on `feature_pipeline` at 1440px: first node
// at x = -256, last at x = 1769, and nothing on screen saying either exists. A
// pipeline is read from its trigger, so a clamped fit is anchored there and the
// graph runs off to the right, where its own arrows already point.
//
// WHAT IT LEAVES ALONE, deliberately:
//
//   * `<ReactFlow minZoom>` is untouched. Issue #1261 set it to 0.1 so an
//     operator CAN zoom out to see shape and so the Zoom Out control is not
//     disabled from load. A floor on the initial fit and a floor on zooming are
//     different decisions, and only the first is #1361's.
//   * A graph whose natural fit is already legible — anything up to about six
//     nodes, which is most authored workflows — is never touched.
//     `startAnchoredFit` returns `null` and React Flow's own fit stands.
//
// WHY IT WATCHES THE TRANSFORM RATHER THAN FIRING ON MOUNT. Two attempts that
// do not work, recorded so they are not tried again:
//
//   * On mount, or in an effect gated on the node array: this is a CHILD of
//     `<ReactFlow>`, and React runs a child's effects before its parent's, so
//     the correction lands and the fit that is about to undo it lands after.
//     Measured — the viewport was set correctly and re-centred a frame later.
//   * Gated on `useNodesInitialized()`: it never turns true on this canvas.
//     `layout()` rebuilds the node array on every repaint, so React Flow's own
//     measurement does not survive (which is why the nodes carry
//     `initialWidth`/`initialHeight` hints at all — see `graph.ts`). Measured:
//     the hook reported `false` for the entire life of the view.
//
// So the trigger is the fit's own result. React Flow clamps with
// `clamp(zoom, minZoom, maxZoom)`, which returns the floor exactly, so a
// viewport sitting at `LEGIBLE_FIT_ZOOM` is the signal that the fit ran and
// wanted to go further. Correcting on that signal cannot race it: it IS it.
//
// WHY ONCE PER GRAPH. The operator's viewport is theirs the moment they touch
// it. Panning, zooming, opening the inspector (which pans — see
// `RevealSelectedNode`) and closing a rail must never yank the canvas back to
// where it started, so a graph gets exactly one correction and the guard is
// keyed by workflow id: a different workflow is a different graph and deserves
// its own.

import { useEffect, useRef } from "react";
import { useReactFlow, useStore, type Node } from "@xyflow/react";

import type { WorkflowNodeData } from "@/lib/workflow-sample";

import {
  centredFitX,
  contentBounds,
  LEGIBLE_FIT_ZOOM,
  startAnchoredFit,
} from "./graph";

export function FitGraphToPane({
  nodes,
  graphId,
}: {
  nodes: Node<WorkflowNodeData>[];
  /** The workflow these nodes belong to, or `null` while none is open. */
  graphId: string | null;
}) {
  const reactFlow = useReactFlow<Node<WorkflowNodeData>>();
  const paneWidth = useStore((s) => s.width);
  const paneHeight = useStore((s) => s.height);
  const viewportX = useStore((s) => s.transform[0]);
  const zoom = useStore((s) => s.transform[2]);
  const correctedFor = useRef<string | null>(null);

  useEffect(() => {
    if (!graphId || nodes.length === 0) return;
    if (paneWidth <= 0 || paneHeight <= 0) return;
    if (correctedFor.current === graphId) return;
    // Not the fit's result yet — either it has not run, or it ran and did not
    // need the floor, in which case the graph fits whole and there is nothing
    // to anchor.
    if (zoom !== LEGIBLE_FIT_ZOOM) return;

    const bounds = contentBounds(nodes);
    // ...and the viewport is placed where only the fit places it. React Flow's
    // zoom controls step by a fixed ratio, so an operator who zooms in from a
    // clamped fit and back out returns to exactly `LEGIBLE_FIT_ZOOM` — without
    // this, the canvas would jump to the graph's start under their hands. See
    // `centredFitX`.
    if (Math.abs(viewportX - centredFitX(bounds, paneWidth, zoom)) > 0.5) return;

    const corrected = startAnchoredFit(bounds, paneWidth, paneHeight);
    if (!corrected) return;

    correctedFor.current = graphId;
    // `duration: 0`. This is not a movement the operator should watch — it is
    // where the canvas was always meant to open, arriving a frame after React
    // Flow's own fit rather than instead of it.
    void reactFlow.setViewport(corrected, { duration: 0 });
  }, [graphId, nodes, paneWidth, paneHeight, viewportX, zoom, reactFlow]);

  return null;
}
