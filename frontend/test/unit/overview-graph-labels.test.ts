import { describe, expect, it } from "vitest";

import {
  boxHitsCircle,
  focusLabelIds,
  LABEL_PRIORITY,
  labelBoxPx,
  planLabels,
  type LabelCandidate,
  type LabelIcon,
} from "@/views/overview/kg/label-plan";

/**
 * The Overview graph's label declutter (issue #1104).
 *
 * The rule this replaced was one boolean, and it failed at both ends: at rest
 * only the company and its departments were named, so every agent was an
 * anonymous circle; the moment a pillar was focused, every node in the tree was
 * named at once and the names smeared into each other.
 *
 * Two properties are worth guarding here, and neither is visible from a
 * screenshot:
 *
 * 1. **Priority decides who survives a collision.** Silent failure: the label
 *    you are pointing at loses to a sibling that happened to be nominated
 *    first, and the graph looks like hover simply does nothing.
 * 2. **The overlap is measured in SCREEN space.** Labels hold one on-screen
 *    size at every camera depth (`fixedLabel` counter-scales through
 *    `--kg-cam-k`), so graph units answer this question correctly at exactly
 *    one zoom level and wrongly everywhere else — and wrongly in the direction
 *    that matters, because zooming out is what packs nodes together. That
 *    regression would restore the pile-up this issue is about while every unit
 *    of the layout still looked right.
 * 3. **Node icons are obstacles, not just other labels** (issue #1258). The
 *    circles most likely to sit on a name belong to tools and SOP tasks, which
 *    are never named at rest and so contributed no box of their own — a label
 *    could clear every other label and still render as `[icon]folio Support`.
 *    The two rules that keep that pass from over-correcting are worth pinning
 *    down too: a label may sit on its OWN node's icon (it hangs off that very
 *    circle, so at low zoom it always overlaps it), and a label with nowhere
 *    clear is dropped rather than drawn illegibly.
 */

const W = 880;

const cand = (over: Partial<LabelCandidate> & { id: string }): LabelCandidate => ({
  text: "AAAA",
  x: 0,
  y: 0,
  dy: 20,
  fontPx: 10,
  priority: LABEL_PRIORITY.worker,
  ...over,
});

describe("planLabels", () => {
  it("drops the lower-priority label of a colliding pair", () => {
    const kept = planLabels(
      [
        cand({ id: "quiet", x: 0, priority: LABEL_PRIORITY.worker }),
        cand({ id: "hovered", x: 28, priority: LABEL_PRIORITY.hovered }),
      ],
      { x: 0, y: 0, w: W },
      W,
    );
    expect([...kept]).toEqual(["hovered"]);
  });

  it("keeps both when the boxes clear each other", () => {
    const kept = planLabels(
      [
        cand({ id: "left", x: 0, priority: LABEL_PRIORITY.worker }),
        cand({ id: "right", x: 32, priority: LABEL_PRIORITY.hovered }),
      ],
      { x: 0, y: 0, w: W },
      W,
    );
    expect(kept).toEqual(new Set(["left", "right"]));
  });

  it("measures in screen space, so zooming out drops a label the same graph gap kept", () => {
    const pair = [
      cand({ id: "left", x: 0, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "right", x: 32, priority: LABEL_PRIORITY.worker }),
    ];
    // camera width === canvas width: one graph unit is one px, both fit
    expect(planLabels(pair, { x: 0, y: 0, w: W }, W)).toEqual(new Set(["left", "right"]));
    // pulled back to half scale the nodes are 16px apart while the labels are
    // still 24px wide — in graph units nothing changed at all
    expect([...planLabels(pair, { x: 0, y: 0, w: W * 2 }, W)]).toEqual(["left"]);
  });

  it("panning the camera never changes the outcome", () => {
    const pair = [
      cand({ id: "left", x: 0, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "right", x: 32, priority: LABEL_PRIORITY.worker }),
    ];
    expect(planLabels(pair, { x: -400, y: 250, w: W }, W)).toEqual(
      planLabels(pair, { x: 0, y: 0, w: W }, W),
    );
  });

  it("collides on the rendered width, so a long name costs its neighbour", () => {
    const neighbour = cand({ id: "neighbour", x: 100, priority: LABEL_PRIORITY.worker });
    const short = cand({ id: "named", x: 0, text: "A", priority: LABEL_PRIORITY.hovered });
    const long = { ...short, text: "A".repeat(40) };
    expect(planLabels([short, neighbour], { x: 0, y: 0, w: W }, W)).toEqual(
      new Set(["named", "neighbour"]),
    );
    expect([...planLabels([long, neighbour], { x: 0, y: 0, w: W }, W)]).toEqual(["named"]);
  });

  it("separates labels that share an x but sit on different rows", () => {
    const stacked = [
      cand({ id: "row-0", x: 0, dy: 20, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "row-1", x: 0, dy: 40, priority: LABEL_PRIORITY.worker }),
    ];
    expect(planLabels(stacked, { x: 0, y: 0, w: W }, W)).toEqual(new Set(["row-0", "row-1"]));
  });
});

describe("planLabels vs node icons (issue #1258)", () => {
  const icon = (over: Partial<LabelIcon> & { id: string }): LabelIcon => ({
    x: 0,
    y: 0,
    r: 7,
    ...over,
  });

  // "AAAA" at fontPx 10 hanging on dy 20, at one px per graph unit, is the box
  // x -15..15, y 12..22 — the numbers every case below is placed against.
  const named = cand({ id: "named", priority: LABEL_PRIORITY.worker });

  it("drops a label that lands on an unnamed neighbour's icon", () => {
    // the tool sits squarely in the middle of the word — no other LABEL is
    // anywhere near, which is exactly why the old pass let this through
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [])).toEqual(new Set(["named"]));
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [tool]).size).toBe(0);
  });

  it("lets a label through its own node's icon", () => {
    // one circle, two readings: as the label's own node it is exempt, as
    // anyone else's it blocks. r 18 reaches the box from the node's centre.
    const own = icon({ id: "named", x: 0, y: 0, r: 18 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [own])).toEqual(new Set(["named"]));
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [{ ...own, id: "someone-else" }]).size).toBe(
      0,
    );
  });

  it("keeps the self exemption load-bearing at every depth", () => {
    // zoomed out, `dy` has shrunk with the graph while the font has not, so
    // the box straddles its own node whatever that node's radius is. Without
    // the exemption this would drop every label at once rather than the one in
    // the way — the failure would read as "labels stopped working when I
    // zoomed out", not as a collision bug.
    const own = icon({ id: "named", x: 0, y: 0, r: 7 });
    expect(planLabels([named], { x: 0, y: 0, w: W * 4 }, W, [own])).toEqual(new Set(["named"]));
  });

  it("gives no label a way through an icon, however high its priority", () => {
    // an icon is not a competitor that can lose a tie — it is drawn either
    // way, so the hovered name would render underneath it
    const hovered = cand({ id: "hovered", priority: LABEL_PRIORITY.hovered });
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    expect(planLabels([hovered], { x: 0, y: 0, w: W }, W, [tool]).size).toBe(0);
  });

  it("drops the blocked label without disturbing the ones that fit", () => {
    // the no-valid-placement policy is 'drop', and it is local: a name with
    // nowhere clear costs only itself
    const blocked = cand({ id: "blocked", x: 0, priority: LABEL_PRIORITY.hovered });
    const clear = cand({ id: "clear", x: 200, priority: LABEL_PRIORITY.worker });
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    expect(planLabels([blocked, clear], { x: 0, y: 0, w: W }, W, [tool])).toEqual(
      new Set(["clear"]),
    );
  });

  it("measures icons in screen space too, so zooming out drops what 1:1 kept", () => {
    // a radius RIDES the graph, unlike the font, so it must be projected
    // through the camera rather than compared against px as-is. 30 units clear
    // at 1:1; pulled back to a tenth, the box is still 10px tall while that
    // gap has collapsed to 3px and the icon is inside the word.
    const above = icon({ id: "tool", x: 0, y: -30, r: 6 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [above])).toEqual(new Set(["named"]));
    expect(planLabels([named], { x: 0, y: 0, w: W * 10 }, W, [above]).size).toBe(0);
  });

  it("ignores an icon the caller left out, so hidden nodes never block", () => {
    // the caller only nominates circles it actually draws; a carousel node
    // faded to nothing must not take a name off a node you can see
    expect(planLabels([named], { x: 0, y: 0, w: W }, W)).toEqual(new Set(["named"]));
  });
});

describe("boxHitsCircle", () => {
  const box = { x0: 0, y0: 0, x1: 10, y1: 10 };

  it("is a disc test, not a bounding-box one", () => {
    // (13,13) is 4.24 from the corner: its bounding SQUARE overlaps the box at
    // r 4, the circle itself does not. Getting this wrong throws away labels
    // that clear the drawn avatar with room to spare.
    expect(boxHitsCircle(box, 13, 13, 4)).toBe(false);
    expect(boxHitsCircle(box, 13, 13, 5)).toBe(true);
  });

  it("catches a circle that swallows the box outright", () => {
    expect(boxHitsCircle(box, 5, 5, 1)).toBe(true);
    expect(boxHitsCircle(box, 5, 5, 100)).toBe(true);
  });

  it("treats touching as clear, matching the label-vs-label rule", () => {
    expect(boxHitsCircle(box, 15, 5, 5)).toBe(false);
    expect(boxHitsCircle(box, 15, 5, 5.01)).toBe(true);
  });

  it("says no to a zero or negative radius", () => {
    expect(boxHitsCircle(box, 5, 5, 0)).toBe(false);
    expect(boxHitsCircle(box, 5, 5, -3)).toBe(false);
  });

  it("agrees with the box the planner actually measures", () => {
    // guards the two staying in the same units: labelBoxPx is px, so the
    // circle handed to boxHitsCircle has to be projected the same way
    const b = labelBoxPx(cand({ id: "x" }), { x: 0, y: 0, w: W }, 1);
    expect(b).toEqual({ x0: -15, x1: 15, y0: 12, y1: 22 });
    expect(boxHitsCircle(b, 0, 20, 7)).toBe(true);
    expect(boxHitsCircle(b, 0, 0, 7)).toBe(false);
  });
});

describe("focusLabelIds", () => {
  const branches = [
    { source: "self", target: "team" },
    { source: "team", target: "task-a" },
    { source: "team", target: "task-b" },
    { source: "task-a", target: "worker" },
    { source: "worker", target: "tool" },
  ];

  it("names the focused node and its direct children, and stops there", () => {
    expect(focusLabelIds(branches, "team")).toEqual(new Set(["team", "task-a", "task-b"]));
  });

  it("follows what was clicked: an agent names its tools, not its pillar's tasks", () => {
    expect(focusLabelIds(branches, "worker")).toEqual(new Set(["worker", "tool"]));
  });

  it("names nothing when nothing is focused", () => {
    expect(focusLabelIds(branches, null).size).toBe(0);
  });
});
