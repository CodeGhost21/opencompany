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

## Known debt

Catalogued rather than hidden. Both lists are complete as of this document.

| Debt | Sites | Detail |
| --- | --- | --- |
| Arbitrary font sizes | 192 | [`typography.md`](typography.md#migration) |
| Font sizes below 10px | 15 | [`typography.md`](typography.md#sizes-below-the-scale) |
| Hardcoded hex colours | 26 | [`color.md`](color.md#hardcoded-colour-debt) |
| No vector logo asset | — | [`../brand/README.md`](../brand/README.md#6-logo--marks) |
| Geist Mono not installed | — | [`typography.md`](typography.md#the-mono-face) |

None of these break the build; all of them are places where a future change
will not propagate. They are safe to fix incrementally, file by file.
