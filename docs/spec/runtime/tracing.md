# Performance tracing and the request timeline

The companion to [crash-reporting.md](crash-reporting.md). That document covers
**what broke**; this one covers **what was happening when it broke** — the
timeline, and the link between a console action and the host request it caused.

Everything here is **off by default**, and stays off until an operator sets a
sample rate. That is not caution for its own sake: transactions are billed on a
quota separate from errors, and unlike an error a transaction is emitted for
every request whether or not anything went wrong. Choosing a rate on an
operator's behalf would be choosing a recurring bill on their behalf, which is
the same argument the DSN itself is configuration rather than a compiled-in
constant.

## What "timeline" already means with tracing off

Worth stating first, because most of the value is here and costs nothing:

| | Host | Console |
|---|---|---|
| Stack trace on every event | yes (`attach_stacktrace`) | yes |
| Breadcrumbs before the error | `warn!` / `info!`, capped at 100 | `fetch`, `xhr`, history navigations, capped at 100 |
| Release and environment | yes | yes |

`max_breadcrumbs` is written down at the SDK's own default of 100 rather than
inherited, because the length of the timeline is a decision and should not
change silently under an SDK release. Host breadcrumbs follow `RUST_LOG`: at
this crate's default filter (`error`) a report carries the error and **no**
breadcrumbs, and `RUST_LOG=info` collects them. See crash-reporting.md for why
the bridge is not filtered separately.

## Turning tracing on

| Variable | Surface | Meaning |
|---|---|---|
| `OPENCOMPANY_SENTRY_TRACES_SAMPLE_RATE` | host | Fraction of served requests recorded as a transaction, `0`–`1`. Absent or `0` records none. |
| `VITE_SENTRY_TRACES_SAMPLE_RATE` | console (build time) | Fraction of page loads traced, `0`–`1`. Absent or `0` installs no tracing integration at all. |
| `VITE_SENTRY_TRACE_PROPAGATION_TARGETS` | console (build time) | Comma-separated extra origins that may receive `sentry-trace` / `baggage` headers. Same-origin is always allowed. |

A value outside `0`–`1` is **refused, not clamped**, and the process says so —
`Traces::Unreadable` on the host, a single `console.warn` in the console. `100`
is far more likely to mean "100%" than "1.0", and a clamp would record every
request for someone who meant nothing of the kind. An operator who typed `50%`
has to be able to tell their typo from a working default, which is the same
lesson `Silence::Unreadable` records for the enable switch.

The boot line says which it is:

```
crash reporting: reporting to https://o0.ingest.sentry.io/0 as opencompany@0.1.1+abc123 (self-hosted), performance tracing off
crash reporting: reporting to https://o0.ingest.sentry.io/0 as opencompany@0.1.1+abc123 (self-hosted), tracing 5% of requests
```

### What a rate costs

The honest unit is "transactions per month", and it is not proportional to how
often something breaks — it is proportional to traffic.

| Rate | A host serving 10 req/s | What it is for |
|---|---|---|
| `0` (default) | 0 | Errors and breadcrumbs only. |
| `0.01` | ~260k / month | Enough to see p95 latency by route. |
| `0.05` | ~1.3M / month | A usable sample of slow requests. |
| `1.0` | ~26M / month | Almost certainly a mistake outside a short investigation. |

Sentry's free tier is on the order of 10k transactions a month, so `0.05` on a
busy host will exhaust it in hours. **Start at `0.01`, look at the bill, and
raise it.** The console's rate is per page load rather than per request and is
correspondingly cheaper, but the same advice applies.

The console also pays a **bundle** cost, measured on this branch rather than
estimated (`npm run build`, gzipped size of `dist/assets/*.js`):

| Bundle | Gzipped JS | Delta |
|---|---|---|
| Without `browserTracingIntegration` | 829,884 B | — |
| With it (this branch) | 851,254 B | **+20.9 KB** |

That 21 KB is paid by every console bundle, including one built with no DSN,
because the integration is referenced from `initSentry` and cannot be
tree-shaken behind a runtime condition. The integration is still only
*installed* when a rate was asked for — `tracesSampleRate: 0` would sample
everything out anyway, but the integration patches `fetch`, `history` and the
performance observer to do it, and an install that wanted none of this should
not pay that at run time.

## Distributed tracing: console → host

With `browserTracingIntegration` on, the console attaches `sentry-trace` and
`baggage` headers to its API requests; the host's tower layer reads them and
continues the same trace instead of starting an unrelated one. A failed action
in the browser and the server request that served it are then one trace.

Three things make this work, and each was checked rather than assumed:

- **The header targets.** Default is same-origin only (`/^\//`). That covers
  both shapes the console actually runs in — the Vite dev server proxies `/api`
  to the host, and a bundle the host serves is same-origin with it. A
  `sentry-trace` header sent to a third party tells them the app is
  instrumented and hands them an id that correlates their logs with the
  operator's, so a cross-origin host has to be named explicitly.
- **CORS.** No change was needed. `server::cors` answers a preflight by
  **echoing** `Access-Control-Request-Headers`, so `sentry-trace` and `baggage`
  are allowed already. Had it sent a fixed list, adding these headers would have
  broken every cross-origin API call.
- **Transaction naming.** `sentry` is built with `tower-axum-matched-path`, so a
  transaction is named for the matched route
  (`/api/v1/companies/{company}/agents`) rather than the concrete path. Naming
  by raw path would produce one transaction name per company id, which makes the
  transaction list unreadable and spends the quota on cardinality.

The layers are added in `observability::instrument_http`, called once from
`server::routes::router`, and **only when the live client is actually
recording** — so an install with tracing off carries no extra middleware at all
rather than middleware that samples nothing.

## Scrubbing a transaction, and why it is not a `before_send`

This is the part that most needed care, because the SDK does not offer the seam
the rest of the design assumes.

**The console** has one: `beforeSendTransaction` is a *separate* hook from
`beforeSend`, and a transaction never passes through the latter. Without wiring
it, every span would have left unscrubbed while `sanitizeEvent` gave the
appearance of covering everything. `sanitizeTransaction` is that hook.

**The host has none.** `sentry` 0.47 — the version this crate pins, for the
reason in `Cargo.toml` — has `before_send` and `before_send_log` and **no
`before_send_transaction`**. `Transaction::finish` builds an envelope and hands
it straight to `Client::send_envelope`, bypassing every callback. So on the host
the guarantee is made one layer lower, at `ScrubbingTransport`: a wrapper around
the SDK's own transport that scrubs each envelope on its way out.

That is a better place for it than the hook would have been. It is the last
point before bytes leave the process, so it covers **every** envelope item kind
rather than the ones the SDK happens to expose a callback for today — including
any a future SDK version starts emitting.

What a transaction loses:

| Field | Treatment |
|---|---|
| `server_name`, `user` | dropped, as for events |
| `request` | narrowed to method plus the URL **without** userinfo, query string or fragment |
| `name` | scrubbed (normally the matched route, which is safe by construction) |
| span `description`, span `data` | scrubbed |
| `tags`, `extra`, `contexts.Other` | scrubbed |

### The magic-link query parameter

A transaction and a navigation breadcrumb both carry URLs, and the console's
sign-in URL is `?code=<43 characters>` — a working sign-in for anyone who can
read the operator's Sentry project. `App.tsx` clears it with
`history.replaceState`, but that runs *after* the SDK's history instrumentation
is installed, so the breadcrumb records the URL as it was.

Both scrubbers therefore gained a **query-string pass** that redacts the values
of `code`, `state`, `sig` and `signature` — plus everything already in
`SECRET_KEYS` — wherever they appear as URL query parameters. Those four names
are query-only on purpose: `code` is far too common in prose and in
`code=ECONNREFUSED` to redact everywhere, but inside `?…=` there is no
ambiguity.

## Session Replay: deliberately not shipped

Replay is the most literal reading of "timeline", and it is the one thing here
that was evaluated and **declined**. Three reasons, in order of weight:

1. **It records the DOM.** On this surface that is the founder's chat with their
   agents, their ledger figures and their workspace text — precisely the content
   `redaction` and `sanitizeEvent` exist to keep out of a report. Every other
   control in this design is about *not* sending content; a replay sends all of
   it by default.
2. **Masking removes the reason to have it.** `maskAllText` + `blockAllMedia` is
   the only defensible configuration, and it reduces a text-heavy operator
   console to a recording of grey rectangles moving. What survives is roughly
   what breadcrumbs already say, for far more money and risk.
3. **It costs 40 KB gzipped on every bundle** — measured, not estimated:
   851,254 B without it against 892,177 B with it. That is paid by every
   operator, including the overwhelming majority who would never switch it on,
   because a conditionally-referenced integration cannot be tree-shaken.

An operator who wants it anyway can add `Sentry.replayIntegration` to
`frontend/src/lib/sentry.ts` and set `replaysOnErrorSampleRate`. The two sample
rates are pinned to `0` in that file with a comment saying why, so the decision
is visible at the place it would be reversed rather than only here.

Profiling is not shipped either, for a smaller version of reason 3: it needs a
second SDK package and answers a question ("which function is slow") that this
project has not yet had.

## What is tested

| Property | Test |
|---|---|
| A rate outside `0`–`1` is refused, not clamped | `config::test::a_rate_that_is_not_a_fraction_is_refused_rather_than_clamped`, and the console's `refuses a rate that is not a fraction rather than clamping it` |
| An explicit `0` reads the same as unset | `config::test::performance_tracing_is_off_until_a_rate_is_asked_for` |
| The boot line distinguishes off from a typo | `config::test::the_boot_line_says_what_tracing_will_do` |
| No credential survives a transaction | `observability::test::gated::no_credential_survives_a_transaction` |
| The transport scrubs a transaction envelope | `observability::test::gated::the_transport_scrubs_a_transaction_envelope` |
| An envelope with no transaction is passed through untouched | `observability::test::gated::an_envelope_with_no_transaction_is_passed_through_untouched` |
| A magic-link code is redacted from a URL | `redaction::test::a_url_query_loses_the_parameters_that_name_a_credential`, and the console's `redacts a magic-link code from a navigation breadcrumb` |
| A `?` in prose is not a query string | `redaction::test::a_question_mark_in_prose_is_not_a_query_string` |

## Known limitations

- **Host spans are HTTP-level only.** One transaction per served request, with
  no child spans for database calls or agent turns, because nothing in the tree
  opens a `sentry` span. A `tracing` span does *not* become a Sentry span here:
  the bridge is configured for events and breadcrumbs only.
- **The desktop webview cannot trace**, for the same CSP reason it cannot
  report — `connect-src 'self' ipc:`.
- **A trace links the console to the host, not the host to a model provider.**
  Outbound calls the host makes are not instrumented.
