// Keeps the node inspector's subject on screen (issue #1231).
//
// Renders nothing. It is a child of `<ReactFlow>` purely so it can reach the
// canvas store — the same reason `WorkflowMiniMap` is one — and it exists as a
// component rather than as a hook called from `WorkflowsView` because
// `useReactFlow` needs a provider, and the only provider in this view is the
// one `<ReactFlow>` creates for its own children.
//
// The behaviour, and the two halves of it that matter equally:
//
//   opening — pan just far enough that the selected node clears the overlay.
//             "Just far enough" is the point: a node that is already visible
//             does not move the canvas at all, so the common click is still a
//             click and not an animation.
//
//   closing — pan back to exactly the viewport the operator was looking at,
//             which is the property `CanvasShell`'s convention comment already
//             promises ("closing the panel restores it instantly") and which a
//             pan on open would otherwise quietly break. Unless the operator
//             has panned or zoomed since, in which case their view is theirs.
//
// See `node-reveal.ts` for the arithmetic and for why this pans rather than
// zooms or reflows.

import { useEffect, useRef } from "react";
import { useReactFlow, useStore } from "@xyflow/react";

import { NODE_H, NODE_W } from "./graph";
import {
  revealViewport,
  shouldRestore,
  type Viewport,
} from "./node-reveal";

/** How long the reveal and its undo take. Long enough to read as the canvas
 * moving rather than cutting, short enough not to be in the way of the panel
 * that is opening over it. */
const REVEAL_MS = 200;

export function RevealSelectedNode({
  nodeId,
  duration = REVEAL_MS,
}: {
  /**
   * The node the inspector is open on, or `null` when it is closed.
   *
   * Deliberately the INSPECTOR's node rather than `selectedNodeId`: the copilot
   * shares this overlay slot and wins while it is open (issue #303), and a
   * conversation about the whole workflow has no one node it must not hide.
   */
  nodeId: string | null;
  /** Overridable so a test can take the animation out of the way. */
  duration?: number;
}) {
  const { getNode, getViewport, setViewport } = useReactFlow();
  const paneWidth = useStore((s) => s.width);

  /** The viewport to go back to — captured before the FIRST reveal of a run of
   * selections, so walking node to node with the panel open and then closing it
   * returns to where the walk started, not to the last node's stop. */
  const restoreRef = useRef<Viewport | null>(null);
  /** The viewport the last reveal asked for. */
  const appliedRef = useRef<Viewport | null>(null);

  useEffect(() => {
    if (!nodeId) {
      const from = restoreRef.current;
      const to = appliedRef.current;
      restoreRef.current = null;
      appliedRef.current = null;
      // Nothing was moved, so there is nothing to put back — the overwhelmingly
      // common case, since most nodes are nowhere near the overlay.
      if (!from || !to) return;
      if (!shouldRestore(getViewport(), from, to)) return;
      void setViewport(from, { duration });
      return;
    }

    const node = getNode(nodeId);
    if (!node) return;
    const viewport = getViewport();
    const next = revealViewport({
      node: {
        x: node.position.x,
        y: node.position.y,
        // `measured` is React Flow's own measurement and wins when it has one;
        // `initialWidth`/`initialHeight` are the hints `layout` puts on every
        // node (issue #1230) and cover the frame before the first measure.
        width: node.measured?.width ?? node.initialWidth ?? NODE_W,
        height: node.measured?.height ?? node.initialHeight ?? NODE_H,
      },
      viewport,
      paneWidth,
    });
    if (!next) return;
    // Re-anchor if the operator has panned or zoomed since the last reveal:
    // "where they were before the inspector started moving things" is now their
    // view, not the one we captured two nodes ago.
    const anchor = restoreRef.current;
    const applied = appliedRef.current;
    if (!anchor || !applied || !shouldRestore(viewport, anchor, applied)) {
      restoreRef.current = viewport;
    }
    appliedRef.current = next;
    void setViewport(next, { duration });
  }, [nodeId, paneWidth, duration, getNode, getViewport, setViewport]);

  return null;
}
