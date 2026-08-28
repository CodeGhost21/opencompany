import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const composer = readFileSync(resolve(here, "../../src/views/chat/MessageComposer.tsx"), "utf8");

describe("composer intent emphasis (issue #1341)", () => {
  it("uses a subtle primary tint for the selected intent instead of a solid fill", () => {
    expect(composer).toContain('? "bg-primary/10 text-brand-700 dark:text-brand-300"');
    expect(composer).not.toContain('? "bg-primary text-primary-foreground"');
  });

  it("leaves the circular Send button as the primary-filled composer action", () => {
    const idx = composer.indexOf('aria-label="Send"');
    expect(idx).toBeGreaterThan(-1);
    const sendButton = composer.slice(Math.max(0, idx - 300), idx);
    expect(sendButton).toContain("rounded-full");
  });
});
