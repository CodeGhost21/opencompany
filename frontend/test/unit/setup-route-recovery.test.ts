import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { VIEWS } from "@/lib/console-routes";

const here = dirname(fileURLToPath(import.meta.url));
const read = (rel: string) => readFileSync(resolve(here, "../../src", rel), "utf8");

describe("first-run setup recovery (issue #1417)", () => {
  it("keeps the manual setup address routable", () => {
    expect(VIEWS).toContain("setup");
  });

  it("offers the same route beside the product-tour replay control", () => {
    const settings = read("views/SettingsView.tsx");

    expect(settings).toContain('href="#/setup"');
    expect(settings.indexOf('href="#/setup"')).toBeGreaterThan(settings.indexOf("Replay tour"));
  });
});
