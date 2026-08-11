# Figma design system generator

Writes the design tokens from `frontend/src/index.css` into a Figma file as
variables, styles and components. The Figma library is a **build output**, not
a hand-drawn artefact — run this again after a token changes and the file
catches up.

This exists because the hosted Figma MCP server is capped at **6 tool calls per
month** on a Starter plan with a View seat, which is not enough to build or
maintain a library. A development plugin runs the same Plugin API with **no
rate limit and no seat gate**.

---

## Install (once, ~30 seconds)

1. Open the **Figma desktop app** (this does not work in the browser).
2. Open the file you want to write to — for OpenCompany that is
   [OpenCompany Design System](https://www.figma.com/design/bUj8Ofz2EQL6Y8DU06zbDR).
3. Menu → **Plugins → Development → Import plugin from manifest…**
4. Choose `figma-plugin/manifest.json` from this repo.

It now appears under **Plugins → Development → OpenCompany Design System**.
Importing links the plugin to these files on disk, so editing `code.js` and
re-running picks up the change — no reinstall.

## Run

**Plugins → Development → OpenCompany Design System**, then:

| Option | Does |
| --- | --- |
| **Everything** | Variables, styles, pages, components |
| **Tokens only** | Variables and styles; leaves components untouched |
| **Rebuild components** | Replaces components that already exist — **detaches their instances**. Leave off unless you mean it. |

**Safe to re-run.** Everything is matched by exact name and updated in place,
so a second run changes values without duplicating anything. Verified by the
test below, which runs the generator twice and asserts nothing was added.

## What it writes

| | Count | |
| --- | --- | --- |
| `Primitives` | 39 | Brand ramp, neutrals, status hues. Scopes `[]` — hidden from every picker. |
| `Color · Light` | 32 | Semantic roles, each an **alias** to a primitive |
| `Color · Dark` | 32 | Same names, independently tuned values |
| `Scale` | 22 | Radius, spacing, font sizes |
| Text styles | 17 | `Body/`, `Label/`, `Heading/`, `Mono/` — size bound to `Scale` |
| Effect styles | 12 | `Elevation/` and `Elevation Dark/` |
| Components | 8 | Button, Status Badge, Input, Badge, Alert, Avatar, Tab, Card |

Every variable carries an explicit scope and a **WEB code syntax** naming the
real CSS variable, so `color/status/running` reports in Dev Mode as
`var(--status-running)` — the name that exists in this codebase, not an
invented one.

## Test

```sh
node figma-plugin/test.js
```

Executes the generator against a mock Plugin API. It catches the class of bug
that would otherwise only appear after loading the plugin — missing tokens,
wrong call order, writing text before its font is loaded — and asserts:

- no variable is left at `ALL_SCOPES`
- every variable has a value and a code syntax
- exactly three pages exist
- **a second run adds nothing** (idempotency)

It does not validate Figma's own semantics: whether a sizing mode is legal in a
given structural context, or whether a scope name is one Figma accepts. Those
only show up in the real app.

## Keeping tokens in sync

`code.js` holds the token block near the top. It is transcribed from
`frontend/src/index.css` **by hand, deliberately** — parsing the stylesheet
would mean resolving `color-mix()`, `oklch()` and the cascade, and would break
silently the first time one of them moved.

Values are hex here because the Plugin API takes sRGB; they are the same
colours the stylesheet declares in oklch. When you change a token, change it in
`index.css` first (that is the source of truth), then mirror it here and re-run.

## Plan limits this cannot work around

Enforced by Figma on the file, not by this plugin. A Professional plan lifts
both:

- **3 pages max**, so Foundations and Components share pages instead of one
  page per component.
- **1 mode per collection**, so Light and Dark are parallel collections rather
  than two modes of one `Color` collection with a toggle. Merging them is the
  first thing worth doing after an upgrade.

## See also

- [`docs/design-system/README.md`](../docs/design-system/README.md) — the layer rule and how to change a token
- [`docs/brand/README.md`](../docs/brand/README.md) — why these values
- `#/styleguide` in the running console — the same system rendered by the real stylesheet
