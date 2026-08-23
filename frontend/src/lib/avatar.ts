// Which face a teammate or a person wears.
//
// Everybody has one whether or not they chose it: a stable id is hashed into
// one of the mascots shipped in `public/avatars/`, so an untouched roster reads
// as a set of individuals rather than a column of grey squares. This module is
// the other half — what happens when somebody *does* choose.
//
// The stored reference grammar mirrors `src/company/avatar.rs` exactly; see
// `docs/spec/runtime/avatars.md` for why it is closed rather than "store a URL".

import type { OpenCompanyClient } from "@/api/client";

/**
 * The mascots shipped in `public/avatars/blob-<flavour>.webp`.
 *
 * **Must stay in step with `TINY_FLAVOURS` in `src/company/avatar.rs`**, which
 * is what the host validates against: a flavour one side accepts and the other
 * has no file for renders as a broken image on every surface at once.
 *
 * Eleven rather than the eight tone keys, deliberately — the tones are a hue
 * circle that avoids amber, green and red so they cannot be confused with run
 * status, and the mascots are under no such constraint.
 */
export const TINY_FLAVOURS = [
  "amber",
  "blue",
  "clay",
  "cloud",
  "ember",
  "graphite",
  "green",
  "indigo",
  "rose",
  "teal",
  "violet",
] as const;

export type TinyFlavour = (typeof TINY_FLAVOURS)[number];

/** The image types an uploaded avatar may be — the `accept` a file input wants. */
export const AVATAR_ACCEPT = "image/png,image/jpeg,image/webp,image/gif";

/**
 * The host's upload ceiling, in whole megabytes — for saying so before somebody
 * picks a file, not for enforcing it.
 *
 * The enforcement is `MAX_AVATAR_BYTES` in `src/company/avatar.rs`, and it stays
 * there: a limit checked in the browser is a limit anybody can skip. This is the
 * number the picker prints, so the copy and the refusal name the same figure.
 */
export const MAX_AVATAR_MB = 4;

/**
 * Picks a mascot from a seed, for whoever has not chosen one.
 *
 * A hash rather than a random draw, for the reason that matters to an operator:
 * a teammate keeps the same face across reloads, browsers and machines with
 * nothing persisted anywhere. Drawing randomly at creation would need a stored
 * field; drawing randomly at render would give the same teammate a new face
 * every time the page reloaded.
 *
 * Seeded on the id wherever there is one, so renaming somebody does not change
 * their face.
 */
export function hashedFlavour(seed: string): TinyFlavour {
  let hash = 0;
  for (let i = 0; i < seed.length; i++) hash = (hash * 31 + seed.charCodeAt(i)) | 0;
  return TINY_FLAVOURS[Math.abs(hash) % TINY_FLAVOURS.length];
}

/**
 * The reference to draw for somebody: what they chose, else the hashed default.
 *
 * `chosen` is the host's field, absent when nobody has chosen — which is not the
 * same as "no face", and is exactly why the host skips the key rather than
 * defaulting it.
 */
export function avatarRef(chosen: string | undefined, seed: string): string {
  const trimmed = chosen?.trim();
  return trimmed || `tiny:${hashedFlavour(seed)}`;
}

/** Where a mascot lives on disk. */
export function tinySrc(flavour: string): string {
  return `/avatars/blob-${flavour}.webp`;
}

/** The workspace node id a `blob:` reference names, or `null` for any other form. */
export function blobNodeId(ref: string): string | null {
  const trimmed = ref.trim();
  return trimmed.startsWith("blob:") ? trimmed.slice("blob:".length) : null;
}

/**
 * The `src` for a reference that needs no fetch — a mascot — or `null` for one
 * that does.
 *
 * Split from {@link resolveAvatarSrc} so the common case stays synchronous: the
 * overwhelming majority of faces on any screen are mascots, and making every
 * avatar await a promise would flash an empty gutter on every mount for the sake
 * of the few that are uploads.
 */
export function staticAvatarSrc(ref: string): string | null {
  const trimmed = ref.trim();
  if (trimmed.startsWith("tiny:")) return tinySrc(trimmed.slice("tiny:".length));
  // An unrecognised reference is drawn as nothing rather than as itself. The
  // host refuses to store anything but the two forms, so this can only be
  // version skew — and putting an unknown string into a `src=` is the one thing
  // the closed grammar exists to prevent.
  return null;
}

/**
 * Object URLs for uploaded avatars, keyed by host, company and node id.
 *
 * Module-level and never revoked, which is deliberate rather than sloppy. An
 * avatar is drawn in dozens of places on one screen — chat gutters, facepiles,
 * the members pane, the org chart — and the same faces recur on every page the
 * operator visits. Revoking on unmount would mean refetching a teammate's face
 * each time it scrolled out of a list and back, and per-component caching would
 * fetch the same bytes once per component. The cost is bounded by the number of
 * *uploaded* avatars a company has, each capped at 4 MB by the host.
 *
 * The host is part of the key because the map outlives a connection switch: the
 * desktop console remounts `AppShell` when it changes hosts, but this module
 * does not reload, so a key that named only company and node would let the
 * second host draw the first host's bytes when two hosts hold the same
 * company/node ids — a cloned or restored company, say. Node ids are minted per
 * host, so the collision is exactly the case this prefix exists for.
 *
 * The promise is cached, not the URL, so N components mounting at once share one
 * request instead of racing N.
 */
const blobUrls = new Map<string, Promise<string | null>>();

/**
 * The `src` for any reference, fetching an uploaded one through the client.
 *
 * A plain `<img src="…/workspace/blob/{id}">` would not work: the route needs
 * the credential the client holds, and an image element cannot carry one. So the
 * bytes are fetched through the authenticated client and wrapped in an object
 * URL the element can point at — the same shape `fetchBlobUrl` uses for the
 * workspace viewer, but cached, because a face is drawn far more often than a
 * document is opened.
 *
 * Resolves to `null` when the reference names nothing this host holds any more
 * (an avatar whose node was deleted from the Files tab). Callers draw the tone
 * tile they were already drawing underneath, so a deleted image degrades to a
 * coloured square rather than to a broken-image glyph.
 */
export function resolveAvatarSrc(
  client: OpenCompanyClient,
  company: string | null,
  ref: string,
): string | Promise<string | null> {
  const staticSrc = staticAvatarSrc(ref);
  if (staticSrc) return staticSrc;
  const node = blobNodeId(ref);
  if (!node) return Promise.resolve(null);
  const key = `${client.baseUrl}|${company ?? ""}|${node}`;
  let pending = blobUrls.get(key);
  if (!pending) {
    pending = client
      .getBlob(`${client.scopeFor(company)}/workspace/blob/${encodeURIComponent(node)}`)
      .then((blob) => URL.createObjectURL(blob))
      .catch(() => {
        // Not cached as a failure: a face that 404s because the workspace was
        // mid-write should be retried on the next mount, not remembered as
        // missing for the life of the tab.
        blobUrls.delete(key);
        return null;
      });
    blobUrls.set(key, pending);
  }
  return pending;
}

/** What `POST …/avatars` answers with. */
export interface UploadedAvatar {
  /** The reference to store — `blob:<nodeId>`. */
  avatar: string;
  nodeId: string;
  /** The type the host **sniffed** from the bytes, not the one the browser declared. */
  mime: string;
  size: number;
}

/**
 * Uploads an image and returns the reference to store.
 *
 * Nothing is worn by uploading: the caller then saves the reference onto a
 * teammate or onto themselves. Keeping the two steps apart is what lets a picker
 * preview an image before it is committed.
 */
export async function uploadAvatar(
  client: OpenCompanyClient,
  company: string | null,
  file: File,
): Promise<UploadedAvatar> {
  const form = new FormData();
  form.append("file", file, file.name);
  return client.postForm<UploadedAvatar>(`${client.scopeFor(company)}/avatars`, form);
}
