import { defineConfig } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { LIVE_BRAIN, MCP_FIXTURE_BIND, MOCK_BRAIN_BIND } from "./test/e2e/capabilities";

// `package.json` is `"type": "module"`, so this file is ESM and `__dirname`
// does not exist here — it type-checks against `@types/node` and then throws at
// load. Which is the whole lesson of #406 in one line.
const here = dirname(fileURLToPath(import.meta.url));

/**
 * Playwright config for the operator-console end-to-end suite.
 *
 * The suite drives a *running* OpenCompany host — the Rust binary serving the
 * built `frontend/dist` via `OPENCOMPANY_CONSOLE_DIR`. There are two ways to
 * get one, and `PW_BASE_URL` alone decides which applies:
 *
 * **`PW_BASE_URL` set** — you brought your own host and this config touches
 * nothing else: no `webServer`, and `PW_STORAGE_STATE` keeps its exact former
 * meaning (unset ⇒ no `globalSetup` ⇒ no sign-in). Unchanged contract.
 *
 * **`PW_BASE_URL` unset** — the config brings a host up itself through
 * `test/e2e/host.sh` and authenticates against it, so `npm run e2e` is the
 * whole command. It needs `target/debug/opencompany` to exist; `host.sh` says
 * so, and says how, rather than failing obscurely.
 *
 * That second path exists because of issue #406. This suite is typechecked by
 * CI (`npm run typecheck:e2e`) and executed by nobody, which is how
 * `workflow-edit-delete.spec.ts` came to reference a fixture that was never
 * committed and stay red indefinitely without one report. Typechecking a spec
 * proves it compiles, not that it holds. Running it was possible before this —
 * but only if you already knew which four environment variables to set, and a
 * suite that is hard to start is a suite nobody starts.
 *
 * CI runs it in two lanes (#428, #467): `Console E2E` against a default-feature
 * host, and `Console E2E (live brain)` against a feature-gated one with the
 * fixtures below standing behind it.
 *
 * # `PW_LIVE_BRAIN=1` — the second lane
 *
 * Four specs need an agent that actually executes, which needs a host built
 * with `--features openhuman,tinycortex,mcp` **and** something for that harness
 * to think with. When we manage the host, this config supplies the second half:
 * it starts `test/e2e/mock-brain.mjs` and `test/e2e/mcp-server.mjs` alongside
 * the host, and hands the host the inference endpoint through
 * `PW_HOST_PASSTHROUGH` — the escape hatch `host.sh`'s allowlist keeps for
 * exactly this. The binary is still yours to supply (`PW_HOST_BINARY`, or
 * `target/debug/opencompany` built with those features); nothing here can tell
 * a feature-gated host from a default one, which is why the flag is a
 * declaration rather than a probe. See `test/e2e/capabilities.ts`.
 *
 * Against a host you brought yourself (`PW_BASE_URL`), the flag still enables
 * the four specs but the fixtures are yours to start too — this config will not
 * reconfigure a host it did not launch.
 */

const providedBaseURL = process.env.PW_BASE_URL;

/** Whether this config is responsible for the host, as opposed to driving yours. */
const managesHost = !providedBaseURL;

const baseURL = providedBaseURL || "http://127.0.0.1:8080";

/**
 * Where the shared signed-in session lands. Defaulted only when we manage the
 * host: against a host we started, every spec needs a session and there is no
 * reason to make the caller name a path for it. Against yours, an unset
 * variable keeps its existing meaning.
 */
const storageState =
  process.env.PW_STORAGE_STATE ||
  (managesHost ? resolve(here, "../target/e2e/storage-state.json") : undefined);

// `global-setup.ts` writes that file but does not create its directory, and
// `target/e2e/` does not exist on a clean checkout.
if (storageState) {
  mkdirSync(dirname(storageState), { recursive: true });
}

/** Whether this run also brings up the live-brain fixtures (issue #467). */
const managesFixtures = managesHost && LIVE_BRAIN;

/**
 * What the host is told about inference, and which of those names `host.sh` is
 * allowed to forward.
 *
 * The bearer is a placeholder and nothing checks it: the host needs *a*
 * credential only because a configured credential is what makes it choose a
 * live harness over the offline echo brain (`harness_inference_from_env`).
 *
 * `PW_HOST_PASSTHROUGH` is the load-bearing line. `host.sh` starts the host
 * from an empty environment and copies in an allowlist, so a variable set here
 * and not named there reaches nothing.
 */
const inferenceEnv: Record<string, string> = managesFixtures
  ? {
      OPENCOMPANY_INFERENCE_KEY: "mock-brain",
      OPENCOMPANY_INFERENCE_URL: `http://${MOCK_BRAIN_BIND}/v1`,
      PW_HOST_PASSTHROUGH: "OPENCOMPANY_INFERENCE_KEY OPENCOMPANY_INFERENCE_URL",
    }
  : {};

/**
 * One `webServer` entry per fixture, ahead of the host.
 *
 * Both are plain Node scripts with no dependencies, and both answer `/healthz`,
 * which is what lets Playwright wait for them rather than leaving the first
 * agent turn to discover a backend that is not up yet. Ordering is a courtesy
 * only — the host reads its inference URL at boot but does not dial it until a
 * turn runs, well after every server here is ready.
 */
const fixtureServers = managesFixtures
  ? [
      {
        command: `node ./test/e2e/mock-brain.mjs --bind ${MOCK_BRAIN_BIND}`,
        url: `http://${MOCK_BRAIN_BIND}/healthz`,
        reuseExistingServer: !process.env.CI,
        timeout: 30_000,
        stdout: "pipe" as const,
        stderr: "pipe" as const,
      },
      {
        command: `node ./test/e2e/mcp-server.mjs --bind ${MCP_FIXTURE_BIND}`,
        url: `http://${MCP_FIXTURE_BIND}/healthz`,
        reuseExistingServer: !process.env.CI,
        timeout: 30_000,
        stdout: "pipe" as const,
        stderr: "pipe" as const,
      },
    ]
  : [];

export default defineConfig({
  testDir: "./test/e2e",
  globalSetup: storageState ? "./test/e2e/global-setup.ts" : undefined,
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  reporter: [["list"]],
  use: {
    baseURL,
    storageState,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  webServer: managesHost
    ? [
        ...fixtureServers,
        {
          command: "./test/e2e/host.sh",
          url: `${baseURL}/healthz`,
          // A host already listening on the default bind is almost always the
          // one you are developing against, so drive it rather than fight it
          // for the port. In CI that would mean silently testing something
          // unknown.
          reuseExistingServer: !process.env.CI,
          // Covers a cold `npm run build` for the console bundle plus the
          // host's own boot, with room to spare.
          timeout: 180_000,
          stdout: "pipe" as const,
          stderr: "pipe" as const,
          env: {
            PW_HOST_BIND: new URL(baseURL).host,
            ...inferenceEnv,
          },
        },
      ]
    : undefined,
});
