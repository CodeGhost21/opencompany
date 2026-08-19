// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import {
  adoptEmbeddedHost,
  adoptLocalHosts,
  getConnection,
  listConnections,
  resetConnections,
  restoreConnections,
} from "@/connections/registry";
import { readProfiles } from "@/connections/profileStore";
import { scopedKey } from "@/connections/types";

/**
 * More than one host on one machine.
 *
 * The desktop used to run exactly one: a single data root, a single embedded
 * host, and — in the console — a prune that treated *any other* embedded
 * profile as a dead row from a previous launch. That prune is the part that
 * cannot survive a roster: an operator's second local company looks exactly
 * like last launch's ghost, because both are "an embedded profile that is not
 * the one being adopted".
 *
 * So these tests are about the set: every running instance keeps its own row
 * and its own id across relaunches, and only the instances that really are gone
 * are dropped.
 */

const ACME = "0f9d8c7b6a5e4f3d2c1b0a9988776655";
const BEAM = "1122334455667788aabbccddeeff0011";

beforeEach(() => {
  resetConnections();
  window.localStorage.clear();
});

function relaunch(): void {
  resetConnections();
  restoreConnections();
}

describe("several hosts on this machine", () => {
  it("keeps a row per instance", () => {
    const [acme, beam] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:65145", instanceId: ACME, label: "This computer" },
      { baseUrl: "http://127.0.0.1:65146", instanceId: BEAM, label: "Acme" },
    ]);

    expect(acme).not.toBe(beam);
    expect(listConnections()).toHaveLength(2);
    expect(getConnection(beam)?.label).toBe("Acme");
  });

  it("gives each instance an id that survives a relaunch", () => {
    // THE regression the single-host prune would reintroduce. Both instances
    // move to a fresh ephemeral port on every launch, so neither is
    // recognisable by address — and every browser-local key is scoped by the
    // connection id, so a re-mint orphans one instance's tour state, last-read
    // channel and mail draft with nothing reporting it.
    const before = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:65145", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:65146", instanceId: BEAM },
    ]);

    relaunch();
    const after = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:51001", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:51002", instanceId: BEAM },
    ]);

    expect(after).toEqual(before);
    expect(listConnections()).toHaveLength(2);
    expect(readProfiles()).toHaveLength(2);
    expect(scopedKey("oc-tour", { connection: after[1], company: null })).toBe(
      scopedKey("oc-tour", { connection: before[1], company: null }),
    );
  });

  it("follows the port each instance is actually listening on", () => {
    adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:65145", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:65146", instanceId: BEAM },
    ]);
    relaunch();
    const [acme, beam] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:51001", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:51002", instanceId: BEAM },
    ]);

    expect(getConnection(acme)?.baseUrl).toBe("http://127.0.0.1:51001");
    expect(getConnection(beam)?.baseUrl).toBe("http://127.0.0.1:51002");
  });

  it("drops only the instances that are really gone", () => {
    const [, beam] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:65145", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:65146", instanceId: BEAM },
    ]);

    // Someone removed the Acme instance from the roster. The other one is not
    // a ghost — it is the company they are still using.
    relaunch();
    const [still] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:51002", instanceId: BEAM },
    ]);

    expect(still).toBe(beam);
    expect(listConnections()).toHaveLength(1);
    expect(readProfiles()).toHaveLength(1);
  });

  it("does not let two instances adopt one id-less profile", () => {
    // What an older shell wrote: one embedded row, no identity recorded,
    // because no version that wrote it reported one. Exactly one instance may
    // inherit it — two would share a connection id, and with it every scoped
    // key, which is the failure `types.ts` exists to prevent.
    window.localStorage.setItem(
      "oc.connections.v1",
      JSON.stringify([
        {
          id: "vad0klxipf59",
          baseUrl: "http://127.0.0.1:65275",
          label: "This computer",
          defaultCompany: null,
          credential: { kind: "cookie" },
        },
      ]),
    );
    restoreConnections();

    const [acme, beam] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:51001", instanceId: ACME },
      { baseUrl: "http://127.0.0.1:51002", instanceId: BEAM },
    ]);

    expect(acme).toBe("vad0klxipf59");
    expect(beam).not.toBe(acme);
    expect(listConnections()).toHaveLength(2);
  });

  it("takes the name the core reports over the remembered one", () => {
    // Renaming happens in the core, which owns the roster. A label written to
    // `localStorage` at first sight would otherwise outrank it forever.
    const [id] = adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:65145", instanceId: ACME, label: "Acme" },
    ]);
    relaunch();
    adoptLocalHosts([
      { baseUrl: "http://127.0.0.1:51001", instanceId: ACME, label: "Acme Holdings" },
    ]);

    expect(getConnection(id)?.label).toBe("Acme Holdings");
    expect(readProfiles()[0]?.label).toBe("Acme Holdings");
  });

  it("still behaves as one host for a shell that reports one", () => {
    // `adoptEmbeddedHost` is the one-host call, kept because the shell and the
    // console ship independently: a `pnpm dev` console against an older
    // `cargo` build gets exactly this path.
    const one = adoptEmbeddedHost({ baseUrl: "http://127.0.0.1:65145", instanceId: ACME });
    relaunch();
    const again = adoptEmbeddedHost({
      baseUrl: "http://127.0.0.1:51001",
      instanceId: ACME,
    });

    expect(again).toBe(one);
    expect(listConnections()).toHaveLength(1);
  });
});
