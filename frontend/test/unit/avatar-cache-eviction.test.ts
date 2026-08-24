// The uploaded-avatar object-URL cache is bounded, and eviction revokes.
//
// `blobUrls` in `src/lib/avatar.ts` is module-level and cached for the life of
// the tab, which is the right trade for faces that recur on every page — but a
// cache that never gave anything back would pin one blob per distinct uploaded
// node ever viewed, and that set is unbounded (every face change mints a new
// node, and the host in the key multiplies the set across connections). Past
// the cap the oldest entry must be dropped *and* its object URL revoked, or
// the backing blob stays alive even though the cache entry is gone.
//
// Two corollaries of "bounded" are pinned here too. Eviction must never pick
// an entry whose fetch is still in flight: such an entry pins no blob, so
// dropping it saves nothing and throws away a fetch a mounted tile is waiting
// on. And when a workspace node is deleted, `forgetAvatarNode` drops its face
// on the spot rather than letting the cache keep drawing a file that no longer
// exists until the cap or a reload.

// @vitest-environment node

import { describe, expect, it, vi } from "vitest";

import { OpenCompanyClient } from "@/api/client";

/** The cache is module-level, so each test re-imports `avatar` fresh. */
async function freshAvatar() {
  vi.resetModules();
  return import("@/lib/avatar");
}

/** Node ids must be minted per host and unique — distinct faces in the tests. */
function node(i: number): string {
  return `01J8Z5Q9YQ${String(i).padStart(14, "0")}`;
}

/** Stub the URL APIs with distinct URLs so revocation is observable per face. */
function stubUrlApi() {
  let n = 0;
  const createObjectURL = vi.fn(() => `blob:face-${n++}`);
  const revokeObjectURL = vi.fn();
  (URL as { createObjectURL?: unknown }).createObjectURL = createObjectURL;
  (URL as { revokeObjectURL?: unknown }).revokeObjectURL = revokeObjectURL;
  return { createObjectURL, revokeObjectURL };
}

/** Stub `fetch` to answer any request with a tiny blob, recording the URLs. */
function stubFetch() {
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
  return requested;
}

function client() {
  return new OpenCompanyClient({
    baseUrl: "https://host",
    company: null,
    operatorToken: null,
    sessionHeader: null,
  });
}

describe("resolveAvatarSrc bounded cache", () => {
  it("revokes the oldest object URL and refetches the face once past the cap", async () => {
    const { resolveAvatarSrc } = await freshAvatar();
    const { revokeObjectURL } = stubUrlApi();
    const requested = stubFetch();

    // `MAX_BLOB_URLS` is 64, so a 65th distinct face evicts the first.
    const ids = Array.from({ length: 65 }, (_, i) => node(i));
    for (const id of ids) {
      await resolveAvatarSrc(client(), "acme", `blob:${id}`);
    }

    // The first entry was evicted, and its object URL revoked — not silently
    // dropped, which would keep the blob alive with no way to reach it.
    expect(revokeObjectURL).toHaveBeenCalledTimes(1);

    // The evicted face is a cache miss again: asking for it fetches once more
    // rather than answering from the hoard.
    const before = requested.length;
    await resolveAvatarSrc(client(), "acme", `blob:${ids[0]}`);
    expect(requested).toHaveLength(before + 1);

    vi.unstubAllGlobals();
  });

  it("never evicts a face whose fetch is still in flight", async () => {
    const { resolveAvatarSrc } = await freshAvatar();
    const { revokeObjectURL } = stubUrlApi();

    // The 65th request stays in flight until released, so it has no URL yet
    // when the 66th and 67th faces push the map over the cap.
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const requested: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: string | URL | Request) => {
        requested.push(String(input));
        if (requested.length === 65) await gate;
        return {
          ok: true,
          status: 200,
          blob: async () => new Blob([String(input)]),
        } as unknown as Response;
      }),
    );

    const ids = Array.from({ length: 67 }, (_, i) => node(i));
    for (const id of ids.slice(0, 64)) {
      await resolveAvatarSrc(client(), "acme", `blob:${id}`);
    }
    // 65th starts and is held in flight; 66th and 67th resolve and each
    // evicts the oldest *resolved* face — skipping the pending 65th.
    const inFlight = resolveAvatarSrc(client(), "acme", `blob:${ids[64]}`);
    for (const id of ids.slice(65)) {
      await resolveAvatarSrc(client(), "acme", `blob:${id}`);
    }
    expect(revokeObjectURL).toHaveBeenCalledTimes(2); // faces 0 and 1, not 64

    release();
    const url = await inFlight;

    // The held face was not thrown away by eviction: it resolves to a live
    // URL rather than `null`, and that URL was never revoked.
    expect(url).not.toBeNull();
    expect(revokeObjectURL).not.toHaveBeenCalledWith(url);

    vi.unstubAllGlobals();
  });

  it("forgetAvatarNode revokes the URL and makes the face a cache miss", async () => {
    const { resolveAvatarSrc, forgetAvatarNode } = await freshAvatar();
    const { revokeObjectURL } = stubUrlApi();
    const requested = stubFetch();

    // One face, resolved and cached.
    const id = node(0);
    const url = (await resolveAvatarSrc(client(), "acme", `blob:${id}`)) as string;

    // Deleting the node revokes its object URL — the backing blob is released
    // rather than pinned by a cache entry nothing will read again.
    forgetAvatarNode(client(), "acme", id);
    expect(revokeObjectURL).toHaveBeenCalledWith(url);

    // And the next resolve is a miss: it fetches again instead of answering
    // from the hoard (in the real world that re-fetch 404s and the caller
    // draws the tone tile).
    const before = requested.length;
    await resolveAvatarSrc(client(), "acme", `blob:${id}`);
    expect(requested).toHaveLength(before + 1);

    vi.unstubAllGlobals();
  });
});
