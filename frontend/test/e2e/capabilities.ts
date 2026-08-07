/**
 * What the host under test can actually do, so a spec that needs more than the
 * default build can say so instead of failing (issue #428).
 *
 * # Why these are opt-in variables and not a probe of the host
 *
 * A probe would be better, and there is very nearly one: `GET /tiny` reports
 * the vendored runtime modules. It cannot be used for this. `openhuman` is
 * reported as `enabled: true` unconditionally — the field describes the
 * vendored *checkout*, not the `openhuman` cargo feature — so a default build
 * answers exactly like a feature-gated one. A probe that cannot distinguish the
 * two would skip nothing, or skip everything, and would look authoritative
 * while doing it.
 *
 * So the caller declares what it brought. That is honest about where the
 * knowledge lives: whoever built the binary and started the inference backend
 * is the only party that knows.
 *
 * # The skips now have a lane behind them (issue #467)
 *
 * Four of the suite's best specs sit behind `LIVE_BRAIN`, and they are the ones
 * that exercise the product rather than the console's own rendering. CI runs
 * them: `Console E2E (live brain)` builds `--features openhuman,tinycortex,mcp`
 * and `playwright.config.ts` stands up `mock-brain.mjs` and `mcp-server.mjs`
 * behind it. The default-feature `Console E2E` lane still skips them, because
 * on a host without the harness the thing they test is not compiled in — that
 * skip is a true statement about that host, not a debt.
 */

/**
 * A host built with `--features openhuman,tinycortex,mcp` **and** an inference
 * backend behind it — either the mock that echoes `__MOCK_LLM__` or one whose
 * tool choices are scripted (`SPAWNONE`).
 *
 * Set `PW_LIVE_BRAIN=1` when both are true. Without them the agent harness is
 * not compiled in at all, so a sent message is answered by nothing, a workflow
 * node with no inference source never runs, and no orchestrator exists to open
 * a card.
 */
export const LIVE_BRAIN = process.env.PW_LIVE_BRAIN === "1";

/**
 * Whether the run brings up its own host, as opposed to driving one you started
 * and named with `PW_BASE_URL`. Mirrors `playwright.config.ts`'s `managesHost`,
 * and settles the same question for the fixtures: a host this run launched was
 * pointed at fixtures this run also launched, so their addresses are known here
 * without being restated.
 */
const MANAGES_HOST = !process.env.PW_BASE_URL;

/**
 * Where `mock-brain.mjs` listens when this run starts it. The host is handed
 * `http://…/v1` as its `OPENCOMPANY_INFERENCE_URL`; override with
 * `PW_MOCK_BRAIN_BIND` if 8099 is taken.
 */
export const MOCK_BRAIN_BIND = process.env.PW_MOCK_BRAIN_BIND || "127.0.0.1:8099";

/** Where `mcp-server.mjs` listens when this run starts it. */
export const MCP_FIXTURE_BIND = process.env.PW_MCP_FIXTURE_BIND || "127.0.0.1:8098";

/**
 * The **URL** of an MCP server an agent may be told to call.
 *
 * `PW_MCP_SERVER` was retired in #414, and rightly: it carried a path to a
 * *stdio* server, which this host rejects outright (`src/company/mcp.rs` refuses
 * any declaration with a `command`), so it gated a spec that could not have
 * passed even had the fixture existed. `mcp.spec.ts` now drives the console
 * surface against a default host and needs nothing.
 *
 * It comes back here carrying a URL, for the half that page cannot reach: an
 * *agent* calling a tool on a real server over the real transport
 * (`mcp-agent.spec.ts`), which needs the harness and therefore the live-brain
 * lane. That lane starts `mcp-server.mjs`, which is why this is defaulted only
 * when the run manages the host. Against a host you brought, name your own.
 */
export const MCP_SERVER =
  process.env.PW_MCP_SERVER ||
  (LIVE_BRAIN && MANAGES_HOST ? `http://${MCP_FIXTURE_BIND}/mcp` : undefined);

/** The reason string a `LIVE_BRAIN` skip carries, so no skip is ever bare. */
export const LIVE_BRAIN_REASON =
  "needs a --features openhuman,tinycortex,mcp host plus an inference backend; " +
  "set PW_LIVE_BRAIN=1 to run. The `Console E2E (live brain)` CI lane does " +
  "(issue #467).";
