// SPDX-License-Identifier: GPL-3.0-or-later

/**
 * Which node labels the knowledge graph actually draws (issue #1104).
 *
 * The graph used to decide this with one boolean: label `self` and `team`
 * always, label *everything* the moment a pillar was focused. That rule failed
 * at both ends — at rest every agent was an anonymous circle, and in focus a
 * whole tree's worth of names turned on at once and smeared into each other.
 *
 * The replacement is two pure steps, both here:
 *
 * 1. the component nominates **candidates** with a priority (the roster at
 *    rest, the focused node and its direct children in focus, plus whatever is
 *    hovered or selected), alongside the **icons** — every node circle actually
 *    drawn, whether or not it is ever named;
 * 2. {@link planLabels} runs a greedy declutter over them — highest priority
 *    placed first, and any later label whose box overlaps one already placed,
 *    or lands on a node's icon, is dropped. Every node keeps a `<title>`, so a
 *    dropped label still has a native tooltip underneath it.
 *
 * ## Why icons are obstacles too (issue #1258)
 *
 * The first cut of this pass only compared a label against other *labels*, and
 * that is half the picture. The nodes most likely to sit in a text box's path
 * are tools and SOP tasks — numerous, closely packed, and deliberately left
 * unnamed at rest, so they contributed no box of their own to collide with. A
 * label could dodge every other label and still land squarely on a neighbour's
 * circle avatar, which is how "Portfolio Support" came to render as
 * `[icon]folio Support`. Icons are circles with a radius, not boxes, so the
 * test against them is {@link boxHitsCircle}, not {@link overlaps}.
 *
 * ## Why the collision pass measures in SCREEN space
 *
 * Labels are deliberately a constant on-screen size at every camera depth: the
 * camera publishes its zoom as `--kg-cam-k` and each font counter-scales
 * through it. So zooming changes how far apart nodes are, never how wide a
 * label is. Comparing boxes in graph units would therefore get the answer
 * backwards at every depth but one — the same two labels overlap or not
 * depending purely on the camera. {@link planLabels} projects each node through
 * the camera first and measures in pixels.
 *
 * Only the camera's *scale* matters: a pan shifts every box by the same vector
 * and cannot change which pairs overlap, so the caller only has to re-run this
 * when the zoom changes.
 */

/** The live camera rect — the SVG's `viewBox`, in graph units. */
export type LabelCamera = { x: number; y: number; w: number };

export type LabelCandidate = {
  id: string;
  /** the text as rendered (already shortened, if the caller shortens it) */
  text: string;
  /** the node's centre, in graph units */
  x: number;
  y: number;
  /** centre → label baseline, in graph units (radius + row stagger) */
  dy: number;
  /** the label's on-screen font size in px — constant at every camera depth */
  fontPx: number;
  /** higher wins a collision; ties break on the order given */
  priority: number;
};

/**
 * A node's drawn circle avatar — an obstacle no label may sit on (issue #1258).
 *
 * Unlike a label, an icon rides the graph completely: both its centre and its
 * radius are in graph units, so zooming in makes it cover more pixels. `id`
 * matches the {@link LabelCandidate} of the same node, which is how a label is
 * let through its OWN icon; see {@link planLabels}.
 */
export type LabelIcon = {
  id: string;
  /** the node's centre, in graph units */
  x: number;
  y: number;
  /** the drawn radius, in graph units */
  r: number;
};

/**
 * Label priorities, highest first. The bands are far enough apart that a
 * caller can add a small tie-breaker (degree, say) without crossing a band.
 */
export const LABEL_PRIORITY = {
  /** the node under the pointer — the one name the reader just asked for */
  hovered: 1000,
  /** whatever the detail panel is currently showing */
  selected: 900,
  /** the company at the core */
  self: 800,
  /** the node a focused tree is built around */
  focused: 700,
  /** a department: the trunk in focus, the whole ring at rest */
  team: 600,
  /** the focused node's direct children */
  child: 500,
  /** the roster — agents and people — named at rest */
  worker: 400,
} as const;

/**
 * Monospace advance width as a fraction of the font size. The labels are set in
 * `var(--font-mono)` (Geist Mono), whose advance is 0.6em; every glyph is that
 * wide, so a character count is an exact width rather than an estimate.
 */
const MONO_ADVANCE = 0.6;

/**
 * Horizontal breathing room around a label box, in px, so two survivors on the
 * same row never quite touch. There is no vertical counterpart on purpose: the
 * rows a label can sit on are already staggered by a fixed amount, and padding
 * the box past its own line height turns that stagger into a collision and
 * throws away a label that reads perfectly well.
 */
const LABEL_GAP_X = 3;

export type LabelBox = { x0: number; y0: number; x1: number; y1: number };

/**
 * A candidate's label box in screen px, padded by {@link LABEL_GAP_X}.
 * `scale` is px per graph unit (canvas width ÷ camera width).
 */
export function labelBoxPx(c: LabelCandidate, cam: LabelCamera, scale: number): LabelBox {
  const cx = (c.x - cam.x) * scale;
  // `dy` rides the graph, so it scales with the camera; the font does not.
  const baseline = (c.y - cam.y) * scale + c.dy * scale;
  const halfW = (c.text.length * MONO_ADVANCE * c.fontPx) / 2 + LABEL_GAP_X;
  return {
    x0: cx - halfW,
    x1: cx + halfW,
    // a baseline sits ~0.8em below the cap line and ~0.2em above the descender
    y0: baseline - c.fontPx * 0.8,
    y1: baseline + c.fontPx * 0.2,
  };
}

const overlaps = (a: LabelBox, b: LabelBox): boolean =>
  a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;

/**
 * Does a label box touch a circle? All four values are screen px.
 *
 * An icon is a disc, so the box's bounding square would reject placements that
 * clear the drawn circle by a comfortable margin at every corner. The exact
 * test is the cheap one anyway: clamp the centre into the box to get the
 * nearest point on it, then ask whether that point is inside the radius. A
 * centre that lands *inside* the box clamps to itself, distance zero, which is
 * the fully-covered case and reads true as it should.
 */
export function boxHitsCircle(
  box: LabelBox,
  cx: number,
  cy: number,
  r: number,
): boolean {
  if (r <= 0) return false;
  const nx = Math.min(Math.max(cx, box.x0), box.x1);
  const ny = Math.min(Math.max(cy, box.y0), box.y1);
  const dx = cx - nx;
  const dy = cy - ny;
  return dx * dx + dy * dy < r * r;
}

/**
 * The ids whose labels survive the declutter, measured in screen px.
 *
 * `canvasW` is the width the camera rect maps onto — the nominal SVG width, so
 * the px here are the same px `fontPx` is quoted in. Candidates are placed
 * highest priority first; a label that collides with one already placed is
 * dropped rather than nudged, because nudging is what the old two-row stagger
 * did and it only moved the pile-up.
 *
 * `icons` are the node circles actually drawn — pass every visible node, not
 * just the ones eligible for a label, since an unnamed tool hides text exactly
 * as well as a named agent does. Two rules make them behave:
 *
 * - **a label may sit on its own node's icon.** Its box hangs off that very
 *   node by `dy`, which rides the graph while the font does not, so zoomed far
 *   enough out every box overlaps its own circle. Blocking on that would drop
 *   every label at once rather than the one in the way.
 * - **a label with nowhere clear is dropped, not shrunk or drawn anyway.** That
 *   is already what an unwinnable label-vs-label collision does, and the node
 *   still carries a `<title>`, so the name stays one hover away instead of
 *   being rendered illegibly under an avatar.
 */
export function planLabels(
  candidates: readonly LabelCandidate[],
  cam: LabelCamera,
  canvasW: number,
  icons: readonly LabelIcon[] = [],
): Set<string> {
  const scale = cam.w > 0 ? canvasW / cam.w : 1;
  const order = candidates.map((c, i) => ({ c, i }));
  order.sort((a, b) => b.c.priority - a.c.priority || a.i - b.i);
  // icons never move or drop out, so project them once rather than per label
  const discs = icons.map((n) => ({
    id: n.id,
    cx: (n.x - cam.x) * scale,
    cy: (n.y - cam.y) * scale,
    // a radius rides the graph, so unlike `fontPx` it scales with the camera
    r: n.r * scale,
  }));
  const placed: LabelBox[] = [];
  const kept = new Set<string>();
  for (const { c } of order) {
    const box = labelBoxPx(c, cam, scale);
    if (placed.some((p) => overlaps(p, box))) continue;
    if (discs.some((d) => d.id !== c.id && boxHitsCircle(box, d.cx, d.cy, d.r))) continue;
    placed.push(box);
    kept.add(c.id);
  }
  return kept;
}

/**
 * The focused node and its direct children in a focused tree — the only nodes
 * guaranteed a label there. `branches` is the tree's edge list; a child is any
 * branch whose source is the focused node.
 *
 * Focus follows what was clicked: a pillar names its tasks, an agent names its
 * tools. Everything else in the tree is one hover (or one native tooltip) away.
 */
export function focusLabelIds(
  branches: readonly { source: string; target: string }[],
  focusId: string | null,
): Set<string> {
  const set = new Set<string>();
  if (!focusId) return set;
  set.add(focusId);
  for (const b of branches) if (b.source === focusId) set.add(b.target);
  return set;
}
