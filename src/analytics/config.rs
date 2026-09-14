//! The enable/disable decision: whether this process reports at all, and where.
//!
//! Kept apart from both the transport and the payload because it is the part
//! that has to be *provably* right. It is pure — an [`EnvSource`] and a
//! [`Deployment`] in, a [`Decision`] out — so every branch of it is tested in
//! the default build, with no network and no feature flag.

use crate::analytics::types::TenantIdKey;
use crate::app::config::EnvSource;
use crate::app::deployment::Deployment;

/// Operator override: `on` forces reporting, `off` forbids it.
pub const ENABLE_ENV: &str = "OPENCOMPANY_ANALYTICS";
/// The OpenPanel client id. Half of the pair; useless on its own.
pub const CLIENT_ID_ENV: &str = "OPENCOMPANY_ANALYTICS_CLIENT_ID";
/// The OpenPanel client secret. **Configuration, never a compiled-in constant**
/// — a secret baked into a public binary is a secret everyone has, and this one
/// grants write access to the operator's collector.
pub const CLIENT_SECRET_ENV: &str = "OPENCOMPANY_ANALYTICS_CLIENT_SECRET";
/// The collector URL. **Required, with no default**, because a self-hosted
/// collector has no canonical address and guessing one means reporting to
/// somebody else's — see [`resolve`].
pub const ENDPOINT_ENV: &str = "OPENCOMPANY_ANALYTICS_ENDPOINT";
/// The secret that makes a hosted tenant's analytics id unguessable.
///
/// **Configuration, never a compiled-in constant**, and for a sharper reason
/// than the project token: a salt baked into a GPL-3.0 binary is a salt every
/// reader of the source already has, which is no salt at all. Injected by the
/// platform that provisions tenants; never given to the collector. Absent means
/// the host identifies itself by its random instance id instead — see
/// [`TenantIdKey`](crate::analytics::types::TenantIdKey).
pub const ID_KEY_ENV: &str = "OPENCOMPANY_ANALYTICS_ID_KEY";

/// An OpenPanel write client: an id and a secret, which authenticate together.
///
/// OpenPanel takes both as request **headers** — `openpanel-client-id` and
/// `openpanel-client-secret` — rather than as a field in the body, which is the
/// one structural difference from the token this replaced. It is a difference
/// worth having: a credential in a header never rides through the payload
/// builder, so no test fixture, recorded event or captured body can carry it.
///
/// A newtype rather than two bare `String`s for one reason: neither half must be
/// printed, logged, or serialized by accident. It derives **neither** `Debug`
/// nor `Serialize` — the hand-written `Debug` redacts both halves — because
/// `serde_json::to_value(&some_config)` is precisely how a credential reaches a
/// payload (issue #1741, `SecretValue`). Nothing in this module ever serializes
/// a config struct; the two values are read out explicitly, once, at the moment
/// the request headers are set.
///
/// **The id is redacted too**, although OpenPanel's own web SDK ships client ids
/// to browsers and treats them as public. The reason is local rather than
/// cryptographic: an id names the operator's project on the operator's
/// collector, this repository is GPL-3.0 and its container logs are routinely
/// pasted into public issues, and there is no line in the tree that is better
/// for having it. Redacting the half that does not need it costs nothing;
/// leaking the half that does costs everything, and a type with one printable
/// field and one redacted one is a type someone eventually prints.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientCredentials {
    id: String,
    secret: String,
}

impl ClientCredentials {
    /// Wraps a client id and secret read from configuration.
    pub fn new(id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            secret: secret.into(),
        }
    }

    /// The client id, for the one caller that puts it in a header.
    pub fn expose_id(&self) -> &str {
        &self.id
    }

    /// The client secret, for the one caller that puts it in a header.
    pub fn expose_secret(&self) -> &str {
        &self.secret
    }
}

impl std::fmt::Debug for ClientCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientCredentials(<redacted>)")
    }
}

/// Why a process is not reporting. Logged once at boot, so an operator who
/// *expected* analytics can tell "switched off" from "misconfigured".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    /// The operator set `OPENCOMPANY_ANALYTICS=off`.
    OptedOut,
    /// Not a hosted tenant, and nobody opted in. **The default.**
    NotHosted,
    /// Reporting was asked for, but neither half of the collector credential is
    /// configured.
    ///
    /// Three reasons rather than one, because the three call for different
    /// edits. OpenPanel authenticates a write client with an id **and** a
    /// secret, so "no credential" and "half a credential" are different
    /// mistakes: the second is what a half-finished secret rollout looks like,
    /// and an operator staring at "no credential is configured" while
    /// `OPENCOMPANY_ANALYTICS_CLIENT_ID` is plainly set in their env file has
    /// been told something that reads as false.
    NoCredentials,
    /// A client secret is configured, but no client id.
    NoClientId,
    /// A client id is configured, but no client secret.
    NoClientSecret,
    /// A credential is configured that could not be put in an HTTP header.
    ///
    /// New with OpenPanel and worth its own reason. Mixpanel's token rode in
    /// the request **body**, where any string at all is legal JSON, so a
    /// mangled token was refused by the collector and that was the end of it.
    /// These two ride in headers, and `reqwest` will not build a request whose
    /// header value contains a control byte — so a secret that picked up a
    /// stray newline in the middle (a `kubectl create secret` on a wrapped
    /// file, most often) would otherwise install a tracker that fails to
    /// construct one single request, forever, behind a `debug!` nobody reads.
    ///
    /// The reason never quotes the value, for the same reason
    /// [`Self::UnusableEndpoint`] does not.
    UnusableCredential,
    /// Reporting was asked for, but no collector endpoint is configured.
    ///
    /// **There is deliberately no default to fall back to.** OpenPanel is
    /// self-hosted, so its address is whatever the operator runs it at, and
    /// there is no address this crate could pick that is not somebody else's
    /// collector. Defaulting would send a tenant's telemetry to a third party
    /// nobody configured — the same failure [`Self::UnusableEndpoint`] exists to
    /// prevent, arriving from the other direction. So an absent endpoint is
    /// silence with its own reason, and the reason names the variable to set.
    NoEndpoint,
    /// `OPENCOMPANY_ANALYTICS` was set to something this does not recognise.
    ///
    /// A separate reason from [`Self::OptedOut`] on purpose: an operator who
    /// typed `of` gets the outcome they meant *and* a boot line saying their
    /// value was not understood, rather than silence they cannot distinguish
    /// from a working opt-out.
    Unreadable,
    /// `OPENCOMPANY_ANALYTICS_ENDPOINT` is set to something no client could
    /// POST to — no scheme, a scheme that is not `http`/`https`, no host, or
    /// bytes this process cannot read.
    ///
    /// Silence rather than reporting, because the alternative is the failure
    /// this whole module is built to prevent: boot prints "reporting to …",
    /// the tracker is installed, and every batch dies in `reqwest` behind a
    /// `debug!` nobody has enabled. An operator reading their own logs would
    /// have no reason to look again. Naming it as a *reason* is the only thing
    /// that turns a silent misconfiguration into one line they can act on.
    ///
    /// The reason is a constant and never quotes the value: an authenticated
    /// proxy's URL is exactly where a credential lives — see
    /// `crate::analytics::boot`.
    UnusableEndpoint,
    /// `OPENCOMPANY_ANALYTICS_ENDPOINT` is a plain `http://` URL to a host that
    /// is not loopback, so the client credential would cross a network in the
    /// clear.
    ///
    /// New with OpenPanel, and it exists because of *where* the credential
    /// travels now. Mixpanel's token rode in the request body to one fixed,
    /// TLS-only address that this crate chose; there was no configuration that
    /// could downgrade it. OpenPanel's address is whatever the operator types,
    /// and its credential rides in a request **header** on every single
    /// request — so `OPENCOMPANY_ANALYTICS_ENDPOINT=http://collector.internal/track`
    /// puts a long-lived write secret on the wire, in cleartext, once per event,
    /// for the life of the tenant (CWE-319). "Internal network" is not a defence
    /// a container can verify, and this module does not get to assume one.
    ///
    /// **Loopback is the documented exception.** `http://127.0.0.1:3000/track`,
    /// `http://[::1]:3000/track` and `http://localhost:3000/track` never leave
    /// the host, so there is no wire to read; that is the shape a developer
    /// running the collector beside the workload actually uses, and every gated
    /// test in this crate. Refusing it would refuse the only http case that is
    /// genuinely safe.
    ///
    /// Silence rather than a warning-and-send, for the reason
    /// [`Self::UnusableEndpoint`] gives one level down: the alternative is a
    /// boot line nobody reads while the secret ships anyway, and a credential
    /// disclosed is not a thing an operator can un-disclose after noticing. The
    /// fix is one character in one variable, and the reason names it.
    ///
    /// The reason never quotes the value, like every other reason here.
    InsecureEndpoint,
}

impl Silence {
    /// The stable reason slug, for the boot log line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OptedOut => "operator opted out",
            Self::NotHosted => "not a hosted tenant and no explicit opt-in",
            Self::NoCredentials => {
                "no collector credential is configured (OPENCOMPANY_ANALYTICS_CLIENT_ID \
                 and OPENCOMPANY_ANALYTICS_CLIENT_SECRET)"
            }
            Self::NoClientId => "OPENCOMPANY_ANALYTICS_CLIENT_ID is not configured",
            Self::NoClientSecret => "OPENCOMPANY_ANALYTICS_CLIENT_SECRET is not configured",
            Self::UnusableCredential => {
                "the configured collector credential contains bytes that cannot go in an \
                 HTTP header"
            }
            Self::NoEndpoint => "OPENCOMPANY_ANALYTICS_ENDPOINT is not configured",
            Self::Unreadable => "the OPENCOMPANY_ANALYTICS value is not recognised",
            Self::UnusableEndpoint => {
                "the OPENCOMPANY_ANALYTICS_ENDPOINT value is not a usable http(s) URL"
            }
            Self::InsecureEndpoint => {
                "OPENCOMPANY_ANALYTICS_ENDPOINT is a plain http:// URL to a non-loopback \
                 host, which would send the collector credential in the clear on every \
                 request; use https, or a loopback address"
            }
        }
    }
}

/// What this process will do.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Send nothing. No client is constructed, so nothing *can* be sent.
    Silent(Silence),
    /// Report to `endpoint` as `credentials`.
    Report {
        /// The collector URL.
        endpoint: String,
        /// The write client this process authenticates as.
        credentials: ClientCredentials,
    },
}

impl Decision {
    /// Whether this decision reports.
    pub fn reports(&self) -> bool {
        matches!(self, Self::Report { .. })
    }
}

/// Resolves the decision.
///
/// The order matters and is the whole policy:
///
/// 1. `OPENCOMPANY_ANALYTICS=off` wins over everything. An operator switching
///    it off must not be overruled by a deployment kind, a token, or a future
///    default.
/// 2. A value that is set but unrecognised resolves to **silence**, whatever
///    the deployment. The deployment default is reserved for a switch that is
///    *absent*. Falling an unreadable value through to the default meant a
///    hosted tenant whose operator typed `OPENCOMPANY_ANALYTICS=of` kept
///    reporting — a typo in the opt-out direction silently ignored, which is
///    the one direction that must never be silently ignored.
/// 3. Otherwise reporting is on **only** for [`Deployment::HostedTenant`], or
///    when an operator explicitly sets `OPENCOMPANY_ANALYTICS=on`. Decision 1
///    of #1739: silence is the default and reporting is the exception, so a
///    self-hosted or desktop install that has said nothing sends nothing.
/// 4. **Both halves of the client credential are required.** OpenPanel
///    authenticates a write client with an id and a secret together, so one
///    without the other is a misconfiguration rather than a partial
///    configuration, and the reason says which half is missing — see
///    [`Silence::NoClientId`].
/// 5. **An endpoint is required, and there is no default.** The collector is
///    self-hosted; its address is whatever the operator runs it at. A default
///    would be somebody else's collector, and quietly reporting to a third
///    party nobody configured is the accident the endpoint check below already
///    refuses to make in the other direction.
/// 6. And the endpoint has to be one a client could post to. A decision that
///    says [`Decision::Report`] is a promise the boot line then repeats out
///    loud, so an endpoint that cannot be sent to is silence with a reason,
///    not reporting — see [`is_usable_endpoint`].
/// 7. **And one the credential can safely cross.** The client secret is a
///    request header on every request, so a plain `http://` endpoint to a
///    non-loopback host puts it on the wire in cleartext once per event. That
///    is silence with its own reason too — see [`is_secure_endpoint`] and
///    [`Silence::InsecureEndpoint`].
pub fn resolve(deployment: Deployment, env: &dyn EnvSource) -> Decision {
    // Read through `get_os`, not `get`. [`EnvSource::get`] maps a non-Unicode
    // value to `None`, which here would read as "the operator said nothing" and
    // leave a hosted tenant reporting — the same failure as the unreadable
    // spelling below, arriving by a different route. The trait's own docs point
    // a reader that must tell *malformed* from *unset* at `get_os` for exactly
    // this reason.
    //
    // Blank is still absent, for the same reason a blank token is: a variable
    // set to whitespace is a variable nobody meant to set. See [`non_blank`].
    let switch = match env.get_os(ENABLE_ENV) {
        Some(raw) => match raw.into_string() {
            Ok(value) => {
                let value = value.trim().to_ascii_lowercase();
                if value.is_empty() { None } else { Some(value) }
            }
            Err(_) => return Decision::Silent(Silence::Unreadable),
        },
        None => None,
    };

    match switch.as_deref() {
        Some("off" | "false" | "0" | "no") => return Decision::Silent(Silence::OptedOut),
        Some("on" | "true" | "1" | "yes") => {}
        // Set, but not a spelling of yes or no. Both directions of that typo
        // are now silence: it was never an opt-in, and — since it reached a
        // hosted tenant's deployment default and kept reporting — it must not
        // be a failed opt-*out* either. Silence is the safe answer to "I cannot
        // tell what you asked for", and the boot line says which value it could
        // not read.
        Some(_) => return Decision::Silent(Silence::Unreadable),
        None => {
            if deployment != Deployment::HostedTenant {
                return Decision::Silent(Silence::NotHosted);
            }
        }
    }

    // A non-Unicode half already fails closed on its own: `get` maps it to
    // `None` and the match below reports it missing. Reporting *less* than was
    // configured is always the safe direction for a credential.
    let credentials = match (
        non_blank(env, CLIENT_ID_ENV),
        non_blank(env, CLIENT_SECRET_ENV),
    ) {
        (Some(id), Some(secret)) => {
            if !is_header_safe(&id) || !is_header_safe(&secret) {
                return Decision::Silent(Silence::UnusableCredential);
            }
            ClientCredentials::new(id, secret)
        }
        (None, None) => return Decision::Silent(Silence::NoCredentials),
        (None, Some(_)) => return Decision::Silent(Silence::NoClientId),
        (Some(_), None) => return Decision::Silent(Silence::NoClientSecret),
    };

    // Read through `get_os`, like the switch, so that bytes this process cannot
    // decode are *unusable* rather than *absent*. The two now resolve to
    // different reasons, and an operator who mistyped their proxy URL should be
    // told the value was unreadable rather than that they never set one.
    //
    // There is no fallback in either arm. Reporting to a default collector an
    // operator never named is worse than reporting nothing at all: it is
    // telemetry leaving for an address nobody chose, and no amount of reading
    // the boot line would reveal it, because the line would name a destination
    // that is real.
    let endpoint = match env.get_os(ENDPOINT_ENV) {
        None => return Decision::Silent(Silence::NoEndpoint),
        Some(raw) => match raw.into_string() {
            Err(_) => return Decision::Silent(Silence::UnusableEndpoint),
            Ok(value) => match value.trim() {
                // Blank is absent, as it is for the credential and the switch.
                "" => return Decision::Silent(Silence::NoEndpoint),
                // Shape before transport security, and the order matters for
                // the reason an operator is given: a value that does not parse
                // has no host to judge, and "this will not parse" sends them
                // somewhere different from "this would leak the secret".
                configured if !is_usable_endpoint(configured) => {
                    return Decision::Silent(Silence::UnusableEndpoint);
                }
                configured if !is_secure_endpoint(configured) => {
                    return Decision::Silent(Silence::InsecureEndpoint);
                }
                configured => configured.to_string(),
            },
        },
    };

    Decision::Report {
        endpoint,
        credentials,
    }
}

/// Whether `raw` is something a client could actually POST a batch to: an
/// absolute `http`/`https` URL with a host.
///
/// This is the check that stops [`resolve`] promising what the transport cannot
/// deliver. `OPENCOMPANY_ANALYTICS_ENDPOINT=collector.internal/track` — a
/// hostname written without a scheme, which is how anyone would first write it
/// — resolved to [`Decision::Report`]: boot said "reporting to
/// collector.internal/track", the tracker was installed, and every send failed
/// with `RelativeUrlWithoutBase` behind a `debug!` line. Nothing an operator
/// would ever see said the endpoint was the problem.
///
/// It matters more now than it did, because there is no default endpoint to
/// fall back to: every reporting deployment types this variable by hand.
///
/// **Parsed with `url`, the same crate `reqwest` parses with, rather than
/// approximated.** The first version of this check hand-rolled the grammar to
/// avoid what it wrongly believed would be a new dependency — `url` has been an
/// unconditional one since issue #673, added there with the rule this check
/// should have followed: it must be *the same* parser `reqwest` uses, because
/// "a grant key computed by a second, hand-rolled reader is a bypass waiting to
/// be found". The hand-rolled version accepted five shapes `reqwest` rejects
/// outright:
/// `http://[::1/track` (unclosed bracket), `http://host:99999/track` and
/// `:65536` (port out of range), `http://host:abc/track`,
/// `http://host:8080:9090/track`, and `http://999.999.999.999/track`. Each one
/// resolved to `Report` and then dropped every batch — the exact failure the
/// check exists to prevent, reintroduced by the check itself. The IPv4-shaped-
/// host rule (`127.0.0.1.5` is rejected, `exa_mple.com` is not) is the tell
/// that the tail here is unbounded: an approximation of a grammar this fiddly
/// is a standing source of the same bug. One parser, and it is the transport's
/// own.
///
/// Two things are still checked beyond parsing, because `url` is happy with
/// both and `reqwest` is not:
///
/// * **the scheme.** `url` parses `ftp://collector.internal/track` and
///   `reqwest` will even *build* a request from it; the send then fails with
///   "URL scheme is not allowed". Measured, not assumed.
/// * **a non-empty host**, defensively. No input has been found where `url`
///   returns a parsed `http`/`https` URL with an empty host — `https://` is a
///   parse error, and `http:///track` is *not* the counter-example it looks
///   like, because `url` normalizes it to `http://track/`, taking the first
///   path segment as the host. The guard stays because "there is somewhere to
///   connect to" is the property actually being asserted, and it should not
///   rest on a normalization rule holding forever.
///
/// This asks a different question from the endpoint redaction in
/// `crate::analytics::boot` — that one is about what may be *printed* — so the
/// two are not two halves of one rule.
fn is_usable_endpoint(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|host| !host.is_empty())
}

/// Whether the client credential can cross `raw` without being readable on the
/// wire: `https`, or `http` to a loopback host.
///
/// This asks a different question from [`is_usable_endpoint`] — that one is
/// "can a client send here at all", this one is "may this client's secret go
/// there" — and they are kept apart because they resolve to different reasons
/// and send an operator to different edits.
///
/// The rule exists because of where the OpenPanel credential travels.
/// Mixpanel's token rode in the body of a request to one fixed `https` address
/// this crate chose; no configuration could downgrade it. OpenPanel's address
/// is typed by the operator and its secret is a **request header on every
/// request**, so `http://collector.internal/track` writes a long-lived write
/// credential to the network in cleartext once per event, forever
/// ([CWE-319](https://cwe.mitre.org/data/definitions/319.html)). A container
/// cannot verify anyone's claim that the network in between is private, so this
/// does not assume it.
///
/// **Loopback is the exception, and it is a real one.** Traffic to
/// `127.0.0.0/8`, `::1` or `localhost` **does not leave the host**: it goes over
/// the host's loopback interface and reaches no link anyone else is on, so there
/// is no wire between machines for it to be read off. It is also how the
/// collector is run beside the workload in development and in every gated test
/// in this crate. Refusing it would refuse the one `http` case that is actually
/// safe.
///
/// That is deliberately narrower than "nobody can see it", because the narrower
/// claim is the true one: a sufficiently privileged local process can capture
/// `lo`. It does not weaken the exception, though — anything with that access on
/// a tenant's host can already read the process environment the credential was
/// loaded from, so the capture gains it nothing it did not have. The property
/// this rests on is that the secret never crosses a network **between hosts**,
/// which is what CWE-319 is about.
///
/// `localhost` is matched **by exact name**, not by suffix. RFC 6761 reserves
/// `*.localhost` for loopback as well, and a resolver may honour that — but
/// "may" is not a property this check can rest a credential on, and the strict
/// subset is the safe direction: it can only refuse an endpoint that would have
/// worked, loudly, with a named reason and a one-character fix. Widening it
/// later costs nothing; narrowing it after a secret has shipped costs the
/// secret.
///
/// Matched on `Url::host()` rather than on the raw string, so that
/// `http://127.0.0.1:3000/track`, `http://[::1]/track` and
/// `http://user@localhost/track` are all judged on the host `url` actually
/// parsed out, and a value like `http://127.0.0.1.evil.example/track` — which
/// merely *starts* with a loopback address — is not.
pub(crate) fn is_secure_endpoint(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    // `Url` lower-cases the scheme while parsing, so `HTTPS://…` arrives here
    // as `https` and needs no case handling of its own.
    if parsed.scheme() == "https" {
        return true;
    }
    match parsed.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

/// Whether `raw` is something the transport could put in an HTTP header.
///
/// **Deliberately a strict subset of what `http::HeaderValue` accepts**, not an
/// approximation of it: every byte must be printable ASCII with no space
/// (`0x21..=0x7E`). `HeaderValue` is more permissive — it takes space, tab and
/// the whole `0xA0..=0xFF` range — so anything this accepts, `reqwest` accepts,
/// and the subset direction is the safe one. A check that were merely
/// *approximate* could accept a value the transport then refuses, which is
/// exactly the "boot said reporting and nothing was ever sent" failure
/// [`is_usable_endpoint`] exists to prevent; a strict subset cannot.
///
/// It is written here rather than deferred to `reqwest` for the reason
/// [`Silence::UnusableCredential`] gives: this module is un-gated and un-feature
/// -flagged on purpose, so the whole decision is provable in the default build,
/// with no network and no `reqwest` in the graph (`--no-default-features` drops
/// it entirely). The gated transport test asserts the subset claim against
/// `HeaderValue::from_str` itself, so the two cannot drift apart silently.
///
/// The cost of being strict is refusing a credential OpenPanel would have
/// accepted. An OpenPanel client id and secret are generated opaque tokens —
/// this has never been observed to reject one — and the failure is loud, named
/// and reversible, which is the direction to be wrong in.
fn is_header_safe(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

/// The tenant-identity key, if this deployment configured one.
///
/// Read through `get`, not `get_os`, and that is deliberate rather than an
/// oversight of the rule the switch and the endpoint follow: there is no
/// unsafe direction to fail into here. A key that cannot be read is treated as
/// absent, and absent means the host falls back to its random instance id —
/// which is *more* private than any keyed digest, not less. The distinction
/// `get_os` buys elsewhere ("malformed must not read as unset") only matters
/// when unset is the dangerous answer, and here it is the safe one.
pub fn tenant_id_key(env: &dyn EnvSource) -> Option<TenantIdKey> {
    non_blank(env, ID_KEY_ENV).and_then(TenantIdKey::new)
}

/// A configured value, trimmed, or `None` when there is nothing left of it.
///
/// [`EnvSource::get`] already drops an *empty* value, but not a whitespace-only
/// one, and the difference is not academic: a token mounted from a file arrives
/// with a trailing newline more often than not. Untrimmed, a hosted tenant whose
/// token is `"\n"` resolves to [`Decision::Report`], the boot line says
/// "reporting to …", and every batch is refused by the collector — the failure
/// mode #1739 added that line to prevent.
///
/// The endpoint is trimmed by [`resolve`] itself rather than here, because it
/// has to be read through [`EnvSource::get_os`] to tell an unreadable value from
/// an absent one.
///
/// The same trim-and-filter the rest of the tree applies to environment values
/// (`src/bin/opencompany.rs`).
fn non_blank(env: &dyn EnvSource, key: &str) -> Option<String> {
    env.get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::app::config::MapEnv;

    /// A collector address that resolves nowhere. Every reporting test needs
    /// one now: there is no default endpoint left to fall back to.
    const TEST_ENDPOINT: &str = "https://collector.invalid/track";

    /// A fully configured reporting environment, which `pairs` then overrides.
    ///
    /// It takes three variables where it used to take one, and that is the
    /// shape of the change: an OpenPanel deployment configures a client id, a
    /// client secret and the address of the collector it self-hosts.
    fn configured(pairs: &[(&str, &str)]) -> MapEnv {
        let mut all = vec![
            (CLIENT_ID_ENV, "not-a-real-client-id"),
            (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
            (ENDPOINT_ENV, TEST_ENDPOINT),
        ];
        all.extend_from_slice(pairs);
        MapEnv::new(all)
    }

    /// **The decision the GPL posture rests on.** A self-hosted instance that
    /// has been handed a working credential — which is the easiest way to get
    /// this wrong, because a credential looks like consent — still sends
    /// nothing.
    #[test]
    fn a_self_hosted_instance_is_silent_even_with_a_credential() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &configured(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_desktop_instance_is_silent_even_with_a_credential() {
        assert_eq!(
            resolve(Deployment::Desktop, &configured(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    #[test]
    fn a_hosted_tenant_with_a_credential_reports() {
        let decision = resolve(Deployment::HostedTenant, &configured(&[]));
        assert!(decision.reports(), "{decision:?}");
        match decision {
            Decision::Report {
                endpoint,
                credentials,
            } => {
                assert_eq!(endpoint, TEST_ENDPOINT);
                assert_eq!(credentials.expose_id(), "not-a-real-client-id");
                assert_eq!(credentials.expose_secret(), "not-a-real-client-secret");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A hosted tenant with nothing configured is misconfigured, not reporting
    /// to nowhere — and the reason says so.
    #[test]
    fn a_hosted_tenant_without_a_credential_is_silent() {
        assert_eq!(
            resolve(Deployment::HostedTenant, &MapEnv::default()),
            Decision::Silent(Silence::NoCredentials)
        );
    }

    /// **Half a credential is a misconfiguration, and the reason names the half
    /// that is missing.**
    ///
    /// OpenPanel authenticates a write client with an id *and* a secret, so
    /// there is no useful state in between. This is the shape a half-finished
    /// secret rollout has — the id is in the manifest, the secret is still in
    /// the vault — and telling that operator "no credential is configured"
    /// while `OPENCOMPANY_ANALYTICS_CLIENT_ID` is plainly set in their env file
    /// sends them to look at the wrong variable.
    #[test]
    fn half_a_credential_says_which_half_is_missing() {
        let only_id = MapEnv::new([
            (CLIENT_ID_ENV, "not-a-real-client-id"),
            (ENDPOINT_ENV, TEST_ENDPOINT),
        ]);
        assert_eq!(
            resolve(Deployment::HostedTenant, &only_id),
            Decision::Silent(Silence::NoClientSecret)
        );
        assert!(
            Silence::NoClientSecret
                .as_str()
                .contains("OPENCOMPANY_ANALYTICS_CLIENT_SECRET"),
            "the reason must name the variable to set: {}",
            Silence::NoClientSecret.as_str()
        );

        let only_secret = MapEnv::new([
            (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
            (ENDPOINT_ENV, TEST_ENDPOINT),
        ]);
        assert_eq!(
            resolve(Deployment::HostedTenant, &only_secret),
            Decision::Silent(Silence::NoClientId)
        );
        assert!(
            Silence::NoClientId
                .as_str()
                .contains("OPENCOMPANY_ANALYTICS_CLIENT_ID"),
            "the reason must name the variable to set: {}",
            Silence::NoClientId.as_str()
        );
    }

    /// Blank is absent for both halves, and for the same reason it is for the
    /// switch: a secret mounted from a file arrives with a trailing newline
    /// more often than not, and a launcher that exports an empty variable has
    /// configured nothing.
    #[test]
    fn a_blank_half_is_no_half() {
        for blank in ["   ", "\n", "\t\n "] {
            assert_eq!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[(CLIENT_SECRET_ENV, blank)])
                ),
                Decision::Silent(Silence::NoClientSecret),
                "a secret of {blank:?} must not read as configured"
            );
            assert_eq!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[(CLIENT_ID_ENV, blank)])
                ),
                Decision::Silent(Silence::NoClientId),
                "an id of {blank:?} must not read as configured"
            );
        }
    }

    /// And a credential that merely *arrived* with surrounding whitespace is
    /// used, trimmed, rather than put into a header with a newline in it — which
    /// `reqwest` rejects outright when it builds the request.
    #[test]
    fn a_credential_is_trimmed() {
        match resolve(
            Deployment::HostedTenant,
            &configured(&[
                (CLIENT_ID_ENV, "  not-a-real-client-id\n"),
                (CLIENT_SECRET_ENV, "\tnot-a-real-client-secret\n"),
            ]),
        ) {
            Decision::Report { credentials, .. } => {
                assert_eq!(credentials.expose_id(), "not-a-real-client-id");
                assert_eq!(credentials.expose_secret(), "not-a-real-client-secret");
            }
            other => panic!("{other:?}"),
        }
    }

    /// **A credential that cannot go in a header is silence with a reason.**
    ///
    /// This is new with OpenPanel and is a consequence of where the credential
    /// now travels. Mixpanel's token rode in the JSON body, where any string is
    /// legal, so a mangled one was simply refused by the collector. These two
    /// ride in `openpanel-client-id` / `openpanel-client-secret` headers, and
    /// `reqwest` refuses to *build* a request whose header value holds a control
    /// byte — so a secret with an embedded newline (`kubectl create secret` over
    /// a wrapped file is the usual way one arrives) would install a tracker that
    /// never constructs a single request, forever, behind a `debug!` nobody has
    /// enabled. Trimming does not save it: the newline is in the middle.
    #[test]
    fn a_credential_that_cannot_go_in_a_header_is_silence() {
        for mangled in [
            "not-a-real\nclient-secret",
            "not-a-real\rclient-secret",
            "not a real client secret",
            "not-a-real-client-secret\u{0}",
            "not-a-r\u{e9}al-client-secret",
        ] {
            assert_eq!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[(CLIENT_SECRET_ENV, mangled)])
                ),
                Decision::Silent(Silence::UnusableCredential),
                "a secret of {mangled:?} must not resolve to a report that cannot be built"
            );
            assert_eq!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[(CLIENT_ID_ENV, mangled)])
                ),
                Decision::Silent(Silence::UnusableCredential),
                "an id of {mangled:?} must not resolve to a report that cannot be built"
            );
        }

        // The control, without which "reject everything" would pass: the shapes
        // an OpenPanel client actually has still report. Opaque generated
        // tokens — hex, base64url, a uuid, a prefixed key.
        for real_shaped in [
            "0f8b1c2d3e4f5a6b7c8d9e0f1a2b3c4d",
            "op_sk_9zQx-4Kd_7Yb2Lp0",
            "550e8400-e29b-41d4-a716-446655440000",
            "YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo=",
        ] {
            assert!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[
                        (CLIENT_ID_ENV, real_shaped),
                        (CLIENT_SECRET_ENV, real_shaped)
                    ])
                )
                .reports(),
                "{real_shaped:?} is the shape a real credential has and must still report"
            );
        }
    }

    /// And the reason never quotes the credential it rejected, for the same
    /// reason the endpoint reason does not quote the endpoint.
    #[test]
    fn the_unusable_credential_reason_never_quotes_the_credential() {
        let reason = Silence::UnusableCredential.as_str();
        let printed = format!("{:?} {reason}", Silence::UnusableCredential);
        assert!(
            !printed.to_ascii_lowercase().contains("not-a-real"),
            "the reason leaked the credential: {printed}"
        );
        assert!(
            reason.contains("header"),
            "the reason must say what is wrong with it: {reason}"
        );
    }

    /// **There is no default endpoint, and an absent one is silence with its
    /// own reason.**
    ///
    /// This replaced `https://api.mixpanel.com/track`, and dropping the default
    /// rather than re-pointing it is the deliberate half of that. OpenPanel is
    /// self-hosted: its address is whatever the operator runs it at, and any
    /// address this crate picked would be somebody else's collector. A tenant
    /// that configured a credential but no endpoint would then have shipped its
    /// telemetry to a third party nobody named — which is the accident
    /// `Silence::UnusableEndpoint` already refuses to make from the other
    /// direction.
    #[test]
    fn an_absent_endpoint_is_silence_rather_than_a_default() {
        let decision = resolve(
            Deployment::HostedTenant,
            &MapEnv::new([
                (CLIENT_ID_ENV, "not-a-real-client-id"),
                (CLIENT_SECRET_ENV, "not-a-real-client-secret"),
            ]),
        );
        assert_eq!(decision, Decision::Silent(Silence::NoEndpoint));
        assert!(!decision.reports());
        assert!(
            Silence::NoEndpoint
                .as_str()
                .contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
            "the reason must name the variable to set: {}",
            Silence::NoEndpoint.as_str()
        );
    }

    /// A blank endpoint is an absent one, not a broken one: a launcher that
    /// exports an empty variable has configured nothing, and the reason it gets
    /// should send it to set the variable rather than to fix its value.
    #[test]
    fn a_blank_endpoint_is_absent_rather_than_unusable() {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, "  \n")])
            ),
            Decision::Silent(Silence::NoEndpoint)
        );
    }

    /// `off` outranks the deployment kind. The platform can switch a tenant off
    /// without rebuilding it.
    #[test]
    fn off_outranks_a_hosted_deployment() {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENABLE_ENV, "off")])
            ),
            Decision::Silent(Silence::OptedOut)
        );
    }

    /// The self-hoster's opt-in, which is the only way a non-hosted install ever
    /// reports.
    #[test]
    fn a_self_hoster_can_opt_in() {
        assert!(resolve(Deployment::SelfHosted, &configured(&[(ENABLE_ENV, "on")])).reports());
    }

    /// A typo must not opt anybody in.
    #[test]
    fn a_misspelled_switch_does_not_opt_in() {
        assert_eq!(
            resolve(Deployment::SelfHosted, &configured(&[(ENABLE_ENV, "onn")])),
            Decision::Silent(Silence::Unreadable)
        );
    }

    /// **And a typo must not fail to opt anybody out.** This is the direction
    /// that used to leak: an unreadable value fell through to the deployment
    /// default, so a hosted tenant whose operator meant `off` and typed `of`
    /// carried on reporting, with a boot line that said "reporting to …" and
    /// gave them no reason to look again.
    #[test]
    fn a_misspelled_opt_out_does_not_keep_a_hosted_tenant_reporting() {
        for typo in ["of", "offf", "disabled", "0.0", "nope"] {
            let decision = resolve(Deployment::HostedTenant, &configured(&[(ENABLE_ENV, typo)]));
            assert_eq!(
                decision,
                Decision::Silent(Silence::Unreadable),
                "{typo:?} must not leave a hosted tenant reporting"
            );
            assert!(!decision.reports(), "{typo:?}");
        }
    }

    /// **A switch that is set but is not text fails closed too.**
    ///
    /// `EnvSource::get` maps a non-Unicode value to `None`, so reading through
    /// it would have treated `OPENCOMPANY_ANALYTICS=<invalid bytes>` as an
    /// absent switch and left a hosted tenant reporting — the same leak as the
    /// unreadable spelling, by a different route.
    #[cfg(unix)]
    #[test]
    fn a_non_unicode_switch_is_unreadable_rather_than_absent() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        struct NonUnicodeSwitch;
        impl EnvSource for NonUnicodeSwitch {
            fn get_os(&self, key: &str) -> Option<OsString> {
                match key {
                    ENABLE_ENV => Some(OsString::from_vec(vec![0xff, 0xfe, 0x6f, 0x6e])),
                    CLIENT_ID_ENV => Some(OsString::from("not-a-real-client-id")),
                    CLIENT_SECRET_ENV => Some(OsString::from("not-a-real-client-secret")),
                    ENDPOINT_ENV => Some(OsString::from(TEST_ENDPOINT)),
                    _ => None,
                }
            }
        }

        // The premise: this really is a value `get` cannot see at all.
        assert_eq!(NonUnicodeSwitch.get(ENABLE_ENV), None);
        assert!(NonUnicodeSwitch.get_os(ENABLE_ENV).is_some());

        assert_eq!(
            resolve(Deployment::HostedTenant, &NonUnicodeSwitch),
            Decision::Silent(Silence::Unreadable),
            "a switch set to bytes this process cannot read must not read as unset"
        );
    }

    /// The near-miss control: `off` really is matched case-insensitively and
    /// after trimming, so the test above is finding typos rather than finding
    /// every value that is not lowercase and bare.
    #[test]
    fn an_off_switch_is_trimmed_and_case_folded() {
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENABLE_ENV, "  ofF\n")])
            ),
            Decision::Silent(Silence::OptedOut)
        );
    }

    /// The control for the two above: an **absent** switch still falls to the
    /// deployment default, in both directions. Without this, "everything is
    /// silent now" would pass the tests above just as well.
    #[test]
    fn an_absent_switch_still_falls_to_the_deployment_default() {
        assert!(resolve(Deployment::HostedTenant, &configured(&[])).reports());
        assert_eq!(
            resolve(Deployment::SelfHosted, &configured(&[])),
            Decision::Silent(Silence::NotHosted)
        );
    }

    /// A whitespace-only switch is an absent switch, not an unreadable one —
    /// consistent with the credential and endpoint, and it must not flip a
    /// hosted tenant into silence just because a launcher exported an empty
    /// variable.
    #[test]
    fn a_blank_switch_is_treated_as_absent() {
        assert!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENABLE_ENV, "   ")])
            )
            .reports(),
            "a blank switch must not read as unreadable"
        );
    }

    /// The positive control for the endpoint group, and deliberately
    /// **insensitive** to the trim: no surrounding whitespace, so this test
    /// passes both with the filter and without it. Without such a control,
    /// "every test in the group fails when I revert the fix" would be evidence
    /// that the group asserts the implementation rather than the behaviour.
    #[test]
    fn a_configured_endpoint_is_reported_to_exactly() {
        match resolve(
            Deployment::HostedTenant,
            &configured(&[(ENDPOINT_ENV, "http://127.0.0.1:9/track")]),
        ) {
            Decision::Report { endpoint, .. } => assert_eq!(endpoint, "http://127.0.0.1:9/track"),
            other => panic!("{other:?}"),
        }
    }

    /// **A malformed endpoint is silence with a reason, not reporting.**
    ///
    /// `collector.internal/track` — a hostname written without a scheme, which
    /// is how anyone would first write one — used to resolve to
    /// `Decision::Report`. Boot printed "reporting to collector.internal/track",
    /// the tracker was installed, and every send died inside `reqwest` behind a
    /// `debug!` line no operator has enabled. The product said something
    /// true-sounding and then did nothing, which is the one failure this module
    /// exists to make impossible — and it matters more now that every reporting
    /// deployment types this variable by hand.
    #[test]
    fn a_malformed_endpoint_is_silence_rather_than_a_broken_report() {
        for unusable in [
            "collector.internal/track",
            "collector.internal",
            "/track",
            "://collector.internal/track",
            "ftp://collector.internal/track",
            "file:///tmp/track",
            "https://",
            "http://someone:hunter2@/track",
            "http://collector internal/track",
        ] {
            let decision = resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, unusable)]),
            );
            assert_eq!(
                decision,
                Decision::Silent(Silence::UnusableEndpoint),
                "{unusable:?} must not resolve to a report that cannot be sent"
            );
            assert!(!decision.reports(), "{unusable:?}");
        }
    }

    /// The reason names the variable and **never the value**: a collector
    /// fronted by an authenticated proxy carries its key in the very URL that
    /// was rejected, so quoting the bad value would put a credential in the boot
    /// line of every misconfigured tenant. Asserted case-insensitively, because
    /// a guard that matched exact case would read a lowercased leak as clean.
    #[test]
    fn the_unusable_endpoint_reason_never_quotes_the_endpoint() {
        const SECRET: &str = "NotARealCollectorKey";
        let reason = Silence::UnusableEndpoint.as_str();
        assert!(
            reason.contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
            "the reason must name the variable to act on: {reason}"
        );

        // Rejected for having no scheme, and carrying a credential while it is
        // rejected — which is exactly the case that would leak.
        let raw = format!("collector.internal/track?key={SECRET}");
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, raw.as_str())])
            ),
            Decision::Silent(Silence::UnusableEndpoint)
        );
        let printed = format!("{:?} {}", Silence::UnusableEndpoint, reason);
        assert!(
            !printed
                .to_ascii_lowercase()
                .contains(&SECRET.to_ascii_lowercase()),
            "the reason leaked the endpoint credential: {printed}"
        );
        // The self-check: the needle really is findable in the unredacted
        // value, in whatever case it comes back, or the guard above is vacuous.
        assert!(
            raw.to_ascii_lowercase()
                .contains(&SECRET.to_ascii_lowercase())
                && raw
                    .to_ascii_uppercase()
                    .to_ascii_lowercase()
                    .contains(&SECRET.to_ascii_lowercase()),
            "the needle must be findable before redaction: {raw}"
        );
    }

    /// **A non-Unicode endpoint is unusable, not absent.**
    ///
    /// It reads through `get_os` rather than `get` so that the two stay
    /// distinguishable. `get` maps unreadable bytes to `None`, which would tell
    /// an operator who mistyped their collector address that they had never set
    /// one — sending them to add a variable that is already there.
    ///
    /// Under the endpoint default this replaced, the same confusion was
    /// materially worse: unreadable bytes fell back to `api.mixpanel.com`, so a
    /// tenant that pointed analytics at its own collector and mistyped it
    /// reported to a **third party** instead. There is no default left for it
    /// to fall into, so this is now a diagnostic distinction rather than a
    /// containment one — but it is the same read, kept for the same reason.
    #[cfg(unix)]
    #[test]
    fn a_non_unicode_endpoint_is_unusable_rather_than_absent() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        struct NonUnicodeEndpoint;
        impl EnvSource for NonUnicodeEndpoint {
            fn get_os(&self, key: &str) -> Option<OsString> {
                match key {
                    ENDPOINT_ENV => Some(OsString::from_vec(
                        [b"https://collector.invalid/".as_slice(), &[0xff, 0xfe]].concat(),
                    )),
                    CLIENT_ID_ENV => Some(OsString::from("not-a-real-client-id")),
                    CLIENT_SECRET_ENV => Some(OsString::from("not-a-real-client-secret")),
                    _ => None,
                }
            }
        }

        // The premise: a value `get` cannot see at all.
        assert_eq!(NonUnicodeEndpoint.get(ENDPOINT_ENV), None);
        assert!(NonUnicodeEndpoint.get_os(ENDPOINT_ENV).is_some());

        let decision = resolve(Deployment::HostedTenant, &NonUnicodeEndpoint);
        assert_eq!(decision, Decision::Silent(Silence::UnusableEndpoint));
        match &decision {
            Decision::Report { endpoint, .. } => {
                panic!("reported to {endpoint} — an endpoint the operator never configured")
            }
            Decision::Silent(_) => {}
        }
    }

    /// **The endpoint check agrees with what `reqwest` can actually send to.**
    ///
    /// Every row was measured against reqwest 0.12.28 — `Url::parse`,
    /// `Client::post(..).build()`, and for the scheme, what the send does — not
    /// reasoned about. The rows marked below are the ones a hand-rolled grammar
    /// check accepted and `reqwest` rejects; they resolved to `Decision::Report`
    /// and then dropped every event, which is the very failure
    /// `is_usable_endpoint` exists to prevent.
    #[test]
    fn the_endpoint_check_matches_what_the_transport_accepts() {
        // (endpoint, refusal) — `None` reports, `Some(reason)` is silence with
        // that reason. `UnusableEndpoint` means `reqwest` cannot send to it at
        // all; `InsecureEndpoint` means it could, and must not, because the
        // credential would be readable on the way.
        let measured: &[(&str, Option<Silence>)] = &[
            // Rejected by `Url::parse`. Each of these was accepted by the
            // hand-rolled check this replaced.
            ("http://[::1/track", Some(Silence::UnusableEndpoint)), // unclosed IPv6 bracket
            ("http://]::1[/track", Some(Silence::UnusableEndpoint)), // brackets inside out
            (
                "http://collector.internal:99999/track",
                Some(Silence::UnusableEndpoint),
            ), // port out of range
            (
                "http://collector.internal:65536/track",
                Some(Silence::UnusableEndpoint),
            ), // one past the top
            (
                "http://collector.internal:abc/track",
                Some(Silence::UnusableEndpoint),
            ), // port not a number
            (
                "http://host:8080:9090/track",
                Some(Silence::UnusableEndpoint),
            ), // two ports
            ("http://127.0.0.1.5/track", Some(Silence::UnusableEndpoint)), // IPv4-shaped, invalid
            (
                "http://999.999.999.999/track",
                Some(Silence::UnusableEndpoint),
            ), // IPv4-shaped, invalid
            // Rejected by `Url::parse` and by the hand-rolled check alike.
            ("collector.internal/track", Some(Silence::UnusableEndpoint)),
            ("collector.internal", Some(Silence::UnusableEndpoint)),
            ("/track", Some(Silence::UnusableEndpoint)),
            (
                "://collector.internal/track",
                Some(Silence::UnusableEndpoint),
            ),
            ("https://", Some(Silence::UnusableEndpoint)),
            (
                "http://someone:hunter2@/track",
                Some(Silence::UnusableEndpoint),
            ),
            (
                "http://collector internal/track",
                Some(Silence::UnusableEndpoint),
            ),
            // Parsed happily by `url` — and even built by `reqwest` — but not
            // sendable, so checked on top of the parse.
            (
                "ftp://collector.internal/track",
                Some(Silence::UnusableEndpoint),
            ), // scheme refused at send
            ("file:///tmp/track", Some(Silence::UnusableEndpoint)),
            // NOT here: `http:///track`. It looks like an empty host and is
            // not one — `url` normalizes it to `http://track/`, taking the
            // first path segment as the host, and `reqwest` sends to it. A
            // collector named `track` that does not resolve is an unreachable
            // collector like any other, which #1739 makes a no-op on purpose.
            //
            // Sendable, but plain `http` to a host that is not loopback: the
            // client secret is a header on every request, so these would put it
            // on the wire in the clear. Each was accepted before the
            // `InsecureEndpoint` rule.
            (
                "http://collector.internal:65535/track",
                Some(Silence::InsecureEndpoint),
            ), // the top of the range
            (
                "http://collector.internal:/track",
                Some(Silence::InsecureEndpoint),
            ), // empty port is legal
            ("http://exa_mple.com/track", Some(Silence::InsecureEndpoint)),
            ("http://-example.com/track", Some(Silence::InsecureEndpoint)),
            (
                "http://\u{4f8b}\u{3048}.jp/track",
                Some(Silence::InsecureEndpoint),
            ),
            // A host that merely *starts* with a loopback address is not one.
            (
                "http://127.0.0.1.evil.example/track",
                Some(Silence::InsecureEndpoint),
            ),
            (
                "http://localhost.evil.example/track",
                Some(Silence::InsecureEndpoint),
            ),
            // Accepted, and the ones a deployment actually uses: `https`
            // anywhere, and `http` only to loopback.
            (TEST_ENDPOINT, None),
            ("http://127.0.0.1:9/track", None),
            ("http://127.0.0.1:9", None),
            ("http://localhost:9/track", None),
            ("http://LOCALHOST:9/track", None),
            ("https://collector.internal/track", None),
            ("HTTPS://collector.internal/track", None),
            (
                "https://collector.internal/track?key=NotARealCollectorKey",
                None,
            ),
            (
                "https://someone:NotARealCollectorKey@collector.internal/track",
                None,
            ),
            ("https://[::1]:8443/track", None),
            ("http://[::1]/track", None),
            ("https://collector.internal:8443/track#frag", None),
        ];

        for (endpoint, refusal) in measured {
            let decision = resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, endpoint)]),
            );
            match refusal {
                None => match decision {
                    Decision::Report { endpoint: got, .. } => assert_eq!(&got, endpoint),
                    other => panic!("{endpoint:?} must still report: {other:?}"),
                },
                Some(reason) => assert_eq!(
                    decision,
                    Decision::Silent(*reason),
                    "{endpoint:?} must resolve to silence with {reason:?}"
                ),
            }
        }
    }

    /// **A plain `http` endpoint to a non-loopback host is silence, not a
    /// credential in the clear.**
    ///
    /// The OpenPanel client secret is a request header on *every* request, so
    /// `OPENCOMPANY_ANALYTICS_ENDPOINT=http://collector.internal/track` writes a
    /// long-lived write credential to the network in cleartext once per event
    /// for the life of the tenant (CWE-319). Mixpanel had no equivalent
    /// exposure: its token rode in the body of a request to one fixed `https`
    /// address this crate chose, and no configuration could downgrade it.
    ///
    /// Silence rather than a warning-and-send, because a warning is a line
    /// nobody reads while the secret ships anyway, and a disclosed credential
    /// cannot be un-disclosed once noticed. The reason names the variable and
    /// the two ways out.
    #[test]
    fn a_cleartext_endpoint_is_silence_rather_than_a_credential_on_the_wire() {
        for insecure in [
            "http://collector.internal/track",
            "http://collector.internal:8080/track",
            "http://10.0.0.5:3000/track",
            "http://192.168.1.10/track",
            "http://[2001:db8::1]/track",
            "http://collector.example.com/api/track",
            // Not loopback, however much it looks like it.
            "http://127.0.0.1.evil.example/track",
            "http://localhost.evil.example/track",
        ] {
            let decision = resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, insecure)]),
            );
            assert_eq!(
                decision,
                Decision::Silent(Silence::InsecureEndpoint),
                "{insecure:?} would send the client secret in the clear"
            );
            assert!(!decision.reports(), "{insecure:?}");
        }
    }

    /// The control that keeps the test above from passing by rejecting every
    /// `http` URL: **loopback `http` is the documented exception and still
    /// reports.**
    ///
    /// It is not a concession — it is the only `http` case that is actually
    /// safe, because that traffic does not leave the host and so never crosses
    /// a network between machines. It is also
    /// how the collector is run beside the workload in development, and how
    /// every gated transport test in this crate points at its own local
    /// collector; without this arm those tests would be asserting against a
    /// tracker that resolve had already silenced.
    #[test]
    fn loopback_http_is_the_one_cleartext_endpoint_that_still_reports() {
        for loopback in [
            "http://127.0.0.1:3000/track",
            "http://127.0.0.1/track",
            "http://127.1.2.3:9/track",
            "http://[::1]:3000/track",
            "http://[::1]/track",
            "http://localhost:3000/track",
            "http://LocalHost:3000/track",
        ] {
            match resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, loopback)]),
            ) {
                Decision::Report { endpoint, .. } => assert_eq!(endpoint, loopback),
                other => panic!("{loopback:?} is loopback and must still report: {other:?}"),
            }
        }
    }

    /// The insecure reason names the variable and the fix, and — like every
    /// other reason here — **never quotes the value**.
    ///
    /// This one matters more than most: the endpoint it is rejecting is by
    /// definition one an operator typed, and a self-hosted collector is
    /// routinely fronted by an authenticated proxy that carries its key in the
    /// URL. Quoting the rejected value would print that key in the boot line of
    /// every tenant the new rule silences.
    #[test]
    fn the_insecure_endpoint_reason_never_quotes_the_endpoint() {
        const SECRET: &str = "NotARealCollectorKey";
        let reason = Silence::InsecureEndpoint.as_str();
        assert!(
            reason.contains("OPENCOMPANY_ANALYTICS_ENDPOINT"),
            "the reason must name the variable to act on: {reason}"
        );
        assert!(
            reason.contains("https") && reason.contains("loopback"),
            "the reason must name both ways out: {reason}"
        );

        let raw = format!("http://collector.internal/track?key={SECRET}");
        assert_eq!(
            resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, raw.as_str())])
            ),
            Decision::Silent(Silence::InsecureEndpoint)
        );
        let printed = format!("{:?} {}", Silence::InsecureEndpoint, reason);
        assert!(
            !printed
                .to_ascii_lowercase()
                .contains(&SECRET.to_ascii_lowercase()),
            "the reason leaked the endpoint credential: {printed}"
        );
        // The self-check: the needle really is findable in the unredacted
        // value, or the guard above is vacuous.
        assert!(
            raw.to_ascii_lowercase()
                .contains(&SECRET.to_ascii_lowercase()),
            "the needle must be findable before redaction: {raw}"
        );
    }

    /// **Shape is judged before transport security**, so the two reasons stay
    /// distinguishable and each sends an operator to the edit it names.
    ///
    /// `http://collector.internal:99999/track` is both unparseable *and* plain
    /// http; it must be reported as unusable, because there is no host to judge
    /// until it parses and "this will not parse" is the more actionable half.
    #[test]
    fn an_unparseable_cleartext_endpoint_is_unusable_rather_than_insecure() {
        for both in [
            "http://collector.internal:99999/track",
            "http://collector internal/track",
            "http://[::1/track",
        ] {
            assert_eq!(
                resolve(
                    Deployment::HostedTenant,
                    &configured(&[(ENDPOINT_ENV, both)])
                ),
                Decision::Silent(Silence::UnusableEndpoint),
                "{both:?} does not parse, so the reason must be about the parse"
            );
        }
    }

    /// The controls that keep the group above from passing by rejecting
    /// everything: the endpoints a deployment actually uses still resolve, and
    /// still resolve to themselves.
    #[test]
    fn a_usable_endpoint_still_reports_to_exactly_itself() {
        for usable in [
            TEST_ENDPOINT,
            "http://127.0.0.1:9/track",
            "http://127.0.0.1:9",
            "https://collector.internal/track",
            "HTTPS://collector.internal/track",
            "https://collector.internal/track?key=NotARealCollectorKey",
            "https://someone:NotARealCollectorKey@collector.internal/track",
            "https://[::1]:8443/track",
            "https://collector.internal:8443/track#frag",
        ] {
            match resolve(
                Deployment::HostedTenant,
                &configured(&[(ENDPOINT_ENV, usable)]),
            ) {
                Decision::Report { endpoint, .. } => assert_eq!(endpoint, usable),
                other => panic!("{usable:?} must still report: {other:?}"),
            }
        }
    }

    /// The credential must not be printable by accident, because the accident is
    /// a `{:?}` in a log line nobody reviewed.
    ///
    /// **Both halves**, id included. OpenPanel's own web SDK treats a client id
    /// as public, but there is no line in this tree that is better for carrying
    /// it, and a type with one printable field and one redacted one is a type
    /// someone eventually prints in full.
    #[test]
    fn neither_half_of_the_credential_is_printable() {
        let credentials =
            ClientCredentials::new("not-a-real-client-id", "not-a-real-client-secret");
        let printed = format!("{credentials:?}");
        for half in ["not-a-real-client-id", "not-a-real-client-secret"] {
            assert!(
                !printed.contains(half),
                "the Debug impl leaked {half}: {printed}"
            );
        }

        let decision = Decision::Report {
            endpoint: TEST_ENDPOINT.to_string(),
            credentials,
        };
        let printed = format!("{decision:?}");
        for half in ["not-a-real-client-id", "not-a-real-client-secret"] {
            assert!(
                !printed.contains(half),
                "the Debug impl leaked {half} through the decision: {printed}"
            );
        }
    }
}
