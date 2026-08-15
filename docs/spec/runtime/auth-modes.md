# Sign-in modes

How humans prove who they are is **configuration**, not a code path. A company
picks one of three modes, and the one it picks is the only one its host serves.

| Mode | Who signs in | How | Bootstrap list |
|---|---|---|---|
| `email` (default) | an invited address | magic link, optional password, ecosystem hub | `[users].admins` |
| `wallet` | an invited base58 wallet | a signed challenge | `[users].wallets` |
| `none` | nobody | there is no sign-in | — |

`email` is the default and is exactly what every company did before this was
configurable, so a manifest that names no mode is unaffected by this existing.

Everything downstream of *identification* is shared. All three modes converge on
the same `eligibility` → `upsert_from_eligibility` → `mint_session` path, the
same `SessionRecord`, the same cookie, the same roster, the same revocation. Only
the proof differs. That is deliberate: a second way in that also had a second
session model would be two auth systems, and the weaker one would decide the
security of the whole.

## One mode, one door

A mode's routes are the only ones that answer. A wallet company refuses
`auth/request`, `auth/verify`, `auth/login` and `auth/password`; an email company
refuses `auth/wallet/*`; a `none` company refuses all of them.

The refusal is `409 {"code": "auth_mode", "mode": "<mode>"}` — **not** a 404.
The route exists and the request was well-formed; what is wrong is the state of
the company it was aimed at. A 404 is indistinguishable from a version skew or a
typo and sends a client looking for a spelling mistake. For the same reason every
route stays mounted in every mode rather than being left off the router.

Naming the mode in the refusal does not breach the
[generic-failure rule](users.md#every-login-failure-is-identical). That rule
protects *who is on the roster*; the mode is a property of the deployment, and
`GET …/auth/config` publishes it to anonymous callers already — the console
cannot draw a sign-in screen without it.

## Where the mode comes from

The same four layers as every other setting ([config.md](config.md)), earlier
winning:

1. `OPENCOMPANY_AUTH_MODE`
2. `auth_mode` in `config.toml`
3. the company's `[users].mode`
4. `email`

Layers 1–2 are host-wide and reach every company as
`AppConfig::auth_mode_override`; layer 3 is per company. Both meet in
`RuntimeBuilder::with_auth_mode_override`, which resolves the mode **once, at
build**, and caches it on the `CompanyRuntime`. It is read on the request path —
by the login routes, the user-administration routes, and principal resolution —
and re-reading a manifest per request would be a `CompanyStore` read for a value
that cannot change without a rebuild.

The host layers exist because two deployments own the answer rather than the
company definition does: the packaged desktop app, which is always `none`, and a
hosting platform, which must be able to guarantee a mode across every tenant
whatever a tenant wrote.

An unparseable value **aborts boot**. "The sign-in you configured is not the one
you got" is invisible from a running host.

## One identity column

`UserRecord::email` is the login **identity key**, unique per company, and every
storage backend already indexes it. What fills it follows the mode:

| Mode | Key | Example |
|---|---|---|
| `email` | the normalized address | `ada@example.com` |
| `wallet` | `wallet:` + base58 public key | `wallet:7xKX…` |
| `none` | `local:owner` | `local:owner` |

Built and parsed through `LoginIdentity`, never by hand. Two reasons this is one
column rather than three:

- **The invite keyspace shares it.** "Invited" and "joined" are two states of one
  identity, and suspension, session revocation and removal all key off the
  `UserRecord::id` it resolves to. A parallel column would be a second thing each
  of those paths has to remember.
- **The prefixes make it safe.** An address normalized by `normalize_email` is
  lowercase and cannot begin with a scheme the parser recognizes, and base58
  excludes `:`. No wallet can pose as a mailbox or the reverse. An unprefixed key
  is an email, which is what every record written before this loads as — that is
  the whole migration.

Normalization differs by scheme and the difference is load-bearing:
`normalize_email` folds case, and folding the case of a base58 address would map
two distinct wallets onto one identity, so a signature verified against one would
mint a session for the other. `normalize_wallet` trims and nothing else.

Mail paths must ask `LoginIdentity::mailbox()`, which is `None` for everything
that is not an email. A `wallet:7xKX…` handed to an SMTP transport is a bug that
surfaces only in a bounce log, so the absence of a mailbox is a type rather than
a convention.

## `wallet`

Ed25519 over base58 — the Solana-style address space the company's own
tiny.place identity already uses, verified by the same `economy::signer` code, so
there is one answer to "does this signature verify" rather than two.

```text
POST …/auth/wallet/challenge  {address}            → {nonce, message, expiresAtMillis}
POST …/auth/wallet/verify     {nonce, signature}   → session cookie + the user
```

The signed bytes, versioned by their first line:

```text
opencompany-wallet-login-v1\n
<company id>\n
<base58 address>\n
<nonce>\n
<issued at, epoch millis>
```

Every field is bound for a reason, and the reasons are the attacks: the **domain
tag** because a wallet signs for many protocols with one key and a signature
gathered elsewhere would otherwise replay as a login; the **company id** because
otherwise a signature for company A logs its holder into company B on a host
serving both; the **address** to bind the proof to the key it claims; the
**nonce** to make each signature good exactly once; the **issue time** to make a
captured message legible as stale. Clients must sign the `message` verbatim
rather than rebuilding the layout, or every future change to it is a breaking
change for every client.

Three properties are worth stating outright:

- **Verification reads the record, never the request.** The address and the issue
  time come from the stored challenge; the caller supplies only the nonce and the
  signature. Rebuilding the message from anything the caller sent would let them
  choose which wallet they are proving control of.
- **Redemption happens before the signature check.** `LoginCodeStore::consume` is
  atomic, so a nonce cannot be spent twice by two racing requests — and a caller
  who could burn attempts against a live nonce without consuming it would have
  unlimited tries at forging one signature.
- **A challenge is not evidence of anything.** Every well-formed address gets
  one, including addresses this company has never heard of; an ineligible one is
  simply never persisted, so answering it fails with the same `invalid_login` a
  forged signature gets. That keeps the route from being a membership oracle
  *and* stops an anonymous caller filling the code table with invented addresses,
  which they otherwise could — a wallet address is public and costs nothing to
  make up.

Challenges live 5 minutes, against a magic link's 15. The difference is not
caution: a link waits in a mailbox for a human to notice it, while a challenge is
answered by software the moment it arrives.

Challenges share `LoginCodeStore` with magic links and device-pairing codes, kept
apart by hashing under a distinct domain prefix rather than by a discriminant
field — three keyspaces cannot collide, whereas one keyspace plus a check is only
as good as every caller remembering the check.

There is **no password path and no mailbox** in a wallet company: no magic link,
no invite mail, no ecosystem buttons (a hub sign-in resolves to an email address
and would apply an email roster this company does not have). An admin still
invites a base58 address exactly as they would invite a mailbox; the invitee
learns of it out of band, and the invite reports its delivery as `no_mailbox` —
not a failure and not a missing transport, but the honest statement that there
was never a message to send.

There is deliberately no environment counterpart to `OPENCOMPANY_ADMIN_EMAIL`.
That variable exists because a platform-provisioned company's creator is known
only to the control plane, and the control plane records an email address, not a
wallet.

## `none`

For the packaged desktop app: a loopback host, one person, no accounts.

It does not merely hide the login screen. There is no sign-in and **no way to add
a second person** — every route that would invite, re-role, suspend or
re-credential somebody is refused, because a company nobody signs in to has no
way to tell one human from another and an invite would grant an account that can
never be reached. Listing stays available: showing the one local owner is honest,
and an empty screen would not be.

`resolve_principal` answers with that owner **before** the session and bearer
paths, and that order is the point — in `none` mode there is no session to find
and no bearer to verify, so falling through would leave every request
unauthenticated against a host whose whole premise is that its only caller is its
owner.

The owner is a real `UserRecord`, materialized on first use under
`LoginIdentity::Local` and minted as an `Admin`. Not a principal invented per
request: chat attribution, task assignment and the audit trail all key off
`UserRecord::id`, and a synthetic id no store row backs would come apart at the
first of them that looked the user up. It is the one place a user is created
without anyone proving anything, and it can be, because the mode's premise is
that whoever reaches this host is its owner.

**Boot refuses `none` on a routable bind.** A company with no sign-in on a host
anyone can reach is an unauthenticated admin console, not a desktop app, and the
contradiction is otherwise silent — the host does start and does serve. The check
is `AppConfig::is_local_only()`, the same predicate that gates echoing a login
code in a response, and a failure aborts boot exactly as a
selected-but-unavailable storage backend does.

## What the console is told

`GET …/auth/config` → `{"mode": "email"|"wallet"|"none", "passwords": bool}`.

Unauthenticated by construction, like every other login route: the console asks
before anyone has a credential, because it cannot choose a screen otherwise. It
must branch on this rather than on which routes fail — a wallet company and a
misconfigured email company both refuse `auth/request`, and only one of them
should be offered a wallet button. A console that cannot reach this route assumes
`email`, which is what every host predating it does.

## Related

- [Human users](users.md) — the roster, sessions, invites, and the rules every
  mode shares.
- [Configuration](config.md) — the precedence layers and the environment.
- [The company manifest](manifest.md) — `[users]` and the rest of the schema.
