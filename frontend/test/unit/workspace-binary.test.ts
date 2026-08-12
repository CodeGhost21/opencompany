/**
 * Binary workspace nodes on the console side (issue #553).
 *
 * The tree can hold bytes now, and `mime` is the single flag that says so. The
 * host omits `mime`/`size`/`sha256` entirely on a prose note rather than
 * sending nulls, so these pin that "present" is the test — a check that treated
 * a present-but-null field as binary would put every note behind the download
 * card.
 */
import { describe, expect, it, vi } from "vitest";

import { OpenCompanyClient } from "@/api/client";
import {
  blobCacheKey,
  fetchTree,
  formatBytes,
  isBinary,
  uploadFile,
  type FsNode,
} from "@/api/workspace";

function client(handler: (req: { method: string; url: string; body?: string }) => unknown) {
  const transport = {
    request: async (req: { method: string; url: string; body?: string }) => ({
      status: 200,
      statusText: "OK",
      url: req.url,
      text: JSON.stringify(handler(req)),
      header: () => null,
    }),
    subscribe: () => () => {},
  };
  return new OpenCompanyClient(
    { baseUrl: "", company: "acme", operatorToken: "t0ken" },
    transport as never,
  );
}

describe("isBinary", () => {
  it("is true only when the host sent a mime type", () => {
    expect(isBinary({ mime: "image/png" })).toBe(true);
    expect(isBinary({ mime: "application/pdf" })).toBe(true);
  });

  it("is false for a prose note, whose mime the host omits entirely", () => {
    expect(isBinary({ mime: undefined })).toBe(false);
    // Defensive: a null would mean the wire shape changed, and treating it as
    // binary would hide every note's editor behind a download card.
    expect(isBinary({ mime: null as unknown as undefined })).toBe(false);
  });
});

describe("formatBytes", () => {
  it("scales to the unit an operator reads", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(3 * 1024 * 1024 * 1024)).toBe("3.0 GB");
  });

  it("says so rather than rendering NaN when the host sent no size", () => {
    expect(formatBytes(undefined)).toBe("unknown size");
  });

  it("keeps zero a real answer, not a missing one", () => {
    expect(formatBytes(0)).toBe("0 B");
  });
});

describe("blobCacheKey", () => {
  /**
   * The defect in #669: a re-publish overwrites a payload in place, keeping the
   * node id and changing the digest. Keying the viewer's fetch on the id alone
   * meant the operator kept seeing — and downloading — the old bytes.
   */
  it("changes when the bytes change, even though the node id does not", () => {
    const before = { id: "n-img", sha256: "aaa" };
    const after = { id: "n-img", sha256: "bbb" };
    expect(blobCacheKey(after)).not.toBe(blobCacheKey(before));
  });

  it("is stable when nothing about the payload changed", () => {
    expect(blobCacheKey({ id: "n-img", sha256: "aaa" })).toBe(
      blobCacheKey({ id: "n-img", sha256: "aaa" }),
    );
  });

  it("still separates two different nodes that happen to hold identical bytes", () => {
    // Two uploads of the same file share a digest. They are different nodes and
    // must not share a cache entry, or opening one would show the other's name.
    expect(blobCacheKey({ id: "n-a", sha256: "same" })).not.toBe(
      blobCacheKey({ id: "n-b", sha256: "same" }),
    );
  });

  it("falls back to the id when the host reported no digest", () => {
    // Nothing here can detect a change the host did not report, so the honest
    // answer is the pre-#553 behaviour rather than a key that churns.
    expect(blobCacheKey({ id: "n-img", sha256: undefined })).toBe("n-img");
  });
});

describe("the tree read", () => {
  it("carries a binary node's metadata through normalization", async () => {
    const c = client(() => [
      {
        id: "n-img",
        name: "hero.png",
        kind: "file",
        updatedAt: 5,
        mime: "image/png",
        size: 2048,
        sha256: "abc123",
      },
      { id: "n-note", name: "brief.md", kind: "file", updatedAt: 6 },
    ]);

    const nodes: FsNode[] = await fetchTree(c, "acme");
    const image = nodes.find((n) => n.id === "n-img")!;
    expect(image.mime).toBe("image/png");
    expect(image.size).toBe(2048);
    expect(image.sha256).toBe("abc123");
    expect(isBinary(image)).toBe(true);

    // Normalization must not invent the fields on a note — `parentId` and the
    // origins are defaulted, these are not.
    const note = nodes.find((n) => n.id === "n-note")!;
    expect(note.mime).toBeUndefined();
    expect(note.size).toBeUndefined();
    expect(isBinary(note)).toBe(false);
    expect(note.parentId).toBeNull();
  });
});

describe("uploadFile", () => {
  it("posts multipart to the upload route and never sets its own content-type", async () => {
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      statusText: "OK",
      url: "",
      headers: new Headers(),
      text: async () =>
        JSON.stringify({
          id: "n-1",
          name: "hero.png",
          kind: "file",
          updatedAt: 1,
          mime: "image/png",
          size: 3,
        }),
    }));
    vi.stubGlobal("fetch", fetchMock);

    const c = client(() => ({}));
    const file = new File([new Uint8Array([1, 2, 3])], "hero.png", { type: "image/png" });
    const node = await uploadFile(c, "acme", file, "f-1");

    expect(node.mime).toBe("image/png");
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/v1/companies/acme/workspace/upload");
    expect(init.method).toBe("POST");
    expect(init.body).toBeInstanceOf(FormData);
    // The browser must set `content-type` itself so the multipart boundary it
    // generated is in the header; setting it here produces a body the host
    // cannot parse.
    const headers = init.headers as Record<string, string>;
    expect(headers["content-type"]).toBeUndefined();
    expect(headers["authorization"]).toBe("Bearer t0ken");

    const form = init.body as FormData;
    expect((form.get("file") as File).name).toBe("hero.png");
    expect(form.get("parentId")).toBe("f-1");

    vi.unstubAllGlobals();
  });

  it("omits parentId entirely when the upload targets the workspace root", async () => {
    const fetchMock = vi.fn(async () => ({
      ok: true,
      status: 200,
      statusText: "OK",
      url: "",
      headers: new Headers(),
      text: async () => JSON.stringify({ id: "n-1", name: "a.png", kind: "file", updatedAt: 1 }),
    }));
    vi.stubGlobal("fetch", fetchMock);

    const c = client(() => ({}));
    await uploadFile(c, "acme", new File([new Uint8Array([1])], "a.png"), null);

    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect((init.body as FormData).has("parentId")).toBe(false);

    vi.unstubAllGlobals();
  });
});
