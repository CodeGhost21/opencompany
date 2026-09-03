// The only module in the console that imports the Sentry SDK.
//
// Everything worth testing — the decision, the scrubber, the sanitizer — lives
// in `@/lib/crash-reporting`, which imports no SDK and is covered by the unit
// suite. This file is the wiring: what is switched on, what is switched off,
// and why. See `docs/spec/runtime/crash-reporting.md`.

import * as Sentry from "@sentry/react";

import {
  SMOKE_TEST_MESSAGE,
  resolveCrashReporting,
  sanitizeEvent,
} from "@/lib/crash-reporting";

/**
 * The release tag, computed once in `vite.config.ts` and substituted at build
 * time, so the string this bundle reports and the string the source-map upload
 * files under cannot drift. A release the two disagree about is a stack trace
 * Sentry never symbolicates and no error to explain why.
 *
 * Declared in `src/vite-env.d.ts`.
 */
const RELEASE: string = __SENTRY_RELEASE__;

/**
 * Initializes crash reporting, or does nothing at all.
 *
 * Call once, before the first render, so a crash during the first render is
 * reported rather than being the thing that prevents reporting. Silent unless
 * `VITE_SENTRY_DSN` is set to a usable DSN: no console warning, no thrown
 * error, no network. That is the state every local checkout and every CI run is
 * in, and a build that complained about it would train people to ignore it.
 */
export function initSentry(): void {
  const config = resolveCrashReporting(import.meta.env, RELEASE);
  if (!config) return;

  Sentry.init({
    dsn: config.dsn,
    environment: config.environment,
    release: config.release,

    // No IP address and no cookies.
    sendDefaultPii: false,
    // No performance tracing and no session replay. Both are large content
    // surfaces — a replay is a recording of the operator's screen, and spans
    // carry every request's URL and timing — and neither is what this is for.
    // Set to zero as well as left un-integrated, so enabling an integration
    // later cannot silently switch sampling on.
    tracesSampleRate: 0,
    replaysSessionSampleRate: 0,
    replaysOnErrorSampleRate: 0,

    // Every integration is opt-in. The default set includes several that
    // collect content — and two that this list has to put back, because
    // `defaultIntegrations: false` silently disables things that do not look
    // like integrations at all:
    defaultIntegrations: false,
    integrations: [
      // `ignoreErrors` below is consumed by THIS integration. Without it the
      // option is dead config that reads as a working filter.
      Sentry.inboundFiltersIntegration(),
      // `event.request.headers` comes from here, and Sentry derives `os` /
      // `browser` / `device` server-side by parsing the User-Agent in it —
      // so without this every event loses all platform context. `sanitizeEvent`
      // narrows the envelope back down to that one header.
      Sentry.httpContextIntegration(),
      // `window.onerror` and `window.onunhandledrejection`. This is what makes
      // an unhandled promise rejection a report; there is no hand-written
      // listener anywhere in this app, and there should not be.
      Sentry.globalHandlersIntegration(),
      // Wraps `setTimeout`/`requestAnimationFrame`/event listeners so a throw
      // inside one has a usable stack instead of `Script error`.
      Sentry.browserApiErrorsIntegration(),
      // `error.cause` chains, so a wrapped `ApiError` still names what failed.
      Sentry.linkedErrorsIntegration(),
      // One issue per bug rather than one per re-render.
      Sentry.dedupeIntegration(),
      Sentry.functionToStringIntegration(),
      // Breadcrumbs, narrowed to the ones that carry a shape rather than
      // content. `console` would ship anything anyone ever logged and `dom`
      // ships the text of the element clicked — which on this surface is
      // company data. What is left is method, URL and status, and
      // `sanitizeEvent` scrubs those.
      Sentry.breadcrumbsIntegration({
        console: false,
        dom: false,
        fetch: true,
        history: true,
        xhr: true,
      }),
    ],

    beforeSend: sanitizeEvent,

    // Non-actionable browser noise. Every one of these is something the page
    // recovers from on its own: a layout loop the browser already broke, a
    // request the user navigated away from, an extension's fetch shim.
    ignoreErrors: [
      "ResizeObserver loop",
      "Network request failed",
      "Failed to fetch",
      "Load failed",
      "AbortError",
    ],
  });

  // The one-shot pipeline check. Set `VITE_SENTRY_SMOKE_TEST=true` for a single
  // build, load the console once, and look for this message in Sentry: that
  // proves the DSN, the release tag and the source-map upload all work,
  // without waiting for something to break. Then unset it.
  if (config.smokeTest) {
    Sentry.captureMessage(SMOKE_TEST_MESSAGE, "info");
  }
}

/**
 * Whether a client is installed, i.e. whether an event captured now would
 * actually be sent.
 *
 * The crash screen asks before showing the event id `Sentry.ErrorBoundary`
 * hands it. The SDK mints that id locally whether or not reporting is on, so an
 * install with no DSN would otherwise offer the operator a reference that
 * exists nowhere — and they would spend a support round trip finding that out.
 */
export function isReporting(): boolean {
  return Sentry.getClient() !== undefined;
}
