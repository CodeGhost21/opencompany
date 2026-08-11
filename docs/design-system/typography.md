# Typography

**Geist Variable** for everything the operator reads. A mono face for values
that change in place. Three weights. One scale.

---

## The scale

Tailwind's own steps are untouched — 460 call sites already depend on them.
What the system *adds* is the two rungs below `xs` that this console genuinely
needs and had been spelling as arbitrary values.

| Class | Size | Line height | Tracking | Use |
| --- | --- | --- | --- | --- |
| `text-3xs` | 10px | 14px | +0.01em | Table meta, graph node labels, badge counters |
| `text-2xs` | 11px | 16px | +0.005em | Captions, timestamps, key/value rows, sidebar section headers |
| `text-xs` | 12px | 16px | — | Dense body — the console's workhorse |
| `text-sm` | 14px | 20px | — | Default body, form labels, buttons |
| `text-base` | 16px | 24px | — | Long-form prose, empty-state copy |
| `text-lg` | 18px | 28px | — | Card titles |
| `text-xl` | 20px | 28px | — | Section headings |
| `text-2xl` | 24px | 32px | — | View titles |

`text-3xs` and `text-2xs` are defined in the `@theme` block of `index.css`.
Both carry slight positive tracking: below 12px, default spacing closes up and
legibility drops faster than size alone predicts.

**This scale starts lower than most products', on purpose.** 11px appears 109
times in this codebase and 10px 50 times. Those are not one-off exceptions to
be stamped out — they are the two densest rungs of the real scale, and the
system's job was to name them, not to deny them.

**Below 10px is not a size, it is a bug.** Nothing smaller is defined. See
[sizes below the scale](#sizes-below-the-scale).

---

## Weights

| Weight | Class | Use |
| --- | --- | --- |
| Normal (400) | `font-normal` | Body text |
| Medium (500) | `font-medium` | Labels, buttons, active nav, table headers |
| Semibold (600) | `font-semibold` | Headings |

Bold (700) is **not** in the system. Where you want more emphasis than
Semibold, the answer is size, colour, or position — not weight.

---

## The mono face

Mono is for **values that change in place**: run ids, durations, token counts,
timestamps, byte sizes, diff hunks.

The reason is mechanical rather than stylistic. A proportional `1` is narrower
than a `8`, so a live counter reflows its row on every tick. Mono, plus
`font-variant-numeric: tabular-nums`, holds the column still.

Prose is never mono. A paragraph of explanatory text in mono is a style choice
this product does not make.

```
--font-mono: "Geist Mono Variable", ui-monospace, "SF Mono", "Menlo", monospace;
```

> **Not yet installed.** Only `@fontsource-variable/geist` (sans) is a
> dependency, so the stack currently falls through to the platform mono — which
> is legible and correctly tabular, but is not Geist. Installing
> `@fontsource-variable/geist-mono` and adding one `@import` to `index.css` is
> the whole fix; the token already names it first.

`tabular-nums` is applied automatically to `table` elements and to anything
carrying `data-numeric`, set once in the base layer rather than per component.

---

## Sizes below the scale

15 sites currently set type under 10px. At these sizes glyphs lose their
distinguishing features and antialiasing does the rest — this is
unreadability, not density. Each should be raised to `text-3xs`.

| File | Line | Current |
| --- | --- | --- |
| `views/chat/MessageRow.tsx` | 247 | `text-[7px]` |
| `views/overview/kg/KnowledgeDetail.tsx` | 22 | `text-[8px]` |
| `views/overview/kg/KnowledgeGraphFullscreen.tsx` | 94 | `text-[8.5px]` |
| `components/workflow-node.tsx` | 54 | `text-[9px]` |
| `views/chat/ChannelRail.tsx` | 182 | `text-[9px]` |
| `views/overview/kg/KnowledgeDetail.tsx` | 61, 86, 301 | `text-[9px]` |
| `views/overview/kg/KnowledgeDetail.tsx` | 48, 84, 242, 306, 348 | `text-[9.5px]` |
| `views/overview/kg/KnowledgeGraph.tsx` | 1496, 1506 | `text-[9.5px]` |

The knowledge-graph cluster is the bulk of it. Those labels sit on a zoomable
canvas, so the honest fix is to raise the base size and let zoom carry the
density — not to keep shrinking type the viewport cannot resolve.

---

## Migration

192 arbitrary font sizes across the console. The mapping is mechanical for 159
of them; 33 need a judgement call.

### Mechanical — 159 sites

| Find | Replace | Sites |
| --- | --- | --- |
| `text-[11px]` | `text-2xs` | 109 |
| `text-[10px]` | `text-3xs` | 50 |

These are exact matches to the new rungs. A find-and-replace is safe and
changes nothing visually.

```sh
cd frontend
grep -rl 'text-\[11px\]' src --include="*.tsx" | xargs sed -i '' 's/text-\[11px\]/text-2xs/g'
grep -rl 'text-\[10px\]' src --include="*.tsx" | xargs sed -i '' 's/text-\[10px\]/text-3xs/g'
```

Run `npm run typecheck && npm run build` afterwards, then compare
`#/styleguide` and a couple of dense views against a screenshot taken before.

### Judgement needed — 33 sites

| Current | Sites | Where | Suggested |
| --- | --- | --- | --- |
| `text-[10.5px]` | 13 | almost all `kg/KnowledgeDetail.tsx` | `text-3xs` (10px) — half-pixel type does not render as a half pixel |
| `text-[9.5px]` | 7 | `kg/*` | `text-3xs` — see above |
| `text-[9px]` | 5 | `kg/*`, `workflow-node`, `ChannelRail` | `text-3xs` |
| `text-[12.5px]` | 2 | `kg/KnowledgeDetail.tsx`, `KnowledgeGraphFullscreen.tsx` | `text-xs` (12px) |
| `text-[8px]`, `text-[8.5px]`, `text-[7px]` | 3 | `kg/*`, `MessageRow.tsx` | `text-3xs` |
| `text-[11.5px]` | 1 | `kg/KnowledgeDetail.tsx` | `text-2xs` (11px) |
| `text-[13px]` | 1 | `tour/TourTooltip.tsx` | `text-sm` (14px) or `text-xs` |
| `text-[15px]` | 1 | `tour/TourTooltip.tsx` | `text-base` (16px) |

Half-pixel sizes are the clearest signal of drift: they were arrived at by
nudging until something looked right, and the browser rounds them anyway. All
of them collapse onto a real rung.

`TourTooltip` is the one genuinely separate case — it is onboarding copy at
prose sizes, so it should sit on `text-sm`/`text-base` rather than the dense
console rungs.

### Order to do it in

1. The two mechanical replacements (159 sites, zero visual change).
2. `kg/KnowledgeDetail.tsx` — 21 of the remaining 33 are in this one file.
3. `TourTooltip.tsx` — 2 sites, prose sizes.
4. Everything else, ad hoc, as files are touched for other reasons.

Steps 1 and 2 remove 90% of the debt and can each be a single small commit.
