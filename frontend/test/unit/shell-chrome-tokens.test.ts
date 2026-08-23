import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * The chrome tokens live in the token layer, and only there (issue #1178).
 *
 * The shell's two layers are a *token* decision: the window chrome is one
 * semantic value with a light and a dark binding, and every component reaches
 * it through the `bg-chrome` / `border-chrome-border` utilities the `@theme
 * inline` block mints. That is what lets the layer be retuned in one place —
 * the legibility of the sidebar's faintest labels rides on it — and it is what
 * `scripts/ci/assert-design-tokens.sh` is defending when it refuses a raw hex
 * in a component.
 *
 * This test guards the shape of that layer rather than the colours themselves,
 * which are asserted resolved, in a real browser and in both themes, by
 * `test/e2e/shell-two-layer.spec.ts`. Three ways it regresses, all silent:
 *
 *   - a token deleted from one theme block, so the other theme falls back to
 *     the light value and the shell loses a layer in dark;
 *   - the `@theme inline` alias dropped, so `bg-chrome` stops existing and
 *     Tailwind emits nothing at all for it — no error, just an unpainted shell;
 *   - the alias moved to a plain `@theme`, which bakes the light value in at
 *     build time so the `.dark` override never reaches it. `index.css` already
 *     carries that warning at the top of the block; this is the check.
 */

const indexCss = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../../src/index.css"),
  "utf8",
);

/** The body of the first top-level block opened by `selector`, brace-matched. */
function block(selector: string): string {
  const open = indexCss.indexOf(`${selector} {`);
  // Thrown rather than asserted: this runs while the suite is being collected,
  // where a failed `expect` is reported as a collection error rather than as
  // the failing test it belongs to.
  if (open < 0) throw new Error(`no \`${selector} {\` block in index.css`);
  let depth = 0;
  for (let i = indexCss.indexOf("{", open); i < indexCss.length; i += 1) {
    if (indexCss[i] === "{") depth += 1;
    else if (indexCss[i] === "}") {
      depth -= 1;
      if (depth === 0) return indexCss.slice(open, i);
    }
  }
  throw new Error(`unterminated \`${selector}\` block`);
}

/** The declared value of `--name` in `body`, ignoring anything in a comment. */
function declaration(body: string, name: string): string | null {
  const withoutComments = body.replace(/\/\*[\s\S]*?\*\//g, "");
  const match = new RegExp(`(?:^|[;{\\s])--${name}:\\s*([^;]+);`).exec(withoutComments);
  return match ? match[1].trim() : null;
}

describe("chrome tokens", () => {
  const light = block(":root");
  // The dark bindings, not the `.dark { --kg-* }` graph bridge further down.
  const dark = block(".dark");

  it("binds the chrome and its hairline in both themes", () => {
    for (const token of ["chrome", "chrome-border"]) {
      expect(declaration(light, token), `--${token} missing from :root`).not.toBeNull();
      expect(declaration(dark, token), `--${token} missing from .dark`).not.toBeNull();
    }
  });

  it("gives the two themes different chrome, so the layer survives a theme switch", () => {
    expect(declaration(light, "chrome")).not.toBe(declaration(dark, "chrome"));
    expect(declaration(light, "chrome-border")).not.toBe(declaration(dark, "chrome-border"));
  });

  it("routes both through a primitive or a rung rather than a literal", () => {
    // Layer 2 names what a colour means and points at layer 1 for the value.
    // A literal here is the debt `docs/design-system/README.md` describes: a
    // colour the system cannot see, retune, or check the contrast of.
    for (const body of [light, dark]) {
      for (const token of ["chrome", "chrome-border"]) {
        expect(declaration(body, token)).toMatch(/^var\(--/);
      }
    }
  });

  it("mints the utilities from an `@theme inline` block", () => {
    // `inline` is load-bearing: a plain `@theme` resolves the variable at build
    // time, baking the light value in, and the `.dark` override above would
    // then reach nothing.
    // Brace-matched, NOT split-to-end-of-file: `index.css` carries a plain
    // `@theme { … }` after this block, and a search that ran to the end of the
    // file would accept an alias that had been moved into it — which is the
    // exact regression this case exists to catch.
    const theme = block("@theme inline");
    expect(declaration(theme, "color-chrome")).toBe("var(--chrome)");
    expect(declaration(theme, "color-chrome-border")).toBe("var(--chrome-border)");
  });
});

describe("light accessibility tokens", () => {
  const light = block(":root");

  it("gives form controls a stronger boundary than decorative borders", () => {
    // A text field has no fill difference from the card it sits on. Its
    // `--input` stroke must therefore not inherit the deliberately subtle
    // decorative `--border` value (issue #1394).
    expect(declaration(light, "input")).not.toBe(declaration(light, "border"));
    expect(declaration(light, "input")).toBe("var(--surface-light-input)");
    expect(declaration(light, "surface-light-input")).toBe("oklch(0.62 0.0149 286.09)");
  });

  it("uses the AA light error text weight for destructive text", () => {
    expect(declaration(light, "destructive")).toBe("var(--red-text)");
  });

  it("keeps every light status mark at its measured accessible weight", () => {
    expect(declaration(light, "green-mark")).toBe("oklch(0.60 0.1627 151.05)");
    expect(declaration(light, "cyan-mark")).toBe("oklch(0.61 0.1479 237.32)");
    expect(declaration(light, "amber-mark")).toBe("oklch(0.62 0.1585 72.33)");
  });
});
