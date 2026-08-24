// The uploaded-avatar object-URL cache is bounded, and eviction revokes.
//
// `blobUrls` in `src/lib/avatar.ts` is module-level and cached for the life of
// the tab, which is the right trade for faces that recur on every page — but a
// cache that never gave anything back would pin one blob per distinct uploaded
// node ever viewed, and that set is unbounded (every face change mints a new
// node, and the host in the key multiplies the set across connections). Past
// the cap the oldest entry must be dropped *and* its object URL revoked, or
// the backing blob stays alive even though the cache entry is gone.

// @vitest-environment node

import { describe, expect, it, vi } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import { resolveAvatarSrc } from "@/lib/avatar";

describe("resolveAvatarSrc bounded cache", () => {
  it("revokes the oldest object URL and refetches the face once past the cap", async () => {
    const createObjectURL = vi.fn(() => "blob:mock");
    const revokeObjectURL = vi.fn();
    // Node has neither; the stubs stand in for the browser's.
    (URL as { createObjectURL?: unknown }).createObjectURL = createObjectURL;
    (URL as { revokeObjectURL?: unknown }).revokeObjectURL = revokeObjectURL;

    const requested: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string | URL | Request) => {
        requested.push(String(input));
        return {
          ok: true,
          status: 200,
          blob: async () => new Blob([String(input)]),
        } as unknown as Response;
      }),
    );

    const client = new OpenCompanyClient({
      baseUrl: "https://host",
      company: null,
      operatorToken: null,
      sessionHeader: null,
    });

    // `MAX_BLOB_URLS` is 64, so a 65th distinct face evicts the first.
    const ids = Array.from(
      { length: 65 },
      (_, i) => `01J8Z5Q9YQ${String(i).padStart(14, "0")}`,
    );
    for (const id of ids) {
      await resolveAvatarSrc(client, "acme", `blob:${id}`);
    }

    // The first entry was evicted, and its object URL revoked — not silently
    // dropped, which would keep the blob alive with no way to reach it.
    expect(revokeObjectURL).toHaveBeenCalledTimes(1);

    // The evicted face is a cache miss again: asking for it fetches once more
    // rather than answering from the hoard.
    const before = requested.length;
    await resolveAvatarSrc(client, "acme", `blob:${ids[0]}`);
    expect(requested).toHaveLength(before + 1);

    vi.unstubAllGlobals();
  });
});
