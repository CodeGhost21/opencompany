import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const frontendRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const indexHtml = readFileSync(resolve(frontendRoot, "index.html"), "utf8");
const loader = readFileSync(resolve(frontendRoot, "public/openpanel-init.js"), "utf8");
const tauriConfig = readFileSync(
  resolve(frontendRoot, "../crates/opencompany-app/tauri.conf.json"),
  "utf8",
);

describe("OpenPanel console analytics", () => {
  it("loads the configured browser client only after explicit opt-in", () => {
    expect(indexHtml).toContain('src="/openpanel-init.js"');
    expect(loader).toContain('src = "https://openpanel.dev/op1.js"');
    expect(loader).toContain('apiUrl: "https://panel.tinyhumans.ai/api"');
    expect(loader).toContain('clientId: "afe8ec4e-0a6a-427a-aa22-49cbbf137d0a"');
    expect(loader).toContain("window.OPENCOMPANY_CONFIG?.analytics !== true");
    expect(loader).toContain("trackScreenViews: true");
    expect(loader).toContain("trackOutgoingLinks: false");
    expect(loader).toContain("trackAttributes: false");
    expect(loader).toContain('window.op("init"');
  });

  it("permits exactly the required OpenPanel origins in the desktop webview", () => {
    expect(tauriConfig).toContain("script-src 'self'");
    expect(tauriConfig).toContain("connect-src 'self' ipc: http://ipc.localhost");
  });
});
