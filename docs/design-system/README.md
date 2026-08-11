# OpenCompany design system

The implementation contract for the console's visual layer. The *why* lives in
[`docs/brand/README.md`](../brand/README.md); this directory is the *what*.

| Document | Covers |
| --- | --- |
| [`color.md`](color.md) | Every colour token, its role, and its measured contrast |
| [`typography.md`](typography.md) | The type scale, mono policy, and the 192-site migration list |
| [`components.md`](components.md) | Anatomy and required states for each shipped primitive |

**Source of truth:** `frontend/src/index.css`. These documents describe it; if
they ever disagree, the stylesheet wins and the document is a bug.

**Living reference:** run the console and open `#/styleguide`. It renders every
token by reading the variables at runtime, so it cannot drift from the
stylesheet — and it needs no host, company, or sign-in.

---

## The one rule

**Components may only use layer 3.**

The stylesheet is three layers, and nothing skips one:

```
1. PRIMITIVES   --brand-500, --gray-200, --green-mark
                Raw ramps. Theme-independent. Never referenced by a component.
                ↓
2. SEMANTICS    --primary, --border, --status-running
                What a colour means here. Light in :root, dark in .dark.
                ↓
3. UTILITIES    bg-primary, text-status-done-text, shadow-lg
                Tailwind classes. This is all a component may touch.
```

A component that reaches past layer 3 — into a ramp, or into an arbitrary
value like `text-[11px]` or `bg-[#5865f2]` — has made a decision the system
cannot see, cannot theme, and cannot change later. That is the entire failure
mode this structure exists to prevent.

**When the token you need does not exist, add it to layer 2.** Do not
approximate with a near-miss and do not inline a raw value. Naming the need is
the work.

---

## Anti-patterns, and what to do instead

| Instead of | Use | Why |
| --- | --- | --- |
| `text-[11px]` | `text-2xs` | 11px is a real rung of the scale; it now has a name |
| `text-[9px]` | `text-3xs` (10px) | Below 10px is illegible, not dense |
| `bg-[#5865f2]` | a named brand token | A raw hex cannot theme |
| `text-green-600` (Tailwind palette) | `text-status-done-text` | Palette colours carry no meaning and are untuned for this canvas |
| `shadow-lg` on a resting card | `border` + surface lightness | Elevation means "floats above the page" |
| A new accent hue | the existing indigo | There is one accent |
| `text-status-done` for a *label* | `text-status-done-text` | Mark weights measure 3:1 — enough for a dot, not for words |

---

## Changing a token

1. **Change it in `index.css`**, in the semantic layer. Primitives change only
   when the brand itself changes.
2. **Check both themes at `#/styleguide`.** Dark is not a filter over light;
   several tokens are independently tuned.
3. **Re-measure contrast if it is a text or status colour.** The ratios quoted
   in [`color.md`](color.md) are measured, not estimated, and a change makes
   them stale. The helper used to produce them is described there.
4. **Update the affected document in this directory.**

Because every component reads layer 2, a correct token change propagates
everywhere at once — that is the payoff for the indirection.

---

## The Figma library

**File:** [OpenCompany Design System](https://www.figma.com/design/bUj8Ofz2EQL6Y8DU06zbDR)

Generated from these tokens, not drawn by hand. Every swatch, specimen and
component is **bound to a variable** — change a token and the documentation
pages redraw themselves. A foundations page painted with hex literals is a
screenshot that lies the moment the system moves.

| Collection | Variables | Contents |
| --- | --- | --- |
| `Primitives` | 39 | Brand ramp, neutral ramp, status hues. Scopes are `[]` — hidden from every picker, so designers must pick a semantic. |
| `Color · Light` | 32 | Semantic roles, each an **alias** to a primitive. No raw hex. |
| `Color · Dark` | 32 | Same names, independently tuned values. |
| `Scale` | 22 | Radius, spacing, and the font-size ramp. |

Plus **17 text styles** (`Body/`, `Label/`, `Heading/`, `Mono/`) with font size
bound to the `Scale` variables, and **12 effect styles** (`Elevation/` and
`Elevation Dark/`).

Every variable carries an explicit scope and a **WEB code syntax** naming the
real CSS variable — `color/status/running` reports as `var(--status-running)`
— so Dev Mode round-trips to this codebase rather than to an invented name.

### Regenerating it

The library is a **build output**. [`figma-plugin/`](../../figma-plugin/) holds
a Figma development plugin that writes the whole thing — variables, styles,
pages, components — from the same token values. Import it once via Figma
desktop → Plugins → Development → Import plugin from manifest, then run it
whenever a token moves.

It exists because the hosted MCP server is capped at **6 tool calls per month**
on a Starter plan with a View seat. A dev plugin runs the same Plugin API with
no rate limit and no seat gate, which is what makes the library maintainable
rather than a one-time hand-off that drifts.

Re-running is safe: everything is matched by exact name and updated in place.
`node figma-plugin/test.js` executes the generator against a mock Plugin API
and asserts, among other things, that a second run adds nothing.

### Plan constraints worth knowing

The file was built on a Figma **Starter** plan, which forced three departures
from the standard structure. None is a design decision; all are reversible on
Professional:

1. **One mode per collection.** Light/Dark would normally be two modes of a
   single `Color` collection with a toggle. Instead they are two parallel
   collections sharing one set of names. Merging them into modes is the first
   thing to do after an upgrade.
2. **Three pages maximum.** The convention is one page per component; here the
   file is Cover / Foundations / Components, with sections carrying the
   separation.
3. **MCP call limit — 6 per month** on a View seat, which is what stopped the
   original build after Button and Status Badge. The remaining primitives are
   built by the plugin above instead, which is not subject to it.

### Keeping it in sync

The codebase is the source of truth; Figma follows. When a token changes in
`index.css`, update the matching variable's value — the aliases and bound
styles propagate the rest. Do not fix a drift by editing a swatch's fill.

---

## Known debt

Catalogued rather than hidden.

| Debt | Sites | Status |
| --- | --- | --- |
| Arbitrary font sizes | 192 | **Cleared** — [`typography.md`](typography.md#migration) |
| Font sizes below 10px | 15 | **Cleared** — [`typography.md`](typography.md#sizes-below-the-scale) |
| Hardcoded hex colours | 26 | **Cleared** — [`color.md`](color.md#hardcoded-colour-debt) |
| Tailwind palette used for state | 87 | **Cleared** — [`color.md`](color.md#status) |
| Identity colours colliding with status | 6 maps | **Cleared** — [`color.md`](color.md#identity-tones) |
| No vector logo asset | — | Open — [`../brand/README.md`](../brand/README.md#6-logo--marks) |
| Geist Mono not installed | — | Open — [`typography.md`](typography.md#the-mono-face) |
| Figma tokens transcribed by hand | — | Open — [`figma-plugin/README.md`](../../figma-plugin/README.md#keeping-tokens-in-sync) |
| Figma library covers 8 of N components | — | Open — [The Figma library](#the-figma-library) |

### The two checks that keep it cleared

Both should return nothing but `connections.ts` (third-party brand colours,
which are correct as literals):

```sh
cd frontend
# No arbitrary values.
grep -rn 'text-\[' src --include="*.tsx" --include="*.ts"
# No raw palette colours, no raw hex.
grep -rn '\(text\|bg\|border\|ring\|fill\|stroke\)-\(emerald\|rose\|amber\|sky\|red\|green\|blue\|yellow\|violet\|indigo\|teal\|cyan\|slate\)-[0-9]' src --include="*.tsx" --include="*.ts"
grep -rn '#[0-9a-fA-F]\{6\}' src --include="*.tsx" --include="*.ts"
```

Worth a CI job. Debt of this kind returns one convenient exception at a time,
and a grep is cheaper than the argument.
