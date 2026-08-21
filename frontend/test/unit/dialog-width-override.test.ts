import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * A `DialogContent` that asks to be wide has to ask at the `sm:` breakpoint.
 *
 * `DialogContent`'s own base carries `sm:max-w-sm`. `tailwind-merge` dedupes
 * conflicting utilities, but `max-w-2xl` and `sm:max-w-sm` are different
 * *variants*, so both survive the merge and the responsive one wins on every
 * screen ≥640px. A caller writing `max-w-3xl` therefore gets 384px and no
 * warning — the class is present in the DOM, it simply loses.
 *
 * Four dialogs were shipping that way, all on the Work surface: the rendered
 * file viewer (a *document*, in a 40-character measure), the compose form, the
 * declare wizard — where the 384px width was also what wrapped its five-step
 * indicator and left a connector dangling into empty space — and Edit task.
 *
 * This is a source guard rather than a render test on purpose. The failure is
 * invisible at every level below pixels: the component renders, the class is
 * there, the type checks, and only a browser at ≥640px shows the wrong width.
 * A grep is the level the mistake actually lives at.
 */
const SRC = new URL("../../src", import.meta.url).pathname;

/** Every `.tsx` under `src/`. */
function sources(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) sources(path, out);
    else if (name.endsWith(".tsx")) out.push(path);
  }
  return out;
}

/** `max-w-*` that is not behind a breakpoint, inside a `DialogContent` tag. */
const BARE_MAX_W = /<DialogContent[^>]*className="([^"]*)"/g;

describe("DialogContent width overrides", () => {
  it("are all responsive, so none of them silently loses to the base", () => {
    const offenders: string[] = [];
    for (const path of sources(SRC)) {
      const source = readFileSync(path, "utf8");
      for (const [, classes] of source.matchAll(BARE_MAX_W)) {
        const bare = classes
          .split(/\s+/)
          .filter((held) => /^max-w-/.test(held) && !held.startsWith("max-w-["));
        if (bare.length > 0) {
          offenders.push(`${relative(SRC, path)}: ${bare.join(" ")}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
