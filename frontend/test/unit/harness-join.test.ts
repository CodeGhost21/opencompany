import { describe, expect, it } from "vitest";

import type { AcpHarnessStatus } from "@/api/transport/desktop";
import type { HarnessDto } from "@/api/types";
import {
  desktopHarnessId,
  harnessAction,
  isChecking,
  isUsableHere,
  joinHarnesses,
  readinessNote,
  withReadiness,
} from "@/lib/harnesses";

/**
 * The join behind the External harnesses page (issue #1245's detected-harness
 * follow-up).
 *
 * Every case here is one where being subtly wrong looks entirely normal on
 * screen: telling somebody to install a CLI they already have, promising a
 * turn will run on a machine that was never asked, or reporting a managed
 * harness as missing because no binary sits on PATH for it.
 */

const declared = (over: Partial<HarnessDto> = {}): HarnessDto => ({
  id: "main",
  kind: "built_in",
  default: true,
  detected: false,
  runsHere: true,
  ...over,
});

// `runsHere: true` is the desktop-hosted shape: the host serving this company
// is the machine the console is running on, so its survey is the right answer.
const cli = (over: Partial<HarnessDto> = {}): HarnessDto => ({
  id: "claude",
  kind: "acp",
  default: false,
  detected: true,
  agent: "claude",
  transport: "local",
  runsHere: true,
  ...over,
});

const probe = (over: Partial<AcpHarnessStatus> = {}): AcpHarnessStatus => ({
  id: "claude",
  label: "Claude Code",
  readiness: { state: "ready" },
  ...over,
});

/** Where a confirmation would say the adapter turned out to be. */
const ADAPTER = "/usr/local/bin/claude-agent-acp";

describe("joining what the company can bind against what this machine has", () => {
  it("attaches this machine's readiness to a local CLI by id", () => {
    const [row] = joinHarnesses([cli()], [probe()]);
    expect(row.label).toBe("Claude Code");
    expect(row.readiness).toEqual({ state: "ready" });
    expect(row.declared).toBe(false);
  });

  it("carries no path out of the survey, because the survey looks nothing up", () => {
    // The inversion: a harness is not judged by a PATH walk any more, it is
    // judged by being started. So the list arrives with no resolved binaries —
    // a path shows up only alongside the verdict that makes it worth quoting.
    const [row] = joinHarnesses([cli()], [probe()]);
    expect(row.path).toBeUndefined();
    expect(withReadiness([row], "claude", { state: "ready" }, ADAPTER)[0].path).toBe(ADAPTER);
  });

  it("marks a manifest entry as declared and a synthesized one as not", () => {
    const [managed, detected] = joinHarnesses([declared(), cli()], null);
    expect(managed.declared).toBe(true);
    expect(detected.declared).toBe(false);
  });

  it("leaves readiness undefined in a browser rather than claiming not-installed", () => {
    // The distinction the whole page rests on: nothing probed is not the same
    // fact as nothing found, and collapsing them tells someone to install a
    // CLI that is sitting on their machine already.
    const [row] = joinHarnesses([cli()], null);
    expect(row.readiness).toBeUndefined();
    expect(isUsableHere(row)).toBe(false);
    expect(readinessNote(row)).toContain("desktop app");
  });

  it("never probes a managed harness against this machine's PATH", () => {
    // A `built_in` harness has no CLI at all, so a survey miss says nothing
    // about it — reporting "not installed" would be a category error.
    const [row] = joinHarnesses([declared()], []);
    expect(row.readiness).toBeUndefined();
    expect(isUsableHere(row)).toBe(true);
    expect(readinessNote(row)).toContain("nothing to install");
  });

  it("never probes a remote-runner harness against this machine", () => {
    const [row] = joinHarnesses(
      [cli({ id: "shared", transport: "runner", detected: false })],
      [probe({ id: "shared" })],
    );
    expect(row.readiness).toBeUndefined();
    expect(readinessNote(row)).toContain("remote machine");
  });

  it("joins a declared harness by its ACP agent, not its own id", () => {
    // A manifest names its harnesses whatever it likes; this machine's probe
    // catalogue only knows `claude` and `codex`. Keying the join on the
    // harness id makes a perfectly ready CLI read as "Desktop only".
    const [row] = joinHarnesses([cli({ id: "laptop", agent: "claude" })], [probe()]);
    expect(row.id).toBe("laptop");
    expect(row.readiness).toEqual({ state: "ready" });
    expect(isUsableHere(row)).toBe(true);
  });

  it("falls back to the raw id when this machine has no friendly name", () => {
    const [row] = joinHarnesses([cli()], []);
    expect(row.label).toBe("claude");
  });

  it("preserves the host's order, declared before detected", () => {
    const rows = joinHarnesses([declared(), cli({ id: "codex" }), cli()], null);
    expect(rows.map((r) => r.id)).toEqual(["main", "codex", "claude"]);
  });
});

describe("the checking state, before a handshake has settled it", () => {
  it("is never treated as usable", () => {
    // The whole point of the second phase: a credential file existing is not
    // the same claim as the CLI starting, so nothing may act on it until the
    // handshake answers.
    const [row] = joinHarnesses([cli()], [probe({ readiness: { state: "checking" } })]);
    expect(isChecking(row)).toBe(true);
    expect(isUsableHere(row)).toBe(false);
    // Must not claim sign-in: that is precisely what this phase has not
    // established yet, and the old copy asserted it.
    expect(readinessNote(row)).not.toMatch(/signed in/i);
    expect(readinessNote(row)).toContain("Starting it");
  });

  it("is replaced in place when its confirmation lands", () => {
    const rows = joinHarnesses(
      [cli(), cli({ id: "codex", agent: "codex" })],
      [probe({ readiness: { state: "checking" } }), probe({ id: "codex", readiness: { state: "checking" } })],
    );
    const settled = withReadiness(rows, "claude", { state: "ready" });

    expect(settled.find((r) => r.id === "claude")?.readiness).toEqual({ state: "ready" });
    expect(settled.find((r) => r.id === "codex")?.readiness).toEqual({ state: "checking" });
    // The input is not mutated — `withReadiness` returns a new list, which is
    // what lets a stale probe be dropped rather than applied.
    expect(rows[0].readiness).toEqual({ state: "checking" });
  });

  it("drops an answer for a row the current list no longer has", () => {
    // A probe from a superseded load must not resurrect a row: the newer list
    // simply has no such id, so the swap is a no-op rather than an insert.
    const rows = joinHarnesses([cli()], null);
    expect(withReadiness(rows, "gone", { state: "ready" })).toEqual(rows);
  });

  it("settles into a real verdict, including the failure the probe made reachable", () => {
    const rows = joinHarnesses([cli()], [probe({ readiness: { state: "checking" } })]);
    const failed = withReadiness(rows, "claude", {
      state: "spawnFailed",
      reason: "it did not answer within 20s of starting",
    });
    expect(isChecking(failed[0])).toBe(false);
    expect(isUsableHere(failed[0])).toBe(false);
    expect(readinessNote(failed[0])).toContain("20s");
  });
});

describe("whether a harness can take a turn here", () => {
  it("is true only for a ready CLI", () => {
    expect(isUsableHere(joinHarnesses([cli()], [probe()])[0])).toBe(true);
    for (const state of ["notInstalled", "notSignedIn"] as const) {
      const [row] = joinHarnesses([cli()], [probe({ readiness: { state } })]);
      expect(isUsableHere(row)).toBe(false);
    }
  });

  it("names the fix for each unready state", () => {
    // Built the way the page builds it: joined first, then settled by a
    // confirmation that carries both the verdict and — on the states that
    // quote it — where the adapter turned out to be.
    const note = (readiness: AcpHarnessStatus["readiness"], path?: string) =>
      readinessNote(
        withReadiness(joinHarnesses([cli()], [probe()]), "claude", readiness, path)[0],
      );

    expect(note({ state: "notInstalled" })).toContain("install");

    // The state that exists because its absence produced the worst message in
    // the feature: telling somebody to install Claude Code on the machine they
    // run Claude Code on. It must name the install it found *and* the one
    // package that closes the gap, and must not read as "you have not got it".
    const adapter = note({
      state: "adapterMissing",
      cli: "/Users/x/.local/bin/claude",
      package: "@agentclientprotocol/claude-agent-acp",
    });
    expect(adapter).toContain("/Users/x/.local/bin/claude");
    expect(adapter).not.toMatch(/not found|not installed/i);
    // No command to copy: the adapter is this app's dependency, so this app
    // installs it. The instruction is a button, not a paste.
    expect(adapter).not.toMatch(/npm install/i);
    expect(note({ state: "notSignedIn" })).toContain("sign in");
    // A spawn failure carries the harness's own reason verbatim — more
    // accurate than anything this layer could guess — plus the path, so the
    // reason is attributable to a specific binary.
    const failed = note({ state: "spawnFailed", reason: "bad flag --acp" }, ADAPTER);
    expect(failed).toContain("bad flag --acp");
    expect(failed).toContain(ADAPTER);
  });
});

describe("what this app can do about a row, as opposed to say", () => {
  const rowWith = (readiness: AcpHarnessStatus["readiness"]) =>
    withReadiness(joinHarnesses([cli()], [probe()]), "claude", readiness)[0];

  it("offers an install exactly when the CLI is here and the add-on is not", () => {
    expect(
      harnessAction(
        rowWith({
          state: "adapterMissing",
          cli: "/usr/local/bin/claude",
          package: "@agentclientprotocol/claude-agent-acp",
        }),
      ),
    ).toBe("install");
  });

  it("offers an update for an install of ours that is behind the pin", () => {
    expect(harnessAction(rowWith({ state: "adapterOutdated", found: "0.1.0", want: "0.70.0" }))).toBe(
      "update",
    );
  });

  it("offers nothing when installing cannot help", () => {
    // Node missing is the one that matters. An Install button here would fetch
    // an adapter that still could not start, and reporting *that* as a failed
    // installation is how block/buzz sent people to reinstall a working
    // adapter. The other states need a person to act, not this app.
    for (const readiness of [
      { state: "nodeMissing" },
      { state: "notInstalled" },
      { state: "notSignedIn" },
      { state: "ready" },
      { state: "checking" },
    ] as const) {
      expect(harnessAction(rowWith(readiness))).toBe("none");
    }
  });

  it("never offers an action on a harness this machine does not run", () => {
    // A managed harness has no CLI, and a runner one lives elsewhere —
    // installing anything here would be installing it on the wrong machine.
    expect(harnessAction(joinHarnesses([declared()], [])[0])).toBe("none");
    expect(
      harnessAction(
        joinHarnesses([cli({ id: "shared", transport: "runner" })], [probe({ id: "shared" })])[0],
      ),
    ).toBe("none");
  });

  it("names Node, not a failed installation, when the runtime is missing", () => {
    // The whole point of the state: "installation failed" is unactionable and
    // was block/buzz's wording for it.
    const note = readinessNote(rowWith({ state: "nodeMissing" }));
    expect(note).toMatch(/node/i);
    expect(note).not.toMatch(/failed/i);
  });

  it("says which version is installed and which is wanted", () => {
    const note = readinessNote(rowWith({ state: "adapterOutdated", found: "0.1.0", want: "0.70.0" }));
    expect(note).toContain("0.1.0");
    expect(note).toContain("0.70.0");
  });
});

describe("which id the desktop is addressed by", () => {
  it("uses the ACP agent for a declared harness, and the id for a detected one", () => {
    // The two coincide for a detected row and diverge for a declared one, so
    // reaching for `.id` directly reads correctly in every test that only
    // covers detected harnesses — and then sends `laptop` to a shell whose
    // catalogue knows only `claude`, which comes back "not a harness this
    // build knows" and strands the row.
    const [declaredRow] = joinHarnesses([cli({ id: "laptop", agent: "claude" })], []);
    expect(desktopHarnessId(declaredRow)).toBe("claude");
    // The company binding key is untouched by any of this.
    expect(declaredRow.id).toBe("laptop");

    const [detectedRow] = joinHarnesses([cli()], []);
    expect(desktopHarnessId(detectedRow)).toBe("claude");
  });

  it("falls back to the id when a row names no agent", () => {
    // A `built_in` row has no agent at all. Nothing should ask the desktop
    // about it, but returning `undefined` here would turn a caller's mistake
    // into a malformed command rather than a harmless miss.
    const [managed] = joinHarnesses([declared()], []);
    expect(desktopHarnessId(managed)).toBe("main");
  });
});

describe("whose machine a harness actually runs on", () => {
  it("does not probe a declared local harness belonging to a remote host", () => {
    // The desktop connected to a hosted company. That company's manifest can
    // declare `transport = "local"` — local *to the host serving it*, not to
    // the laptop this console runs on. Probing it here reported Ready, and
    // offered to install an adapter, for a machine that will never run those
    // turns.
    const [row] = joinHarnesses([cli({ runsHere: false, detected: false })], [probe()]);
    expect(row.readiness).toBeUndefined();
    expect(isUsableHere(row)).toBe(false);
    expect(harnessAction(row)).toBe("none");
  });

  it("does not probe when the host is too old to say", () => {
    // Absent is not "yes". A host predating the field degrades to not probing,
    // which renders honestly as "can't say from here" — the opposite default
    // would reinstate the bug.
    const { runsHere: _dropped, ...withoutField } = cli();
    const [row] = joinHarnesses([withoutField], [probe()]);
    expect(row.readiness).toBeUndefined();
  });
});
