import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const indexHtml = readFileSync(resolve(process.cwd(), "index.html"), "utf8");
const loader = readFileSync(resolve(process.cwd(), "public/openpanel-init.js"), "utf8");
const tauriConfig = readFileSync(
  resolve(process.cwd(), "../crates/opencompany-app/tauri.conf.json"),
  "utf8",
);

describe("OpenPanel console analytics", () => {
  it("loads the configured browser client with automatic console tracking", () => {
    expect(indexHtml).toContain('src="/openpanel-init.js"');
    expect(loader).toContain('src = "https://openpanel.dev/op1.js"');
    expect(loader).toContain('apiUrl: "https://panel.tinyhumans.ai/api"');
    expect(loader).toContain('clientId: "afe8ec4e-0a6a-427a-aa22-49cbbf137d0a"');
    expect(loader).toContain("trackScreenViews: true");
    expect(loader).toContain("trackOutgoingLinks: true");
    expect(loader).toContain("trackAttributes: true");
    expect(loader).toContain('window.op("init"');
  });

  it("permits exactly the required OpenPanel origins in the desktop webview", () => {
    expect(tauriConfig).toContain("script-src 'self' https://openpanel.dev");
    expect(tauriConfig).toContain("connect-src 'self' ipc: http://ipc.localhost https://panel.tinyhumans.ai");
  });
});
