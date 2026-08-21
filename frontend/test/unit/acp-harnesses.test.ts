// @vitest-environment jsdom

import { describe, expect, it } from "vitest";

import { acpHarnesses } from "@/api/transport/desktop";

/**
 * The IPC half of the harness-management settings panel (issue #1245).
 *
 * Same shape as `local-instances.test.ts`'s "asking the core what it runs":
 * a browser has no local harnesses, and a shell built before this command
 * existed has to read differently from a machine with none installed, or an
 * older shell's settings page would report "you have no coding harnesses"
 * instead of "this shell predates the feature".
 */
describe("asking the core which coding harnesses this machine has", () => {
  function installBridge(answers: Record<string, unknown | Error>): void {
    (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
      core: {
        invoke: (command: string) => {
          const answer = answers[command];
          if (answer instanceof Error) return Promise.reject(answer);
          return Promise.resolve(answer ?? undefined);
        },
        Channel: class {
          onmessage: ((message: string) => void) | null = null;
        },
      },
    };
  }

  function uninstallBridge(): void {
    delete (window as unknown as { __TAURI__?: unknown }).__TAURI__;
  }

  it("answers with an empty catalogue in a browser", async () => {
    uninstallBridge();
    await expect(acpHarnesses()).resolves.toEqual([]);
  });

  it("tells a missing command apart from a machine with nothing installed", async () => {
    installBridge({ oc_acp_harnesses: new Error("Command oc_acp_harnesses not found") });
    await expect(acpHarnesses()).resolves.toBeNull();

    installBridge({ oc_acp_harnesses: [] });
    await expect(acpHarnesses()).resolves.toEqual([]);
    uninstallBridge();
  });

  it("passes the core's readiness catalogue through unchanged", async () => {
    const catalogue = [
      { id: "claude", label: "Claude Code", readiness: { state: "ready" }, path: "/usr/local/bin/claude-agent-acp" },
      { id: "codex", label: "Codex", readiness: { state: "notInstalled" } },
    ];
    installBridge({ oc_acp_harnesses: catalogue });

    await expect(acpHarnesses()).resolves.toEqual(catalogue);
    uninstallBridge();
  });
});
