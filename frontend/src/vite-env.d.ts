/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OC_API?: string;
  readonly VITE_OC_COMPANY?: string;
  readonly VITE_OC_TOKEN?: string;
  /**
   * Crash reporting (`docs/spec/runtime/crash-reporting.md`). Unset — the
   * default everywhere, including every local checkout and every CI run —
   * means the SDK is never initialized and nothing is sent.
   */
  readonly VITE_SENTRY_DSN?: string;
  /** Overrides the `environment` tag. Defaults to `development`/`production`. */
  readonly VITE_SENTRY_ENVIRONMENT?: string;
  /** Exactly `true` fires one smoke event at init, to verify the pipeline. */
  readonly VITE_SENTRY_SMOKE_TEST?: string;
  /**
   * The traced fraction of page loads, `0` to `1`. Absent (= `0`) installs no
   * tracing integration at all. Transactions are billed separately from errors
   * and are emitted whether or not anything failed.
   */
  readonly VITE_SENTRY_TRACES_SAMPLE_RATE?: string;
  /**
   * Comma-separated origins that may receive `sentry-trace` / `baggage`
   * headers, so a console action links to the host request it caused.
   * Same-origin is always allowed.
   */
  readonly VITE_SENTRY_TRACE_PROPAGATION_TARGETS?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/**
 * The Sentry release tag, substituted at build time by `vite.config.ts`.
 *
 * A `define` rather than a `VITE_*` variable because the same string has two
 * consumers that must not disagree — `Sentry.init` inside the bundle and
 * `@sentry/vite-plugin`'s source-map upload — and computing it once in the
 * config is the only arrangement where they cannot drift. A release the bundle
 * and the upload disagree about is a stack trace Sentry never symbolicates,
 * with no error anywhere to say why.
 */
declare const __SENTRY_RELEASE__: string;
