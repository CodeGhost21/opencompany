// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  type AppUpdateInfo,
  checkAppUpdate,
  downloadAppUpdate,
  installAppUpdate,
} from "@/api/transport/desktop";

/**
 * Which of the three update calls is allowed to fail loudly, and which is not.
 *
 * The check runs on a timer nobody started, against an endpoint that is
 * routinely unreachable — a laptop on a train, a company behind a proxy, a
 * release that has not been cut yet. So it answers "no update" for every
 * failure, including the one where the desktop shell is older than this console
 * bundle and has no such command at all.
 *
 * The download is the opposite: it only runs because a check just said there is
 * something to fetch, so a failure there is real and the operator sees it.
 */

interface Bridge {
  __TAURI__?: { core: { invoke: (command: string) => Promise<unknown> } };
}

const AN_UPDATE: AppUpdateInfo = {
  currentVersion: "0.1.0",
  available: true,
  availableVersion: "0.2.0",
  notes: "Fixes the thing.",
};

function shellAnswering(invoke: (command: string) => Promise<unknown>): void {
  (window as unknown as Bridge).__TAURI__ = { core: { invoke } };
}

afterEach(() => {
  delete (window as unknown as Bridge).__TAURI__;
  vi.restoreAllMocks();
});

describe("in a browser, which cannot replace itself", () => {
  it("reports no update rather than reaching for a bridge that is not there", async () => {
    await expect(checkAppUpdate()).resolves.toBeNull();
    await expect(downloadAppUpdate()).resolves.toBeNull();
  });

  it("refuses an install outright", async () => {
    // Unreachable from the console — the banner never renders here — but it is
    // the one call with no harmless answer, so it says so instead of resolving.
    await expect(installAppUpdate()).rejects.toThrow(/desktop/i);
  });
});

describe("against a desktop shell", () => {
  it("passes the offer through", async () => {
    shellAnswering(async () => AN_UPDATE);

    await expect(checkAppUpdate()).resolves.toEqual(AN_UPDATE);
  });

  it("treats a shell too old to know the command as no update", async () => {
    // The console bundle can be newer than the shell around it: a developer
    // running the dev server against a `cargo build` from last week is an
    // ordinary Tuesday. An unknown command rejects, and that must degrade to
    // silence rather than to an error banner on every launch.
    vi.spyOn(console, "debug").mockImplementation(() => {});
    shellAnswering(async (command) => {
      throw new Error(`unknown command: ${command}`);
    });

    await expect(checkAppUpdate()).resolves.toBeNull();
  });

  it("reports a failed download, because somebody asked for that one", async () => {
    shellAnswering(async () => {
      throw new Error("the update could not be downloaded: connection reset");
    });

    await expect(downloadAppUpdate()).rejects.toThrow(/connection reset/);
  });
});
