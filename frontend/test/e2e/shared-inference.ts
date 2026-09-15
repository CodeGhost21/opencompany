import type { APIRequestContext } from "@playwright/test";

/**
 * Puts the shared E2E company's inference back into a state every other spec
 * can run against, after a spec that connected a provider to it.
 *
 * ## Why a spec that adds a provider owes the rest of the run this
 *
 * `playwright.config.ts` brings up **one** host serving **one** company
 * (`companies/e2e_harness`) for the whole run — every spec file that is not
 * first-run / Euler / live-LLM / visual drives that same company. A provider
 * connected by one spec is therefore visible to every spec that runs after it,
 * and two keys-rework decisions (issue #2306, `docs/key-reworks/README.md`)
 * make that far more than a stray row in a list:
 *
 * - **D-first-default (X1):** the first provider a company ever connects
 *   becomes its default automatically, with no opt-out. Every turn whose
 *   workload has no explicit route resolves through the default.
 * - **D-never-clear-default (X14):** deleting or disabling that provider never
 *   clears `inference/default`. A turn then fails closed — "The company
 *   default uses …, which is removed" — and there is no route that unsets the
 *   default; the only supported way back is to pick another provider, or to
 *   route the workloads explicitly.
 *
 * So a spec that connects a row pointing at the discard port and leaves it
 * turns every later agent turn in the run into `inference request failed …
 * 127.0.0.1:9/v1/chat/completions`, and a spec that deletes it afterwards
 * turns them into the fail-closed sentence instead. Thirty-odd unrelated specs
 * went red that way on the live-brain lane.
 *
 * ## What this does
 *
 * 1. Deletes each slug in `slugs`, confirming the in-use guard (a delete of
 *    the default, or of a pinned provider, is refused with `409 in_use`
 *    otherwise). A slug already gone answers 404 and is ignored.
 * 2. If the company now has a default at all — the pristine harness company
 *    has none, so any default was written by a spec, and after step 1 it
 *    names a provider that no longer exists — routes every workload through
 *    `managed` explicitly. That is the platform-injected brain, which on this
 *    run is `mock-brain.mjs` / `live-brain-proxy.mjs`, and it is exactly what
 *    the console's Managed mode writes; an explicit route outranks the default
 *    on the turn path (`resolve_effective_for_tier`), so turns think again.
 *
 * Call it from a `test.afterEach` hook, **not** from a `finally` inside the
 * test: when a test hits its timeout Playwright abandons the test function
 * outright and an in-body `finally` never runs, which is how the leak
 * happened in the first place. A hook runs after a timed-out test, with a
 * request context that still works.
 */
export async function restoreSharedInference(request: APIRequestContext, slugs: string[]) {
  for (const slug of slugs) {
    await request
      .delete(`/api/v1/company/inference/providers/${slug}?confirmInUse=true`)
      .catch(() => {});
  }

  const status = await request.get("/api/v1/company/inference").catch(() => null);
  if (!status || !status.ok()) return;
  const body = (await status.json().catch(() => null)) as {
    defaultChoice?: { provider?: string } | null;
  } | null;
  if (!body?.defaultChoice?.provider) return;

  await request
    .put("/api/v1/company/inference/routes", {
      data: {
        routes: {
          "chat-v1": "managed",
          "reasoning-v1": "managed",
          "agentic-v1": "managed",
          "vision-v1": "managed",
        },
      },
    })
    .catch(() => {});
}
