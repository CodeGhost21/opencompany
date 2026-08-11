# The company credential

How a company proves who it is to the surfaces the platform brokers on its
behalf (issue #586). The routes are listed in
[`api.md`](api.md#credential-bearing-surfaces-feature-gated); this is the model
behind them.

## One key per company

A company belongs to one owner. Its admin sets **one** TinyHumans key through
`PUT …/credential`, and everything the platform backend brokers for the company
rides it. Membership in the company is what grants access to it.

Composio is the first consumer, and it is the one that shows why this works: the
backend derives the Composio entity from whatever bearer it is handed, and a
TinyHumans key is a bearer it recognises. So the key **authorizes Composio
directly** — there is no provisioning step that trades it for a second token,
and no per-tenant provider application to register with Google, Slack or GitHub.
A company with a key set connects Gmail by clicking Connect.

## Where a connection lives

On the backend, keyed by the account the bearer resolves to — under this model,
the company's owning account. Nothing about a connection is scoped to the member
who made it, and nothing new is stored on the instance.

That is what makes "connect Gmail once, every teammate's agents can use it" true:
every agent in the company resolves the *same* credential from the *company's*
store, so the backend resolves one entity for all of them. It is a property of
the resolution, not a feature layered on top of it.

## Resolution — one seam

`company::company_key::resolve` is the only place a company identity is
resolved. Most specific first:

1. **The company's own key** — `tinyhumans/key` in its `SecretStore`, set by an
   admin through the console.
2. **This instance's platform identity** — `TinyhumansTokenSource`: a projected,
   audience-bound pod token the cluster rotates in place, else a static
   `TINYHUMANS_API_KEY`. Unchanged behaviour for a tenant whose admin set
   nothing.
3. **Nothing** ⇒ fail closed. No tools are wired, and the read planes report the
   degraded state rather than offering a picker that cannot work. An absent
   credential means "no tools", never a borrowed identity.

A surface may prepend its **own** escape hatch above that seam. Composio keeps
its BYO `composio/token` for a company that insists on using its own Composio
account, so its full order is `composio/token` → company key → instance
identity → none. What no surface may do is resolve a *company* identity some
other way.

## Rotation

The rotation guarantee — "rotating the company key does not silently leave one
brokered surface on the old credential" — is structural rather than a
convention. Because every brokered surface calls the one resolver, there is no
second resolution that could drift, and no surface that could forget to re-read.

Two mechanics make it land without a restart:

- The key is read **live** from the secret store on every resolution, so a set /
  rotate / clear takes effect on the next cycle.
- The resolved credential contributes its **value** to the harness roster
  fingerprint (`Credential::hash_identity`), so a rotation rebuilds the tool
  roster. This is deliberately different from the projected platform token,
  which contributes its *path*: the cluster rotates that one every few minutes
  and hashing its value would rebuild every agent's roster on that schedule. A
  company key is rotated by a person, on purpose, and a new value really is a
  new identity.

## Write-only, and admin-only

The key is sent on the `key` field, stored, and never echoed. No read route
returns it; `GET …/credential` carries `configured` plus `source` — one of
`company` / `attested` / `static` / `none`, the same vocabulary `GET …/composio`
and the connections read plane already use. `Credential`'s `Debug` redacts it,
and the Composio tools feed whatever value they resolve to the scrubber as a
known secret, so it cannot survive into agent-visible output.

`PUT` requires an admin. This key is the identity every one of the company's
agents presents **and** the account they all spend against, so setting it is a
decision made for the company rather than a member's own — the same reasoning
that made `PUT …/composio/token` admin-only in issue #403. Both a set and a
clear are journaled as `ToolAccessChanged`, told apart from each other, and
attributed to whoever made the change.

## Not the inference key

`inference/key` is a different thing and must stay a different slot. It holds
whatever credential the company's *declared provider* wants — an OpenRouter
`sk-or-…`, a raw BYOK token, an `openai_compatible` key. It is provider-scoped,
not an identity, and handing it to the TinyHumans backend would present one
vendor's credential to another. The two coincide only when the declared provider
is `managed`, and even then they are the same value for different reasons.

## What this does not cover

- **The native OAuth catalog.** `…/connections/{provider}/start` still resolves
  through `HostConnectRoutes` — a stored provider token, else a *projected*
  instance identity, else this host's own registered provider application. That
  precedence is unchanged, so `source: "company"` never appears on a native-only
  provider. That path is entangled with the inert-catalog question in #396 and is
  left alone deliberately rather than papered over.
- **Chat inference and embeddings.** Both still resolve from the environment via
  `hosted_endpoint_from_env`. Moving them onto this seam is issue #585; when it
  lands they inherit the rotation guarantee by construction, because the seam is
  already here.
- **Media generation and `web_search`.** Deliberately environment-only: those run
  on the *platform's* managed credential, never a company-controlled one.

## Known limits, recorded deliberately

- **No per-member attribution.** Spend arrives as one account, so which member
  burned what is not answerable. Per-agent `budget_usd_daily` caps still work;
  per-person accounting does not exist and is not in scope.
- **Removing someone from the roster stops future access**, but nothing already
  spent is separable.
- **Two companies pasting the same key share one entity.** That cannot be
  prevented client-side; it is a deployment caveat, the same one the BYO Composio
  token already carries.
