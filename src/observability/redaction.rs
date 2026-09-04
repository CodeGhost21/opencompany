//! Removing credentials from text on its way out of the process.
//!
//! Applied to every string a crash report can carry — the event message, the
//! log entry, each exception value, each breadcrumb, and every string leaf of
//! the structured fields a `tracing` event brings with it
//! (`docs/spec/runtime/crash-reporting.md`).
//!
//! # Why this file is not behind the feature
//!
//! It names no `sentry::` type and it is the part that has to be right, so it
//! compiles and is tested in every build — including the default one, where a
//! `crash-reporting` lane would never run it. That is the same argument
//! `analytics::config` makes for the enable/disable decision, and it is the
//! reason the `crash-reporting` feature is a *second* gate rather than the only
//! one.
//!
//! # Why it is hand-written rather than a set of regexes
//!
//! The vendored runtime's equivalent (`core::log_redaction`) is seven regexes,
//! and its scars are instructive: `token[=:\s]+\S+` matched
//! `cancellation_token=` and `next_page_token=` until a `\b` was added, and the
//! generic `sk-[A-Za-z0-9]{20,}` left a trailing `_uv` behind on any key with a
//! separator in it until the character class grew.
//!
//! Both failures are the same failure — a regex over raw text has no idea where
//! a *word* begins and ends — so this splits the text into tokens first and
//! asks its questions of whole tokens. `cancellation_token` is one token and
//! normalises to `cancellationtoken`, which is not a secret-bearing key, so the
//! false positive cannot arise rather than being patched out of it. A secret
//! with a separator inside is one token, so there is no fragment to leave
//! behind.
//!
//! It also avoids making `regex` an unconditional dependency of this crate. It
//! is optional today (`Cargo.toml`, under `openhuman`), and a security control
//! that must run in every build cannot be built on a crate that does not.
//!
//! # What this is and is not
//!
//! It is a **last line of defence**, not the first. The first is not putting a
//! credential in a message: `SecretValue` (`ports::types`) exists so a
//! credential is not `Display`, and `analytics::config::ProjectToken` and
//! [`super::config::Dsn`] both refuse to `Debug` themselves. A scrubber is
//! heuristic by construction — it cannot recognise a secret that looks like a
//! word — so a call site that relies on it is one release away from leaking.

use std::borrow::Cow;

/// What a redacted span is replaced with. Deliberately not the empty string: a
/// report that says a value was removed is diagnosable, and one that silently
/// lost a field looks like a bug in the reporter.
pub const REDACTED: &str = "[redacted]";

/// Prefixes that identify a credential on their own, whatever surrounds them.
///
/// This is the half that catches a secret nobody labelled — a bare key pasted
/// into a message, or one embedded in a provider's own error text, which is
/// where the vendored runtime found most of them.
///
/// Case-sensitive on purpose: `AKIA` is an AWS key id and `akia` is a word.
const SECRET_PREFIXES: &[&str] = &[
    // OpenAI, Anthropic, Stripe and everything that copied them.
    "sk-",
    "sk_",
    "rk_",
    "pk_",
    // GitHub: PATs, OAuth, server-to-server, user-to-server, refresh, and the
    // fine-grained form.
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "ghr_",
    "github_pat_",
    // GitLab.
    "glpat-",
    // Slack bot/user/app/legacy tokens and app-level tokens.
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-",
    "xoxe-",
    "xapp-",
    // AWS long-lived and session access key ids.
    "AKIA",
    "ASIA",
    // TinyHumans — this crate's own hosted credential
    // (`company::credentials::API_KEY_ENV`).
    "th_",
    // Shopify, npm, DigitalOcean, SendGrid.
    "shpat_",
    "shpss_",
    "npm_",
    "dop_v1_",
    "SG.",
];

/// The shortest a prefixed token may be before it is treated as a credential.
///
/// A floor rather than an exact length, because every issuer picks its own and
/// several have changed it. It exists to keep a sentence like "the sk- prefix"
/// from being redacted, not to validate anything.
const MIN_PREFIXED_LEN: usize = 12;

/// Keys whose *value* is a credential, normalised by [`normalize_key`].
///
/// The list is deliberately specific. `key` is not on it and neither is `id`:
/// both appear constantly in ordinary diagnostics, and a scrubber that eats
/// half of every message is one an operator turns off.
const SECRET_KEYS: &[&str] = &[
    "token",
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "authtoken",
    "sessiontoken",
    "bearertoken",
    "apikey",
    "apitoken",
    "apisecret",
    "secret",
    "secretkey",
    "clientsecret",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "authorization",
    "credential",
    "credentials",
    "privatekey",
    "signingkey",
    "dsn",
];

/// HTTP authentication schemes, as *values*: the word before the credential,
/// never the credential.
///
/// Exempt from redaction, because `Authorization: Bearer abc123` would
/// otherwise redact the scheme name and leave `abc123` standing — worse than
/// doing nothing, since the message then *looks* scrubbed. `token` is on this
/// list for GitHub's `Authorization: token <pat>` form.
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "digest", "negotiate", "token"];

/// The subset of [`AUTH_SCHEMES`] that, as a *key*, licenses redacting the next
/// token across a bare space — `Bearer abc123` has no `=` or `:` between the
/// two, and no other reading of those two words exists.
///
/// `token` is deliberately **not** here, though it is a scheme in
/// `Authorization: token <pat>`. As an English word it is far too common —
/// "the token was rejected by the provider" would lose `was` — and the ordinary
/// prose case is the one that has to keep working, or an operator turns this
/// off. The `Authorization: token <pat>` form is not lost in practice: a PAT
/// carries an issuer prefix and [`looks_like_a_secret`] catches it on its own.
const SCHEME_KEYS: &[&str] = &["bearer", "basic", "digest", "negotiate"];

/// Removes credentials from `text`.
///
/// Borrows when there is nothing to remove, which is the overwhelmingly common
/// case: this runs on every string of every event, and most events carry none.
pub fn scrub(text: &str) -> Cow<'_, str> {
    match scrub_url_userinfo(text) {
        Cow::Borrowed(borrowed) => scrub_tokens(borrowed),
        Cow::Owned(owned) => Cow::Owned(scrub_tokens(&owned).into_owned()),
    }
}

/// Bytes that may appear inside one token.
///
/// ASCII-only by construction, so every index this yields is a `char`
/// boundary. `=` and `/` are excluded even though base64 uses both: including
/// `=` would swallow the `token=value` separator this pass depends on, and
/// including `/` would make a whole URL one token. A trailing `==` of padding
/// left behind reveals nothing.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'+' | b'~')
}

/// The comparable form of a key token.
///
/// Three normalisations, each earning its place:
///
/// * everything after the last `.`, so `config.token` and `settings.api_key`
///   are the keys they name rather than opaque paths;
/// * `-` and `_` removed, so `api-key`, `api_key`, `apiKey` and the `--api-key`
///   flag are one key;
/// * lower-cased.
///
/// The word-boundary bug this replaces is gone by construction:
/// `cancellation_token` is a single token and normalises to
/// `cancellationtoken`, which is not in [`SECRET_KEYS`].
fn normalize_key(token: &str) -> String {
    let tail = token.rsplit('.').next().unwrap_or(token);
    tail.chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether a token names itself as a credential.
fn looks_like_a_secret(token: &str) -> bool {
    if token.len() >= MIN_PREFIXED_LEN
        && SECRET_PREFIXES
            .iter()
            .any(|prefix| token.starts_with(prefix))
    {
        return true;
    }
    // A JWT: three base64url segments. The header segment always begins with
    // the base64 of `{"`, which is `eyJ`. Session cookies and platform bearers
    // in this crate are JWTs, and they carry no issuer prefix to match on.
    token.len() >= 20 && token.starts_with("eyJ") && token.matches('.').count() >= 2
}

/// Whether `key`, followed by `separator`, means the next token is its value.
///
/// The separator is what keeps prose out of this. "the token was rejected" has
/// a bare space between `token` and `was`, and redacting `was` would be
/// nonsense — so an assignment (`=` or `:`, in any of the shapes JSON, TOML,
/// a URL query and a log line write one) is required, with two exceptions that
/// are unambiguous without one:
///
/// * an auth scheme (`Bearer abc`), which is a credential by definition;
/// * a command-line flag (`--token abc`), which the leading `-` identifies.
fn key_directs_a_secret(key: &str, separator: &str) -> bool {
    // A separator that crosses a line is not an assignment; it is two
    // unrelated log lines that happen to be adjacent.
    if separator.contains('\n') || separator.contains('\r') {
        return false;
    }
    // Anything but the punctuation an assignment is written with — a word, a
    // comma, a bracket — means these two tokens are not a pair.
    if !separator
        .chars()
        .all(|c| matches!(c, ' ' | '\t' | ':' | '=' | '"' | '\'' | '>'))
    {
        return false;
    }
    let normalized = normalize_key(key);
    if SCHEME_KEYS.contains(&normalized.as_str()) {
        return true;
    }
    if !SECRET_KEYS.contains(&normalized.as_str()) {
        return false;
    }
    separator.contains('=') || separator.contains(':') || key.starts_with('-')
}

/// The token pass: split into words, then ask whole-word questions.
fn scrub_tokens(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut copied = 0usize;
    let mut changed = false;

    // The previous token's span, and where the run of separator characters
    // since it ended begins.
    let mut previous: Option<(usize, usize)> = None;
    let mut separator_start = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        if !is_token_byte(bytes[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && is_token_byte(bytes[index]) {
            index += 1;
        }
        let end = index;
        let token = &text[start..end];

        let directed = previous.is_some_and(|(previous_start, previous_end)| {
            key_directs_a_secret(
                &text[previous_start..previous_end],
                &text[separator_start..start],
            )
        }) && !AUTH_SCHEMES.contains(&normalize_key(token).as_str());

        if looks_like_a_secret(token) || directed {
            out.push_str(&text[copied..start]);
            out.push_str(REDACTED);
            copied = end;
            changed = true;
        }

        previous = Some((start, end));
        separator_start = end;
    }

    if changed {
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// The URL pass: `https://user:pass@host/path` loses its userinfo.
///
/// Separate from the token pass because `:` and `@` are exactly the characters
/// that pass uses as separators, so a URL's credential is invisible to it. This
/// is not hypothetical here — `analytics::boot` records an authenticated
/// collector proxy writing its key into container logs through `reqwest`'s own
/// `Display`, and a connector URL takes the same shape.
fn scrub_url_userinfo(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut copied = 0usize;
    let mut changed = false;
    let mut index = 0usize;

    while let Some(offset) = text[index..].find("://") {
        let authority_start = index + offset + 3;
        let authority_end = text[authority_start..]
            .find(|c: char| {
                matches!(
                    c,
                    '/' | '?' | '#' | '"' | '\'' | '<' | '>' | ')' | ',' | ';'
                ) || c.is_whitespace()
            })
            .map_or(text.len(), |n| authority_start + n);
        // `rfind`, not `find`: a password may itself contain an `@`, and the
        // last one is the delimiter the URL grammar means.
        if let Some(at) = text[authority_start..authority_end].rfind('@') {
            out.push_str(&text[copied..authority_start]);
            out.push_str(REDACTED);
            // The `@` stays, so the result still reads as a URL.
            copied = authority_start + at;
            changed = true;
        }
        index = authority_end;
        if index >= text.len() {
            break;
        }
    }

    if changed {
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    } else {
        Cow::Borrowed(text)
    }
}

/// A credential-shaped string for tests, assembled rather than written down.
///
/// [`looks_like_a_secret`] reads a token's prefix and its length and nothing
/// else, so the high-entropy body a real credential carries is filler as far
/// as these tests are concerned — what [`scrub`] is handed is identical either
/// way.
///
/// Written out as a literal, though, `ghp_AAAA…` is byte-for-byte what a
/// leaked token looks like to everything that reads this repository: secret
/// scanners flag the file on every push, and after a genuine incident somebody
/// grepping the tree has to rule each fixture out by hand before they can
/// believe the tree is clean. A scanner that is permanently red about a test
/// fixture is a scanner nobody reads.
///
/// So the prefix — the part under test, and the part that has to stay
/// readable — is written down, and only the body is assembled here. No
/// credential-shaped literal is committed and no coverage is lost.
#[cfg(test)]
pub(crate) fn credential_shaped(prefix: &str, body_len: usize) -> String {
    format!("{prefix}{}", "A".repeat(body_len))
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every one of these is a shape a credential has actually been found in,
    /// in this tree or the vendored one.
    #[test]
    fn a_labelled_credential_loses_its_value() {
        for (input, expected) in [
            ("api_key=hunter2", "api_key=[redacted]"),
            ("api-key: hunter2", "api-key: [redacted]"),
            ("apiKey=hunter2", "apiKey=[redacted]"),
            (r#""token":"hunter2""#, r#""token":"[redacted]""#),
            ("password = hunter2", "password = [redacted]"),
            ("client_secret=hunter2", "client_secret=[redacted]"),
            ("--token hunter2", "--token [redacted]"),
            (
                "Authorization: Bearer hunter2",
                "Authorization: Bearer [redacted]",
            ),
            (
                "authorization: Basic aGk6dGhlcmU",
                "authorization: Basic [redacted]",
            ),
            (
                "config.token = hunter2 and settings.api_key = hunter3",
                "config.token = [redacted] and settings.api_key = [redacted]",
            ),
        ] {
            assert_eq!(scrub(input), expected, "input: {input}");
        }
    }

    #[test]
    fn an_auth_scheme_is_never_mistaken_for_the_credential() {
        // The failure this guards is worse than no scrubbing at all: redacting
        // `Bearer` leaves the credential standing in a message that now looks
        // as though it was cleaned.
        let scrubbed = scrub("Authorization: Bearer hunter2");
        assert!(scrubbed.contains("Bearer"), "{scrubbed}");
        assert!(!scrubbed.contains("hunter2"), "{scrubbed}");
    }

    #[test]
    fn a_self_identifying_credential_needs_no_label() {
        // One per issuer prefix in `SECRET_PREFIXES` that a scanner recognises,
        // plus the JWT arm. Assembled by `credential_shaped`, which explains
        // why they are not written out.
        for input in [
            credential_shaped("sk-ant-api03-", 28),
            credential_shaped("sk-proj-", 20),
            credential_shaped("sk_live_", 20),
            credential_shaped("ghp_", 36),
            format!("{}_{}", credential_shaped("github_pat_", 20), "B".repeat(8)),
            credential_shaped("glpat-", 20),
            format!(
                "xoxb-{}-{}-{}",
                "1".repeat(10),
                "2".repeat(10),
                "A".repeat(12)
            ),
            credential_shaped("AKIA", 16),
            credential_shaped("th_live_", 20),
            credential_shaped("npm_", 28),
            format!("{}.{}", credential_shaped("SG.", 22), "B".repeat(12)),
            // A JWT: `eyJ` and two dots are the whole of the rule.
            format!(
                "{}.{}.{}",
                credential_shaped("eyJ", 17),
                "B".repeat(16),
                "c2ln"
            ),
        ] {
            let sentence = format!("the provider said: {input} was rejected");
            let scrubbed = scrub(&sentence);
            assert!(!scrubbed.contains(&input), "{input} survived: {scrubbed}");
            assert!(scrubbed.contains(REDACTED), "{scrubbed}");
            // Only the credential goes — the sentence around it is the
            // diagnostic and has to survive.
            assert!(scrubbed.contains("was rejected"), "{scrubbed}");
        }
    }

    #[test]
    fn a_credential_with_a_separator_leaves_no_fragment() {
        // The vendored runtime's `sk-[A-Za-z0-9]{20,}` left `[REDACTED]_uv`
        // behind on exactly this shape, because the character class stopped at
        // the underscore. A whole token cannot half-match.
        let input = format!("key {}_uv-9 here", credential_shaped("sk-", 20));
        let scrubbed = scrub(&input);
        assert_eq!(scrubbed, "key [redacted] here");
    }

    #[test]
    fn the_word_boundary_false_positive_cannot_arise() {
        // The regexes this replaces matched all four of these. A tokenizer
        // cannot: none of them normalise to a key in `SECRET_KEYS`.
        for input in [
            "cancellation_token=abc123",
            "next_page_token=abc123",
            "csrf_token_name=session",
            "idempotency_key=abc123",
        ] {
            assert_eq!(scrub(input), input, "{input} must survive untouched");
        }
    }

    #[test]
    fn prose_is_left_alone() {
        for input in [
            "the token was rejected by the provider",
            "no credential is configured for this company",
            "password reset requested",
            "reading the api key from config.toml",
            "sk- is the prefix these keys use",
            // `token` is a scheme in `Authorization: token <pat>` and a common
            // English word everywhere else. The word wins; see `SCHEME_KEYS`.
            "the token expired and no credential was refreshed",
            "storage=mongodb companies=3 outcome=ok",
            "GET /api/v1/companies/acme/agents -> 500 in 42ms",
        ] {
            assert_eq!(scrub(input), input, "{input} must survive untouched");
        }
    }

    #[test]
    fn a_url_loses_its_userinfo() {
        assert_eq!(
            scrub("posting to https://user:hunter2@collector.internal/track failed"),
            "posting to https://[redacted]@collector.internal/track failed"
        );
        // A password containing an `@` still ends at the last one.
        assert_eq!(
            scrub("mongodb://admin:p@ss@db.internal:27017/oc"),
            "mongodb://[redacted]@db.internal:27017/oc"
        );
        // A URL with no userinfo is untouched, including the port colon that
        // looks like an assignment.
        assert_eq!(
            scrub("connecting to https://db.internal:27017/oc"),
            "connecting to https://db.internal:27017/oc"
        );
    }

    #[test]
    fn a_line_break_is_not_an_assignment() {
        // Two adjacent log lines are not a key and its value.
        let input = "refreshing the token\nGET /healthz -> 200";
        assert_eq!(scrub(input), input);
    }

    #[test]
    fn several_credentials_in_one_string_all_go() {
        let input = format!(
            "POST https://key:secret@ingest.example/1 \
             api_key=hunter2 authorization: Bearer {}",
            credential_shaped("ghp_", 36)
        );
        let scrubbed = scrub(&input);
        for leaked in ["secret", "hunter2", "ghp_AAAA"] {
            assert!(!scrubbed.contains(leaked), "{leaked} survived: {scrubbed}");
        }
        assert_eq!(scrubbed.matches(REDACTED).count(), 3, "{scrubbed}");
    }

    #[test]
    fn a_clean_string_is_borrowed_rather_than_copied() {
        // This runs on every string of every event; the common case must not
        // allocate.
        assert!(matches!(
            scrub("company acme finished a cycle in 42ms"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(scrub("api_key=hunter2"), Cow::Owned(_)));
    }

    #[test]
    fn non_ascii_text_is_not_split_mid_character() {
        // The scanner indexes by byte; every index it slices on has to be a
        // char boundary or this panics rather than merely being wrong.
        let input = "l'agent a échoué — token: hunter2 — 完了 🙂";
        let scrubbed = scrub(input);
        assert!(!scrubbed.contains("hunter2"), "{scrubbed}");
        assert!(scrubbed.contains("échoué"), "{scrubbed}");
        assert!(scrubbed.contains("完了 🙂"), "{scrubbed}");
    }

    #[test]
    fn scrubbing_is_idempotent() {
        // A string can pass through more than one seam — a message that was
        // scrubbed at the call site and again in `before_send` — and the second
        // pass must not eat the marker or the text around it.
        let once = scrub("api_key=hunter2 and https://u:p@h/1").into_owned();
        assert_eq!(scrub(&once), once);
    }

    #[test]
    fn an_empty_string_is_handled() {
        assert_eq!(scrub(""), "");
        assert_eq!(scrub("://"), "://");
        assert_eq!(scrub("@"), "@");
    }
}
