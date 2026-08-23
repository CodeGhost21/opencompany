import { readFileSync } from "node:fs";
import { basename } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Every section of the Connections page sits at the same heading level.
 *
 * `ConnectionsView` renders one `h1` ("Connections") and then a row of peer
 * sections — MCP Servers, Inference, the company key, Composio, Channels,
 * repositories, providers, the account choice. They are siblings in the page's
 * outline, so they have to be siblings in its heading levels too.
 *
 * They were all `h3` under that `h1`, which axe reported as `heading-order` on
 * `#/settings/connections` (issue #1392): a jump from 1 to 3 reads to a screen
 * reader as a missing section. Promoting only MCP to `h2` would have fixed the
 * axe finding and broken the outline in a way axe cannot see — heading
 * navigation would then present Inference, Composio, Channels and the rest as
 * subsections *of* MCP Servers rather than as its peers. The fix is that they
 * all move together, and this pins the "together" half.
 *
 * A source guard, like `dialog-width-override`: the failure is a correct-looking
 * component in isolation. Nothing is wrong with an `h3` until you know what it
 * is rendered beside, and that context lives in `ConnectionsView`, not in the
 * section.
 */
const VIEWS = new URL("../../src/views", import.meta.url).pathname;

/**
 * The sections `ConnectionsView` renders directly, each contributing one
 * top-level heading to that page. `McpServersSection` is here too: it also
 * serves the standalone `#/settings/mcp` page, where its heading sits under
 * that page's own `h1` — the same level either way, which is why one tag can
 * serve both.
 */
const SECTIONS = [
  "McpServersSection",
  "InferenceSection",
  "CompanyCredentialCard",
  "ComposioSection",
  "ChannelsSection",
  "RepositoriesCard",
  "ProvidersSection",
  "AccountChoiceSection",
] as const;

/** The heading tags a file opens, in source order. */
function headingLevels(source: string): number[] {
  return [...source.matchAll(/<h([1-6])[\s>]/g)].map(([, level]) => Number(level));
}

describe("Connections page sections", () => {
  it("are all rendered by ConnectionsView, so this list cannot go stale", () => {
    const view = readFileSync(`${VIEWS}/ConnectionsView.tsx`, "utf8");
    const missing = SECTIONS.filter((name) => !view.includes(`<${name}`));

    expect(missing, `no longer rendered by ConnectionsView: ${missing.join(", ")}`).toEqual([]);
  });

  it("each head their section at level two, as peers of one another", () => {
    const offenders: string[] = [];
    for (const name of SECTIONS) {
      const path = `${VIEWS}/connections/${name}.tsx`;
      const levels = headingLevels(readFileSync(path, "utf8"));
      // A section may carry sub-headings; only its own top level is a peer of
      // the other sections'. The shallowest heading in the file is that one.
      const top = Math.min(...levels);
      if (top !== 2) offenders.push(`${basename(path)} heads its section with h${top}`);
    }

    expect(
      offenders,
      `these sections disagree with their peers on the Connections outline:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("keeps the one h1 on the page in ConnectionsView itself", () => {
    const view = readFileSync(`${VIEWS}/ConnectionsView.tsx`, "utf8");

    expect(headingLevels(view)).toEqual([1]);
  });
});
