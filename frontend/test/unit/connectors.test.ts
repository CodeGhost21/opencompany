// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

import { addConnection, resetConnections, restoreConnections } from "@/connections/registry";
import {
  EMBEDDED_LABEL,
  connectorOf,
  localProfiles,
  readProfiles,
  saveProfile,
} from "@/connections/profileStore";
import type { ConnectionProfile } from "@/connections/profileStore";
import { availableConnectors } from "@/connections/types";
import { firstHostCopy } from "@/connections/first-host";
import {
  CLOUD_WAKE_WINDOW_MS,
  WAKE_RETRY_CEILING_MS,
  keepWaking,
  wakeRetryDelay,
} from "@/connections/waking";

/**
 * Where a runtime runs, and the three things that have to stay true about it.
 *
 * A connector is a *durable* claim: a local host and an SSH tunnel both change
 * address on every launch, and a cloud tenant and a self-hosted gateway are
 * both `https://…`. So nothing may be re-derived from the url, an upgrade must
 * not lose what an older version recorded, and a hibernating tenant must not
 * be reported as a host that is gone.
 */

const INDEX_KEY = "oc.connections.v1";

function stored(profiles: unknown[]): void {
  window.localStorage.setItem(INDEX_KEY, JSON.stringify(profiles));
}

const BASE = {
  id: "aaaaaaaaaaaa",
  baseUrl: "http://127.0.0.1:65275",
  label: EMBEDDED_LABEL,
  defaultCompany: null,
  credential: { kind: "cookie" as const },
};

beforeEach(() => {
  resetConnections();
  window.localStorage.clear();
});

describe("reading an older store forward", () => {
  it("recognises the marker the version before connectors wrote", () => {
    expect(connectorOf({ ...BASE, origin: "embedded" })).toEqual({ kind: "local" });
  });

  it("recognises the embedded host a version that wrote no marker at all left", () => {
    // The only evidence is the signature the bug left behind: this client's own
    // label, at a loopback address, with neither newer field present.
    expect(connectorOf(BASE)).toEqual({ kind: "local" });
  });

  it("does not mistake a host someone typed for one this client started", () => {
    // The false positive that matters: guessing wrong here deletes a
    // connection an operator added, because a local profile is pruned against
    // the core's roster and this host is not on it.
    expect(connectorOf({ ...BASE, label: "127.0.0.1:8080" })).toEqual({ kind: "remote" });
    expect(connectorOf({ ...BASE, instanceId: "acme" })).toEqual({ kind: "remote" });
    expect(connectorOf({ ...BASE, baseUrl: "https://acme.example.com" })).toEqual({
      kind: "remote",
    });
  });

  it("prefers what is written down over anything guessed from it", () => {
    const ssh: ConnectionProfile = {
      ...BASE,
      connector: { kind: "ssh", target: { destination: "vps", remotePort: 8080 } },
    };
    // Loopback and this client's own label, and still not a local host: an SSH
    // tunnel is addressed at loopback too, and pruning it against the local
    // roster would drop it on every launch.
    expect(connectorOf(ssh).kind).toBe("ssh");
    saveProfile(ssh);
    expect(localProfiles()).toHaveLength(0);
  });

  it("settles the guess permanently on the first save", () => {
    stored([BASE]);
    restoreConnections();

    const [written] = readProfiles();
    expect(written.connector).toEqual({ kind: "local" });
    // And keeps the retired field beside it, so a build rolled back to the
    // previous version still prunes last launch's address (issue #615).
    expect(written.origin).toBe("embedded");
  });
});

describe("what the store refuses to read", () => {
  it("drops a profile whose connector is not one this build knows", () => {
    // User-writable storage. An unrecognised kind must not reach the shell as
    // a tunnel request for `undefined`.
    stored([{ ...BASE, connector: { kind: "kubernetes" } }]);
    expect(readProfiles()).toHaveLength(0);
  });

  it("drops an ssh connector with nothing to connect to", () => {
    stored([{ ...BASE, connector: { kind: "ssh", target: { remotePort: 8080 } } }]);
    expect(readProfiles()).toHaveLength(0);
  });

  it("keeps a well-formed one", () => {
    stored([
      { ...BASE, connector: { kind: "cloud", tenant: "acme" } },
      {
        ...BASE,
        id: "bbbbbbbbbbbb",
        connector: { kind: "ssh", target: { destination: "bastion", remotePort: 8080 } },
      },
    ]);
    expect(readProfiles()).toHaveLength(2);
  });
});

describe("what a connection is registered as", () => {
  it("treats a url someone supplied as a gateway they run", () => {
    const id = addConnection({ baseUrl: "https://acme.example.com" });
    expect(readProfiles().find((p) => p.id === id)?.connector).toEqual({ kind: "remote" });
  });

  it("carries the connector across a reload", () => {
    addConnection({
      baseUrl: "https://acme.opencompany.cloud",
      connector: { kind: "cloud", tenant: "acme" },
    });

    resetConnections();
    const [id] = restoreConnections();
    // The whole point of persisting it: the address says nothing about which
    // of the two `https://` connectors this is, so a reload that re-derived it
    // would silently lose the waking behaviour.
    expect(connectorOf(readProfiles().find((p) => p.id === id)!)).toEqual({
      kind: "cloud",
      tenant: "acme",
    });
  });
});

describe("a host reached over a tunnel", () => {
  const target = { destination: "acme-vps", remotePort: 8080 } as const;

  it("keeps its id when the tunnel comes back on a different port", () => {
    // The loopback port is bound fresh every launch, so matching on the
    // address would mint a new id per run and orphan every scoped key under
    // it — issue #615, reached through a different connector.
    const first = addConnection({
      baseUrl: "http://127.0.0.1:49221",
      connector: { kind: "ssh", target },
    });

    resetConnections();
    const second = addConnection({
      baseUrl: "http://127.0.0.1:51873",
      connector: { kind: "ssh", target },
    });

    expect(second).toBe(first);
  });

  it("is a different host when the tunnel goes somewhere else", () => {
    const acme = addConnection({
      baseUrl: "http://127.0.0.1:49221",
      connector: { kind: "ssh", target },
    });
    const beam = addConnection({
      baseUrl: "http://127.0.0.1:49222",
      connector: { kind: "ssh", target: { destination: "beam-vps", remotePort: 8080 } },
    });
    const otherPort = addConnection({
      baseUrl: "http://127.0.0.1:49223",
      connector: { kind: "ssh", target: { destination: "acme-vps", remotePort: 9090 } },
    });

    expect(new Set([acme, beam, otherPort]).size).toBe(3);
  });
});

describe("which connectors a runtime can offer", () => {
  it("offers all four where a process can be started", () => {
    expect(availableConnectors(true)).toEqual(["local", "cloud", "remote", "ssh"]);
  });

  it("offers a browser only the two it can honour", () => {
    // `local` and `ssh` both need a process on this machine. A browser build
    // has no core to start one in, so a tab for either would do nothing.
    expect(availableConnectors(false)).toEqual(["cloud", "remote"]);
  });
});

describe("somebody with no host at all", () => {
  it("tells a desktop operator what went wrong, because something did", () => {
    // The host inside the application did not start — usually another copy of
    // it holding the data root, which is a thing they can go and fix.
    const copy = firstHostCopy(true);
    expect(copy.title).toBe("No host to show");
    expect(copy.body).toContain("didn't start");
  });

  it("does not tell a hub that a host it never had failed to start", () => {
    // A hub's own origin serves assets and nothing else, so a new one holds
    // zero connections and always did. The desktop's words turn a first run
    // into a fault report about a computer that was never going to run one.
    const copy = firstHostCopy(false);
    expect(copy.body).not.toContain("didn't start");
    expect(copy.body).toContain("Choose where your company runs");
  });

  it("gives both a control, rather than naming one elsewhere", () => {
    // What this screen used to do was point at the switcher. A dead end that
    // describes its own exit is still a dead end.
    expect(firstHostCopy(true).action).toBeTruthy();
    expect(firstHostCopy(false).action).toBeTruthy();
  });

  it("offers the desktop an action, not a choice of somewhere else to run", () => {
    // One machine runs one host, so "where should this run" has a single answer
    // there — and the body must stop offering the alternative it no longer has.
    const copy = firstHostCopy(true);
    expect(copy.action).toBe("Start the host on this computer");
    expect(copy.body).not.toContain("somewhere else");
    // The hub still chooses: it runs no host of its own, so the answer is
    // genuinely elsewhere and the picker stays.
    expect(firstHostCopy(false).action).toBe("Choose where to run");
  });
});

describe("waiting for a hibernating tenant", () => {
  const cloud = { kind: "cloud", tenant: "acme" } as const;

  it("keeps waiting on a cloud tenant that has not answered yet", () => {
    expect(keepWaking(cloud, "down", 0)).toBe(true);
    expect(keepWaking(cloud, "down", CLOUD_WAKE_WINDOW_MS - 1)).toBe(true);
  });

  it("gives up once the window has passed", () => {
    expect(keepWaking(cloud, "down", CLOUD_WAKE_WINDOW_MS)).toBe(false);
  });

  it("does not wait on a host that answered and refused", () => {
    // `unauthenticated` is an answer: the tenant is awake. Retrying it would
    // hide the sign-in the operator has to do behind a spinner.
    expect(keepWaking(cloud, "unauthenticated", 0)).toBe(false);
  });

  it("does not wait on any other connector", () => {
    // Nothing is waking these up. A local host that is not listening is not
    // listening, and a dropped tunnel is a tunnel to report.
    expect(keepWaking({ kind: "local" }, "down", 0)).toBe(false);
    expect(keepWaking({ kind: "remote" }, "down", 0)).toBe(false);
    expect(
      keepWaking({ kind: "ssh", target: { destination: "vps", remotePort: 8080 } }, "down", 0),
    ).toBe(false);
  });

  it("backs off, and stops backing off", () => {
    expect(wakeRetryDelay(0)).toBe(1_000);
    expect(wakeRetryDelay(1)).toBe(2_000);
    expect(wakeRetryDelay(99)).toBe(WAKE_RETRY_CEILING_MS);
  });
});
