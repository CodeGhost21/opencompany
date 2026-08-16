# The hub console

One deployment of the operator console, at its own origin, operating many hosts
that live on other origins — typically one subdomain per tenant.

Read [Users and sessions](users.md) first: this document assumes the two session
carriers described there, and the hub exists because only one of them works
across origins.

The console can be deployed **once**, at its own origin, and operate many hosts
living on other origins — typically one subdomain per tenant. The same bundle
serves both shapes; a hub build sets `VITE_OC_HUB=1` (or is opened with `?hub`).

What changes, and what does not:

| | Same-origin console | Hub console |
|---|---|---|
| Bootstrap connection | its own origin | none — its origin serves assets only |
| Which hosts it knows | the one that served it | the ones someone added, in `localStorage` |
| Session carrier | `HttpOnly` cookie | `x-opencompany-session`, held by the page |
| Event stream | `EventSource` | `fetch` — `EventSource` cannot set a header |

The console holds N hosts either way; that has been true since connections
landed. A hub differs only in having no host at its own origin to seed the list
with.

**Each tenant must allow-list the hub's origin** in
`OPENCOMPANY_CORS_ORIGINS`, or the browser blocks every response. There is no
wildcard: the session is a credential, and `Access-Control-Allow-Origin: *` is
forbidden with credentials.

## The cost, stated plainly

A hub console holds its session tokens in `localStorage`, so script execution on
the hub's origin reaches every host its operator has signed in to — where a
same-origin console's `HttpOnly` cookie would have survived it. This is a real
reduction and is accepted only because the alternative is not a safer console
but no console: the cookie is never sent cross-site, and `SameSite=None` would
merely turn it into a third-party cookie Safari discards.

Two things bound it. The credential is a *session* — revocable from the host's
device list, expiring on its own — and it is only ever chosen where a cookie
could not have worked, a decision derived from the address rather than
configured (`needsCarriedSession`), so it cannot be turned on by mistake for a
deployment that had a cookie available.

## Magic links land on the host, not the hub

A login link is built by the host out of its own base URL, so following one
opens that host's own console rather than the hub. Within the hub, the working
sign-in paths are password, wallet, and ecosystem sign-in. Redirecting a link
back to a hub origin would require the host to know that origin, which nothing
tells it today.
