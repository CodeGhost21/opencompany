import { readdirSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { STYLEGUIDE_COMPONENTS } from "@/views/styleguide-components";

const frontendRoot = resolve(fileURLToPath(new URL("../..", import.meta.url)));

describe("the living styleguide", () => {
  it("names every shipped UI primitive", () => {
    const shipped = readdirSync(resolve(frontendRoot, "src/components/ui"))
      .filter((file) => file.endsWith(".tsx"))
      .map((file) => file.slice(0, -".tsx".length))
      .sort();

    expect([...STYLEGUIDE_COMPONENTS].sort()).toEqual(shipped);
  });
});
