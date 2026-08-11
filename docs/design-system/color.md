# Colour

Every colour token, what it is for, and what it measures. Values are declared
in `frontend/src/index.css` in oklch; the hex on each row is the canonical
value for anything outside CSS (Figma, a slide, a favicon).

**Contrast figures are measured**, using WCAG 2.1 relative luminance against
the light canvas `#FCFCFD` or the dark canvas `#0C0C0F`. They are not
estimates. Re-measure after any change:

```js
// hex → linear → relative luminance → ratio
const lin = c => (c/=255, c <= 0.04045 ? c/12.92 : ((c+0.055)/1.055) ** 2.4);
const lum = h => { const [r,g,b] = h.match(/\w\w/g).map(x => lin(parseInt(x,16)));
                   return 0.2126*r + 0.7152*g + 0.0722*b; };
const ratio = (a,b) => { const [x,y] = [lum(a),lum(b)].sort((p,q)=>q-p);
                         return (x+0.05)/(y+0.05); };
```

Targets: **4.5:1** for text, **3:1** for UI marks and large text.

---

## Brand ramp

Indigo. The only hue the product owns. `--brand-*`, addressable as
`bg-brand-500` etc., though components should prefer the semantic names below.

| Token | Hex | Role |
| --- | --- | --- |
| `--brand-50` | `#EEEDFF` | Tint backgrounds |
| `--brand-100` | `#E0DEFF` | Tint backgrounds, borders on brand surfaces |
| `--brand-200` | `#C7C3FF` | Disabled brand fills |
| `--brand-300` | `#A6A0FF` | Dark-mode active nav ink |
| `--brand-400` | `#857EFF` | **Dark-mode accent** — links, focus, primary |
| `--brand-500` | `#635BFF` | **The brand.** Light-mode accent and all filled brand buttons |
| `--brand-600` | `#524AE0` | Pressed state on brand fills |
| `--brand-700` | `#423BBA` | Light-mode active nav ink |
| `--brand-800` | `#322C8F` | High-contrast ink on tint |
| `--brand-900` | `#241F66` | Reserved |

The ramp is theme-independent — 500 is 500 in both themes. What changes is
*which step the accent role points at*: 500 in light, 400 in dark, because 500
is too dense to read as ink on near-black.

| Pair | Ratio | Verdict |
| --- | --- | --- |
| white on `brand-500` | 4.70:1 | AA — filled buttons, both themes |
| `brand-500` on light canvas | 4.58:1 | AA — links |
| `brand-400` on dark canvas | 5.98:1 | AA — links, dark |
| `brand-700` on `brand-50` tint | 7.98:1 | AA — active nav row |

---

## Neutrals

Cool-tinted at ~286°, chroma 0.001–0.03. See
[`../brand/README.md`](../brand/README.md#neutrals-carry-the-brand) for why they
are not pure grey.

| Token | Hex | Used as |
| --- | --- | --- |
| `--gray-25` | `#FCFCFD` | Light canvas |
| `--gray-50` | `#F4F4F7` | Light muted / sidebar; dark foreground |
| `--gray-100` | `#EEEEF3` | Light secondary |
| `--gray-200` | `#E6E6EC` | Light border |
| `--gray-300` | `#D9D9E3` | Light input border |
| `--gray-400` | `#8C8C9E` | Light idle mark |
| `--gray-500` | `#6E6E80` | Light muted text |
| `--gray-600` | `#9797A8` | Dark muted text |
| `--gray-800` | `#23232C` | Dark secondary |
| `--gray-850` | `#1E1E26` | Dark muted |
| `--gray-875` | `#17171D` | Dark popover |
| `--gray-900` | `#16161D` | Light foreground |
| `--gray-925` | `#131318` | Dark card / sidebar |
| `--gray-950` | `#0C0C0F` | Dark canvas |

---

## Semantic surfaces

Layer 2. These are what components use.

| Token | Light | Dark | Role |
| --- | --- | --- | --- |
| `--background` | `gray-25` | `gray-950` | The canvas |
| `--card` | white | `gray-925` | Resting panels |
| `--popover` | white | `gray-875` | Floating surfaces |
| `--muted` | `gray-50` | `gray-850` | Recessed fills, code, skeletons |
| `--secondary` | `gray-100` | `gray-800` | Secondary button fill |
| `--accent` | brand 7% on canvas | brand 14% on muted | Hover/rest tint under rows |
| `--sidebar` | `gray-50` | `gray-925` | Nav column |
| `--sidebar-accent` | brand 10% | brand 16% | Active nav row background |
| `--border` | `gray-200` | white 9% | The hairline |
| `--input` | `gray-300` | white 14% | Field borders, stronger rules |
| `--ring` | `brand-500` | `brand-400` | Focus |

Light surfaces climb *toward* the viewer with lightness (canvas `#FCFCFD` →
card white), and dark does the same (`#0C0C0F` → `#131318` → `#17171D`). A card
lifts off the page before any shadow is applied.

Dark borders are translucent on purpose: they must read against three
different surface lightnesses, and a fixed colour would vanish on one of them.

**`--accent-foreground` stays neutral.** 40 call sites pair `bg-accent` with
`text-accent-foreground`; indigo text on every hover would make the console
strobe. The single place brand ink is spent is `--sidebar-accent-foreground` —
the nav row you are standing on.

### Text on surface

| Token | Light | Dark | Role |
| --- | --- | --- | --- |
| `--foreground` | 17.56:1 | 17.79:1 | Primary reading text |
| `--muted-foreground` | 4.87:1 | 6.80:1 | Secondary, metadata, captions |
| `--primary` | 4.58:1 | 5.98:1 | Links, emphasis |
| `--destructive` | 3.82:1 | 6.73:1 | Error marks (see caveat below) |

`--muted-foreground` also measures 4.55:1 on `--muted`, so caption text on a
recessed fill still passes.

---

## Status

A closed vocabulary of five. Each ships three weights, because one value cannot
both fill a 6px dot and set legible 11px text.

- **`-mark`** — dots, bars, chart fills, icon glyphs. Target 3:1.
- **`-text`** — words. Target 4.5:1. Materially darker in light mode.
- **`-soft`** — the tinted background a badge sits on.

| State | Light mark | Light text | Dark (both) |
| --- | --- | --- | --- |
| **idle** | `#8C8C9E` 3.22:1 | `#6E6E80` 4.87:1 | `#9797A8` 6.80:1 |
| **running** | `#0EA5E9` 2.70:1 | `#0A6E9C` 5.50:1 | `#38BDF8` 9.12:1 |
| **blocked** | `#F5A524` 1.99:1 | `#A16207` 4.80:1 | `#FFC53D` 12.38:1 |
| **done** | `#12A150` 3.28:1 | `#0A7D3E` 5.10:1 | `#35C77F` 8.95:1 |
| **failed** | `#E5484D` 3.82:1 | `#C62A2F` 5.43:1 | `#FF6369` 6.73:1 |

**Read the two low numbers.** Amber at 1.99:1 and cyan at 2.70:1 do not meet
even the 3:1 UI threshold on the light canvas — that is exactly why the split
exists. They are legitimate on a badge's soft background or as a large filled
bar, and they must never set text or paint a lone thin mark on the canvas. Use
`-text` for anything a person reads.

In dark mode `-mark` and `-text` intentionally collapse to the same bright
value: on near-black it clears 4.5:1 on its own, and a separate text weight
would only be dimmer.

**Never rely on colour alone.** Roughly 1 in 12 men cannot separate the
red/green pair. Every status must also carry an icon, a label, or a position.

---

## Identity tones

A categorical palette for **who**, not what state: the tile behind a desk's
initials, a teammate's avatar, a thread's tint, a skill's category, a memory's
kind. Assigned by hash, so a name keeps its colour across reloads and carries
no meaning beyond "not the other one".

| Token | Mark | Light text | Dark text |
| --- | --- | --- | --- |
| `--tone-1` violet | `#8B5CF6` | `#6D28D9` 6.93:1 | `#C4B5FD` 10.58:1 |
| `--tone-2` blue | `#3B82F6` | `#1D4ED8` 6.54:1 | `#93C5FD` 10.83:1 |
| `--tone-3` teal | `#14B8A6` | `#0F766E` 5.34:1 | `#5EEAD4` 13.20:1 |
| `--tone-4` fuchsia | `#D946EF` | `#A21CAF` 6.17:1 | `#F0ABFC` 11.10:1 |
| `--tone-5` slate | `#64748B` | `#475569` 7.39:1 | `#CBD5E1` 13.16:1 |

**No amber, no green, no red.** That is the whole design of this palette.
Identity used to be drawn from the same Tailwind colours as status, which put
the collision the brand doc warns about directly into the product: a desk
keyed `emerald` wore the exact green that means "done", a skill filed under
Finance wore the red that means "failed", and every task-outcome memory looked
like a failed one.

Five rather than eight, because five hues clear of the status vocabulary is
what the hue circle has room for once brand and five states are spoken for. A
hash over five still distributes well.

**Where the hues do come close — violet against brand indigo, blue against
running cyan — form separates them:**

| | Shape | Carries |
| --- | --- | --- |
| **Identity** | A tile with initials | Who |
| **Status** | A pill or dot with a label | What state |

They never take the same shape. That rule is what makes the remaining hue
proximity safe, and it is the first thing to check when adding a component
that shows both at once.

### Legacy slot names

`TEAM_TONES` and the thread `TONES` map are keyed `sky`, `violet`, `amber`,
`emerald`, `rose`, `cyan`, `indigo`, `teal`. Those keys are **persisted
against desks and members and arrive from the host**, so they cannot be
renamed — they name a slot, not a colour. A desk keyed `amber` resolves to
`--tone-5` (slate), and that is correct.

## Charts

| Slot | Light | Dark |
| --- | --- | --- |
| `--chart-1` | `#635BFF` indigo | `#857EFF` |
| `--chart-2` | `#0EA5E9` cyan | `#38BDF8` |
| `--chart-3` | `#12A150` green | `#35C77F` |
| `--chart-4` | `#F5A524` amber | `#FFC53D` |
| `--chart-5` | `#E93D82` pink | `#FF6BA6` |

Brand leads slot 1; the sequence then walks the hue circle so neighbouring
series never collide. The ordering is chosen so the *two-series* case — by far
the most common — gets indigo and cyan, the pair that survives the most common
colour-vision deficiencies.

Chart colours are marks, not text. Axis labels and legends use
`--muted-foreground`, never the series colour.

---

## The knowledge graph

The Overview graph was ported with its own palette vocabulary (`--kg-*`,
plus unprefixed names inside `.oc-kg`). Rather than rename ~2000 lines, those
names are re-pointed at the semantic layer in `index.css`.

Two consequences: the graph now themes for free — it previously carried its own
hardcoded light/dark hex pairs — and the mapping is strictly one-way. Nothing
outside `.oc-kg` may use those names.

`--kg-brain-1` / `--kg-brain-2` stay deliberately outside the status
vocabulary: they identify *which store* a node came from, and colouring them
with status hues would imply a health they do not carry.

---

## Hardcoded colour debt

**Cleared.** Every colour in `src/` now resolves through a token. Verify with:

```sh
cd frontend
grep -rn '\(text\|bg\|border\|ring\|fill\|stroke\)-\(emerald\|rose\|amber\|sky\|red\|green\|blue\|yellow\|violet\|indigo\|teal\|cyan\|slate\)-[0-9]' src --include="*.tsx" --include="*.ts"
grep -rn '#[0-9a-fA-F]\{6\}' src --include="*.tsx" --include="*.ts"
```

The first returns nothing. The second returns only `src/lib/connections.ts`.

### The one file that keeps its hexes

`connections.ts` holds eleven third-party provider brand colours — Gmail's
red, Slack's aubergine, GitHub's near-black. They are correct as literals:
they identify *someone else*, and a themed approximation of Slack's purple
would be wrong in both themes. They are data about a third party, not a design
decision this system gets to make, and the field says so.

Discord's blurple is the same category but appears in markup rather than data,
so it is named: `--brand-discord`, `--brand-discord-hover`,
`--brand-discord-on-dark`. The token name is what stops a future cleanup
"fixing" it into the palette.

Anything drawn on top of a provider colour must not assume a light or dark
ground — they span `#0F0F0F` to `#EA4335`.
