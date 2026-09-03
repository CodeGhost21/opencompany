// The console half of `docs/spec/runtime/crash-reporting.md`: the decision to
// report at all, and what a report is allowed to carry.
//
// These are the two things that have to be right, and both are pure, so they
// are tested here rather than only in a browser. `@/lib/crash-reporting`
// imports no Sentry runtime for exactly this reason — its only `@sentry/react`
// import is a type, which is erased.

import { describe, expect, it } from "vitest";

import {
  REDACTED,
  resolveCrashReporting,
  sanitizeEvent,
  scrubSecrets,
} from "@/lib/crash-reporting";
import type { ErrorEvent } from "@sentry/react";

const DSN = "https://examplePublicKey@o0.ingest.sentry.io/0";
const RELEASE = "opencompany@0.1.0+d31e532f7c8a";

describe("resolveCrashReporting", () => {
  it("reports nothing when no DSN is configured", () => {
    // The state every local checkout and every CI run is in.
    expect(resolveCrashReporting({}, RELEASE)).toBeNull();
    expect(resolveCrashReporting({ VITE_SENTRY_DSN: "" }, RELEASE)).toBeNull();
    expect(resolveCrashReporting({ VITE_SENTRY_DSN: "   " }, RELEASE)).toBeNull();
  });

  it("reports when a DSN is configured", () => {
    expect(resolveCrashReporting({ VITE_SENTRY_DSN: DSN }, RELEASE)).toEqual({
      dsn: DSN,
      environment: "production",
      release: RELEASE,
      smokeTest: false,
    });
  });

  it("refuses a string that is not a Sentry DSN", () => {
    // Each of these nearly parses, and each would resolve to a client that
    // never delivers — which is the failure the check exists to prevent.
    for (const candidate of [
      "o0.ingest.sentry.io/0", // no scheme
      "ftp://key@o0.ingest.sentry.io/0", // a scheme nothing can POST to
      "https://o0.ingest.sentry.io/0", // no public key
      "https://key@o0.ingest.sentry.io/", // no project id
      "https://key:secret@o0.ingest.sentry.io/0", // the pre-2016 secret form
      "not a dsn",
    ]) {
      expect(resolveCrashReporting({ VITE_SENTRY_DSN: candidate }, RELEASE)).toBeNull();
    }
  });

  it("names the environment, defaulting by build kind", () => {
    expect(resolveCrashReporting({ VITE_SENTRY_DSN: DSN, DEV: true }, RELEASE)?.environment).toBe(
      "development",
    );
    expect(
      resolveCrashReporting(
        { VITE_SENTRY_DSN: DSN, VITE_SENTRY_ENVIRONMENT: "  Hosted-Tenant " },
        RELEASE,
      )?.environment,
    ).toBe("hosted-tenant");
  });

  it("arms the smoke test only on an exact `true`", () => {
    // Vite env values are always strings, so a loose check would make
    // `VITE_SENTRY_SMOKE_TEST=false` fire the event.
    expect(
      resolveCrashReporting({ VITE_SENTRY_DSN: DSN, VITE_SENTRY_SMOKE_TEST: "true" }, RELEASE)
        ?.smokeTest,
    ).toBe(true);
    for (const value of ["false", "1", "TRUE", "yes", ""]) {
      expect(
        resolveCrashReporting({ VITE_SENTRY_DSN: DSN, VITE_SENTRY_SMOKE_TEST: value }, RELEASE)
          ?.smokeTest,
      ).toBe(false);
    }
  });
});

describe("scrubSecrets", () => {
  it("removes a labelled credential and keeps the diagnostic", () => {
    for (const [input, expected] of [
      ["api_key=hunter2", `api_key=${REDACTED}`],
      ["api-key: hunter2", `api-key: ${REDACTED}`],
      [`"token":"hunter2"`, `"token":"${REDACTED}"`],
      ["password = hunter2", `password = ${REDACTED}`],
      ["--token hunter2", `--token ${REDACTED}`],
      ["Authorization: Bearer hunter2", `Authorization: Bearer ${REDACTED}`],
    ]) {
      expect(scrubSecrets(input)).toBe(expected);
    }
  });

  it("never mistakes the auth scheme for the credential", () => {
    // Redacting `Bearer` and leaving the credential is worse than doing
    // nothing: the message then looks scrubbed.
    const scrubbed = scrubSecrets("Authorization: Bearer hunter2");
    expect(scrubbed).toContain("Bearer");
    expect(scrubbed).not.toContain("hunter2");
  });

  it("removes a credential that identifies itself", () => {
    for (const secret of [
      "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      "xoxb-1111111111-2222222222-AAAAAAAAAAAA",
      "th_live_AAAAAAAAAAAAAAAAAAAA",
      "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln",
    ]) {
      const scrubbed = scrubSecrets(`the host said: ${secret} was rejected`);
      expect(scrubbed).not.toContain(secret);
      expect(scrubbed).toContain("was rejected");
    }
  });

  it("strips userinfo from a URL", () => {
    expect(scrubSecrets("GET https://user:hunter2@host.example/api/v1 failed")).toBe(
      `GET https://${REDACTED}@host.example/api/v1 failed`,
    );
    // A port colon is not an assignment.
    expect(scrubSecrets("GET https://host.example:8080/api/v1")).toBe(
      "GET https://host.example:8080/api/v1",
    );
  });

  it("leaves prose and ordinary diagnostics alone", () => {
    // The word-boundary false positive that plagued the regex version cannot
    // arise: `cancellation_token` is one token and normalizes to something
    // that is not a key.
    for (const input of [
      "the token was rejected by the provider",
      "cancellation_token=abc123",
      "next_page_token=abc123",
      "idempotency_key=abc123",
      "GET /api/v1/companies/acme/agents -> 500 in 42ms",
      "no credential is configured for this company",
    ]) {
      expect(scrubSecrets(input)).toBe(input);
    }
  });
});

describe("sanitizeEvent", () => {
  /** One event carrying something it must not send in every field that can. */
  function hostileEvent(): ErrorEvent {
    return {
      type: undefined,
      message: "refresh failed: api_key=hunter2",
      server_name: "operator-laptop",
      user: { email: "operator@example.com", ip_address: "203.0.113.4" },
      request: {
        url: "https://console.example/#/settings?code=magic-link-code",
        headers: { "User-Agent": "Mozilla/5.0", Cookie: "oc_session=hunter2" },
      },
      extra: { state: "password=hunter2" },
      contexts: {
        os: { name: "macOS" },
        browser: { name: "Chrome" },
        device: { family: "Mac" },
        // Anything could be in here, so nothing is kept.
        redux: { store: "hunter2" },
      },
      tags: { origin: "th_live_AAAAAAAAAAAAAAAAAAAA" },
      breadcrumbs: [
        {
          message: "GET https://u:hunter2@host.example/api/v1",
          data: { url: "https://host.example/api/v1?token=hunter2", status_code: 500 },
        },
      ],
      exception: {
        values: [
          {
            type: "ApiError",
            value: "token: ghp_AAAAAAAAAAAAAAAAAAAAAAAA rejected",
            mechanism: { type: "generic", data: { secret: "hunter2" } },
            stacktrace: {
              frames: [
                {
                  filename: "app.js",
                  context_line: 'const key = "hunter2";',
                  pre_context: ["function connect() {"],
                  post_context: ["}"],
                  vars: { key: "hunter2" },
                },
              ],
            },
          },
        ],
      },
    };
  }

  it("lets no credential or identity through", () => {
    const rendered = JSON.stringify(sanitizeEvent(hostileEvent()));
    for (const leaked of [
      "hunter2",
      "ghp_AAAA",
      "th_live_AAAA",
      "operator-laptop",
      "operator@example.com",
      "203.0.113.4",
      "magic-link-code",
    ]) {
      expect(rendered).not.toContain(leaked);
    }
  });

  it("keeps what makes a report worth reading", () => {
    const sanitized = sanitizeEvent(hostileEvent());
    // The diagnostic around each redaction.
    expect(sanitized.message).toContain("refresh failed");
    expect(sanitized.exception?.values?.[0]?.value).toContain("rejected");
    // Which request, and what it answered.
    expect(sanitized.breadcrumbs?.[0]?.data?.status_code).toBe(500);
    expect(sanitized.breadcrumbs?.[0]?.data?.url).toContain("host.example");
    // The one header Sentry needs to derive OS / browser / device server-side.
    expect(sanitized.request).toEqual({ headers: { "User-Agent": "Mozilla/5.0" } });
    expect(sanitized.contexts?.browser?.name).toBe("Chrome");
    // And the surface, so console events filter cleanly beside the host's.
    expect(sanitized.tags?.surface).toBe("console");
  });

  it("allow-lists contexts rather than scrubbing them", () => {
    // An unknown context has unknown shape, so the list fails in the safe
    // direction.
    const sanitized = sanitizeEvent(hostileEvent());
    expect(Object.keys(sanitized.contexts ?? {}).sort()).toEqual(["browser", "device", "os"]);
  });

  it("strips frame locals and source context", () => {
    const frame = sanitizeEvent(hostileEvent()).exception?.values?.[0]?.stacktrace?.frames?.[0];
    expect(frame?.vars).toBeUndefined();
    expect(frame?.context_line).toBeUndefined();
    expect(frame?.pre_context).toBeUndefined();
    expect(frame?.post_context).toBeUndefined();
    // The frame itself survives, or there is no stack trace to read.
    expect(frame?.filename).toBe("app.js");
  });

  it("never drops an event", () => {
    // A scrubber, not a filter: deciding which errors are worth seeing belongs
    // in the operator's own project, where they can see what they suppressed.
    expect(sanitizeEvent({ type: undefined })).not.toBeNull();
  });
});
