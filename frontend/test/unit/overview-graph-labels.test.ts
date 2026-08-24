import { describe, expect, it } from "vitest";

import {
  boxHitsCircle,
  focusLabelIds,
  LABEL_PRIORITY,
  labelBoxPx,
  planLabels,
  type LabelCandidate,
  type LabelIcon,
  type LabelPlan,
} from "@/views/overview/kg/label-plan";

/**
 * The Overview graph's label declutter (issues #1104 and #1258).
 *
 * The rule this replaced was one boolean, and it failed at both ends: at rest
 * only the company and its departments were named, so every agent was an
 * anonymous circle; the moment a pillar was focused, every node in the tree was
 * named at once and the names smeared into each other.
 *
 * Four properties are worth guarding here, and none is visible from a
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
 * 3. **Node icons are obstacles, not just other labels** (#1258). The circles
 *    most likely to sit on a name belong to tools and SOP tasks, which are
 *    never named at rest and so contributed no box of their own — a label could
 *    clear every other label and still render as `[icon]folio Support`.
 * 4. **The icon pass does not over-correct.** A label may sit on its OWN node's
 *    icon (it hangs off that very circle, so at low zoom it always overlaps
 *    it); a label an icon blocks is offered the mirrored row above its node
 *    before it is given up on; and one that fits nowhere is dropped rather than
 *    drawn illegibly. Without the mirror this pass takes a company's own name
 *    off the canvas rather than moving it 20px — a worse bug than the one it
 *    fixes. The mirror is offered ONLY against an icon: property 1 is untouched,
 *    because two labels contending for one row are the same width and moving
 *    one up relocates the contention rather than settling it.
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

/** the ids a plan kept, in the order it placed them */
const ids = (plan: LabelPlan): string[] => [...plan.keys()];
/** the ids a plan kept, order-insensitive */
const idSet = (plan: LabelPlan): Set<string> => new Set(plan.keys());

describe("planLabels", () => {
  it("drops the lower-priority label of a colliding pair", () => {
    const plan = planLabels(
      [
        cand({ id: "quiet", x: 0, priority: LABEL_PRIORITY.worker }),
        cand({ id: "hovered", x: 28, priority: LABEL_PRIORITY.hovered }),
      ],
      { x: 0, y: 0, w: W },
      W,
    );
    expect(ids(plan)).toEqual(["hovered"]);
  });

  it("keeps both when the boxes clear each other", () => {
    const plan = planLabels(
      [
        cand({ id: "left", x: 0, priority: LABEL_PRIORITY.worker }),
        cand({ id: "right", x: 32, priority: LABEL_PRIORITY.hovered }),
      ],
      { x: 0, y: 0, w: W },
      W,
    );
    expect(idSet(plan)).toEqual(new Set(["left", "right"]));
  });

  it("measures in screen space, so zooming out drops a label the same graph gap kept", () => {
    const pair = [
      cand({ id: "left", x: 0, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "right", x: 32, priority: LABEL_PRIORITY.worker }),
    ];
    // camera width === canvas width: one graph unit is one px, both fit
    expect(idSet(planLabels(pair, { x: 0, y: 0, w: W }, W))).toEqual(new Set(["left", "right"]));
    // pulled back to half scale the nodes are 16px apart while the labels are
    // still 24px wide — in graph units nothing changed at all
    expect(ids(planLabels(pair, { x: 0, y: 0, w: W * 2 }, W))).toEqual(["left"]);
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

  it("costs task labels at their full rendered width", () => {
    const neighbour = cand({ id: "neighbour", x: 80, priority: LABEL_PRIORITY.worker });
    const short = cand({ id: "named", x: 0, text: "A", priority: LABEL_PRIORITY.hovered });
    const long = {
      ...short,
      text: "Draft Q3 pricing experiment",
    };
    expect(idSet(planLabels([short, neighbour], { x: 0, y: 0, w: W }, W))).toEqual(
      new Set(["named", "neighbour"]),
    );
    expect(ids(planLabels([long, neighbour], { x: 0, y: 0, w: W }, W))).toEqual(["named"]);
  });

  it("accounts for the 10px floor when deciding which dense labels survive", () => {
    const pair = [
      cand({ id: "priority", x: 0, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "neighbour", x: 29, priority: LABEL_PRIORITY.worker }),
    ];
    // The former 9px label boxes fit at this gap. At the design system floor,
    // the planner drops the quieter one instead of letting the rendered names
    // overlap.
    expect(idSet(planLabels(pair.map((c) => ({ ...c, fontPx: 9 })), { x: 0, y: 0, w: W }, W))).toEqual(
      new Set(["priority", "neighbour"]),
    );
    expect(ids(planLabels(pair, { x: 0, y: 0, w: W }, W))).toEqual(["priority"]);
  });

  it("separates labels that share an x but sit on different rows", () => {
    const stacked = [
      cand({ id: "row-0", x: 0, dy: 20, priority: LABEL_PRIORITY.hovered }),
      cand({ id: "row-1", x: 0, dy: 40, priority: LABEL_PRIORITY.worker }),
    ];
    expect(idSet(planLabels(stacked, { x: 0, y: 0, w: W }, W))).toEqual(
      new Set(["row-0", "row-1"]),
    );
  });

  it("reports where each survivor is drawn, so the renderer cannot guess wrong", () => {
    const plan = planLabels([cand({ id: "named", dy: 26 })], { x: 0, y: 0, w: W }, W);
    expect(plan.get("named")).toBe(26);
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
  // x -15..15, y 12..22 — the numbers every case below is placed against. Its
  // mirror is dy -20: the box x -15..15, y -28..-18.
  const named = cand({ id: "named", priority: LABEL_PRIORITY.worker });

  it("moves a label off an unnamed neighbour's icon", () => {
    // the tool sits squarely in the middle of the word — no other LABEL is
    // anywhere near, which is exactly why the old pass let this through
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, []).get("named")).toBe(20);
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [tool]).get("named")).toBe(-20);
  });

  it("drops a label boxed in on both rows", () => {
    // the whole column is spoken for, so there is no honest place to put the
    // name: it goes, and the node's <title> carries it instead
    const below = icon({ id: "below", x: 0, y: 20, r: 7 });
    const above = icon({ id: "above", x: 0, y: -23, r: 7 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [below, above]).size).toBe(0);
  });

  it("lets a label through its own node's icon", () => {
    // one circle, two readings: as the label's own node it is exempt, as
    // anyone else's it blocks. r 30 reaches both rows from the node's centre,
    // so as a stranger's circle there is nowhere for the name to mirror to.
    const own = icon({ id: "named", x: 0, y: 0, r: 30 });
    expect(planLabels([named], { x: 0, y: 0, w: W }, W, [own]).get("named")).toBe(20);
    expect(
      planLabels([named], { x: 0, y: 0, w: W }, W, [{ ...own, id: "someone-else" }]).size,
    ).toBe(0);
  });

  it("keeps the self exemption load-bearing at every depth", () => {
    // zoomed out, `dy` has shrunk with the graph while the font has not, so
    // the box straddles its own node whatever that node's radius is. Without
    // the exemption this would drop every label at once rather than the one in
    // the way — the failure would read as "labels stopped working when I
    // zoomed out", not as a collision bug.
    const own = icon({ id: "named", x: 0, y: 0, r: 7 });
    expect(idSet(planLabels([named], { x: 0, y: 0, w: W * 4 }, W, [own]))).toEqual(
      new Set(["named"]),
    );
  });

  it("gives no label a way through an icon, however high its priority", () => {
    // an icon is not a competitor that can lose a tie — it is drawn either
    // way, so the hovered name would render underneath it. Priority buys the
    // first pick of the rows, never a pass through one that is taken.
    const hovered = cand({ id: "hovered", priority: LABEL_PRIORITY.hovered });
    const below = icon({ id: "below", x: 0, y: 20, r: 7 });
    const above = icon({ id: "above", x: 0, y: -23, r: 7 });
    expect(planLabels([hovered], { x: 0, y: 0, w: W }, W, [below, above]).size).toBe(0);
  });

  it("costs a blocked label nothing but itself", () => {
    const blocked = cand({ id: "blocked", x: 0, priority: LABEL_PRIORITY.hovered });
    const clear = cand({ id: "clear", x: 200, priority: LABEL_PRIORITY.worker });
    const below = icon({ id: "below", x: 0, y: 20, r: 7 });
    const above = icon({ id: "above", x: 0, y: -23, r: 7 });
    expect(idSet(planLabels([blocked, clear], { x: 0, y: 0, w: W }, W, [below, above]))).toEqual(
      new Set(["clear"]),
    );
  });

  it("offers the mirror only when an icon is the blocker", () => {
    // #1104's rule is untouched: two labels contending for one row are the same
    // width, so moving one up relocates the contention instead of settling it.
    // That was the old two-row stagger, and it is why the loser still just goes.
    const first = cand({ id: "first", x: 0, priority: LABEL_PRIORITY.hovered });
    const second = cand({ id: "second", x: 20, priority: LABEL_PRIORITY.worker });
    expect(ids(planLabels([first, second], { x: 0, y: 0, w: W }, W))).toEqual(["first"]);
  });

  it("will not mirror onto a label already placed there", () => {
    // the row above is not a free parking space: the first candidate owns it,
    // so the second has to give up rather than stack on top of it
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    const first = cand({ id: "first", x: 0, dy: -20, priority: LABEL_PRIORITY.hovered });
    const second = cand({ id: "second", x: 0, dy: 20, priority: LABEL_PRIORITY.worker });
    expect(ids(planLabels([first, second], { x: 0, y: 0, w: W }, W, [tool]))).toEqual(["first"]);
  });

  it("measures icons in screen space too, so zooming out drops what 1:1 kept", () => {
    // a radius RIDES the graph, unlike the font, so it must be projected
    // through the camera rather than compared against px as-is. 30 units above
    // is clear at 1:1; pulled back to a tenth, the box is still 10px tall while
    // that gap has collapsed to 3px and the icon is inside the word — on both
    // rows at once, since at that depth the two rows are 4px apart.
    const above = icon({ id: "tool", x: 0, y: -30, r: 6 });
    expect(idSet(planLabels([named], { x: 0, y: 0, w: W }, W, [above]))).toEqual(
      new Set(["named"]),
    );
    expect(planLabels([named], { x: 0, y: 0, w: W * 10 }, W, [above]).size).toBe(0);
  });

  it("frees the row it vacates, so a mirror can let a quieter label in", () => {
    // Not obvious, and worth pinning rather than discovering: this pass is not
    // monotone in the number of labels. `loud` would sit at dy 20 and take the
    // row `quiet` wanted; blocked by an icon it mirrors instead, and `quiet`
    // now fits the row it just left. Observed on a software company, where the
    // icon pass cost four names and handed back "QA Engineer".
    const loud = cand({ id: "loud", x: 0, priority: LABEL_PRIORITY.hovered });
    const quiet = cand({ id: "quiet", x: 25, priority: LABEL_PRIORITY.worker });
    // no icon: they contend for one row and the quieter one loses outright
    expect(ids(planLabels([loud, quiet], { x: 0, y: 0, w: W }, W))).toEqual(["loud"]);
    // an icon inside `loud`'s row but clear of `quiet`'s sends `loud` upstairs
    const tool = icon({ id: "tool", x: -10, y: 17, r: 5 });
    const plan = planLabels([loud, quiet], { x: 0, y: 0, w: W }, W, [tool]);
    expect(plan.get("loud")).toBe(-20);
    expect(plan.get("quiet")).toBe(20);
  });

  it("stays pan-invariant, the property that lets the caller cache this", () => {
    // the module only re-runs the pass when the ZOOM changes, on the grounds
    // that a pan moves every box by one shared vector. Icons have to move with
    // them, or a graph you dragged would silently label itself differently.
    const tool = icon({ id: "tool", x: 0, y: 20, r: 7 });
    const panned = planLabels([named], { x: -400, y: 250, w: W }, W, [tool]);
    // pinned so the comparison cannot pass by both sides dropping everything
    expect(panned.get("named")).toBe(-20);
    expect(panned).toEqual(planLabels([named], { x: 0, y: 0, w: W }, W, [tool]));
  });

  it("ignores an icon the caller left out, so hidden nodes never block", () => {
    // the caller only nominates circles it actually draws; a carousel node
    // faded to nothing must not take a name off a node you can see
    expect(idSet(planLabels([named], { x: 0, y: 0, w: W }, W))).toEqual(new Set(["named"]));
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

  it("mirrors the box when handed the mirrored row", () => {
    const b = labelBoxPx(cand({ id: "x" }), { x: 0, y: 0, w: W }, 1, -20);
    expect(b).toEqual({ x0: -15, x1: 15, y0: -28, y1: -18 });
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
