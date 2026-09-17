import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";

const frontendRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const indexHtml = readFileSync(resolve(frontendRoot, "index.html"), "utf8");
const loader = readFileSync(resolve(frontendRoot, "public/openpanel-init.js"), "utf8");
const tauriConfig = readFileSync(
  resolve(frontendRoot, "../crates/opencompany-app/tauri.conf.json"),
  "utf8",
);

describe("OpenPanel console analytics", () => {
  beforeEach(() => {
    delete window.op;
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    delete window.OPENCOMPANY_CONFIG;
    document.head.querySelectorAll('script[src="https://openpanel.dev/op1.js"]').forEach((script) => {
      script.remove();
    });
  });

  function runLoader(analytics?: boolean): void {
    if (analytics === undefined) {
      new Function(loader)();
      return;
    }
    Object.defineProperty(window, "OPENCOMPANY_CONFIG", {
      configurable: true,
      value: { analytics },
    });
    new Function(loader)();
  }

  it("does not install a client or script without explicit opt-in", () => {
    runLoader();

    expect(window.op).toBeUndefined();
    expect(document.head.querySelector('script[src="https://openpanel.dev/op1.js"]')).toBeNull();
  });

  it("stays silent when analytics is explicitly disabled", () => {
    runLoader(false);

    expect(window.op).toBeUndefined();
    expect(document.head.querySelector('script[src="https://openpanel.dev/op1.js"]')).toBeNull();
  });

  it("stays silent in the Tauri desktop webview", () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });

    runLoader(true);

    expect(window.op).toBeUndefined();
    expect(document.head.querySelector('script[src="https://openpanel.dev/op1.js"]')).toBeNull();
  });

  it("installs the configured client and script after explicit opt-in", () => {
    runLoader(true);

    expect(window.op).toBeDefined();
    expect(window.op?.q).toContainEqual([
      "init",
      {
        apiUrl: "https://panel.tinyhumans.ai/api",
        clientId: "afe8ec4e-0a6a-427a-aa22-49cbbf137d0a",
        trackScreenViews: false,
        trackOutgoingLinks: false,
        trackAttributes: false,
      },
    ]);
    expect(document.head.querySelector('script[src="https://openpanel.dev/op1.js"]')).not.toBeNull();
  });

  it("loads the configured browser client only after explicit opt-in", () => {
    expect(indexHtml).toContain('src="/openpanel-init.js"');
    expect(loader).toContain('src = "https://openpanel.dev/op1.js"');
    expect(loader).toContain('apiUrl: "https://panel.tinyhumans.ai/api"');
    expect(loader).toContain('clientId: "afe8ec4e-0a6a-427a-aa22-49cbbf137d0a"');
    expect(loader).toContain("window.OPENCOMPANY_CONFIG?.analytics !== true");
    expect(loader).toContain("trackScreenViews: false");
    expect(loader).toContain("trackOutgoingLinks: false");
    expect(loader).toContain("trackAttributes: false");
    expect(loader).toContain('window.op("init"');
  });

  it("permits exactly the required OpenPanel origins in the desktop webview", () => {
    expect(tauriConfig).toContain("script-src 'self'");
    expect(tauriConfig).toContain("connect-src 'self' ipc: http://ipc.localhost");
    expect(tauriConfig).not.toContain("https://openpanel.dev");
    expect(tauriConfig).not.toContain("https://panel.tinyhumans.ai");
  });
});
