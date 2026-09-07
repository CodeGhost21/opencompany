//! SIWX (Sign-In With X) per-action authorization over Ed25519.
//!
//! Every authenticated tiny.place request carries an
//! `Authorization: tiny.place <agentId>:<signature>:<timestamp>` header. The
//! signature covers a **canonical payload** binding the request method, path,
//! timestamp, and a hash of the body, so a captured header cannot be replayed
//! against a different request.
//!
//! Freshness and replay protection:
//! - the timestamp must be within [`SKEW_SECS`] of the verifier's clock;
//! - each accepted signature is recorded in a [`NonceCache`]; a second
//!   presentation of the same signature within the skew window is rejected.
//!   The Ed25519 signature is itself the anti-replay nonce: it is unique per
//!   `(method, path, timestamp, body_hash)` tuple.
//!
//! ## Canonical byte layout (golden, versioned)
//!
//! ```text
//! tiny.place-siwx-v1\n
//! <method>\n
//! <path>\n
//! <timestamp>\n
//! <body_hash>
//! ```
//!
//! Isolated in [`canonical_bytes`] so reconciling with the real tiny.place
//! server, when reachable, is a one-function change. `body_hash` is supplied by
//! the caller (a hex digest of the request body, or an empty string for bodiless
//! requests); this module does not hash bodies itself.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Mutex;

use crate::Result;
use crate::economy::signer::{LocalSigner, verify_b58};
use crate::error::OpenCompanyError;

/// The domain-separation tag pinning the canonical layout version.
pub const SIWX_DOMAIN: &str = "tiny.place-siwx-v1";

/// The `tiny.place ` scheme prefix on the `Authorization` header.
pub const SIWX_SCHEME: &str = "tiny.place";

/// Maximum tolerated clock skew, in seconds, between signer and verifier.
pub const SKEW_SECS: i64 = 300;

/// The parts of a parsed or freshly built SIWX authorization header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiwxHeader {
    /// The signer's base58 `agentId`.
    pub agent_id: String,
    /// The base58-encoded Ed25519 signature.
    pub signature_b58: String,
    /// The signing timestamp, epoch seconds.
    pub timestamp: i64,
}

/// The fields bound by a SIWX signature.
#[derive(Clone, Debug)]
pub struct SiwxPayload<'a> {
    /// HTTP method, uppercased (e.g. `POST`).
    pub method: &'a str,
    /// Request path (e.g. `/a2a/acme`).
    pub path: &'a str,
    /// Signing timestamp, epoch seconds.
    pub timestamp: i64,
    /// Hex digest of the request body, or empty for bodiless requests.
    pub body_hash: &'a str,
}

/// Builds the canonical bytes a SIWX signature covers. See the module docs for
/// the exact layout.
pub fn canonical_bytes(p: &SiwxPayload) -> Vec<u8> {
    format!(
        "{SIWX_DOMAIN}\n{}\n{}\n{}\n{}",
        p.method, p.path, p.timestamp, p.body_hash
    )
    .into_bytes()
}

/// Signs `payload` with `signer`, producing the header parts.
pub fn build_header(signer: &LocalSigner, payload: &SiwxPayload) -> SiwxHeader {
    let signature_b58 = signer.sign_b58(&canonical_bytes(payload));
    SiwxHeader {
        agent_id: signer.agent_id(),
        signature_b58,
        timestamp: payload.timestamp,
    }
}

/// Renders a [`SiwxHeader`] as an `Authorization` header value:
/// `tiny.place <agentId>:<signature>:<timestamp>`.
pub fn header_value(h: &SiwxHeader) -> String {
    format!(
        "{SIWX_SCHEME} {}:{}:{}",
        h.agent_id, h.signature_b58, h.timestamp
    )
}

/// Parses an `Authorization` header value into its parts.
pub fn parse_header(header: &str) -> Result<SiwxHeader> {
    let rest = header
        .strip_prefix(SIWX_SCHEME)
        .map(str::trim_start)
        .ok_or_else(|| {
            OpenCompanyError::InvalidRequest("authorization is not a tiny.place scheme".into())
        })?;

    let mut parts = rest.splitn(3, ':');
    let agent_id = parts.next().unwrap_or_default();
    let signature_b58 = parts.next().unwrap_or_default();
    let ts_raw = parts.next().unwrap_or_default();

    if agent_id.is_empty() || signature_b58.is_empty() || ts_raw.is_empty() {
        return Err(OpenCompanyError::InvalidRequest(
            "authorization must be <agentId>:<signature>:<timestamp>".into(),
        ));
    }
    let timestamp = ts_raw.parse::<i64>().map_err(|_| {
        OpenCompanyError::InvalidRequest("authorization timestamp is not an integer".into())
    })?;

    Ok(SiwxHeader {
        agent_id: agent_id.to_string(),
        signature_b58: signature_b58.to_string(),
        timestamp,
    })
}

/// Verifies an inbound SIWX header against the reconstructed canonical payload.
///
/// Enforces, in order: header shape, clock skew ≤ [`SKEW_SECS`], Ed25519
/// signature validity, and single-use of the signature via `seen`. On success
/// returns the authenticated `agentId`.
pub fn verify(
    header: &str,
    method: &str,
    path: &str,
    body_hash: &str,
    now: i64,
    seen: &NonceCache,
) -> Result<String> {
    let parsed = parse_header(header)?;

    if (now - parsed.timestamp).abs() > SKEW_SECS {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "authorization timestamp is outside the ±{SKEW_SECS}s window"
        )));
    }

    let payload = SiwxPayload {
        method,
        path,
        timestamp: parsed.timestamp,
        body_hash,
    };
    verify_b58(
        &parsed.agent_id,
        &canonical_bytes(&payload),
        &parsed.signature_b58,
    )?;

    if !seen.check_and_insert(&parsed.signature_b58, now, parsed.timestamp)? {
        return Err(OpenCompanyError::InvalidRequest(
            "authorization signature has already been used (replay)".into(),
        ));
    }

    Ok(parsed.agent_id)
}

/// A process-local record of spent single-use values, for replay protection.
///
/// Entries older than the cache's TTL are pruned on insert, bounding memory to
/// the window in which a value can still be accepted. In-memory only;
/// cross-restart persistence is a documented follow-up.
///
/// Two callers keep separate instances with separate windows: SIWX signatures
/// live for [`SKEW_SECS`], x402 authorization nonces for
/// [`x402::MAX_AGE_SECS`](crate::economy::x402::MAX_AGE_SECS). Sharing one set
/// across both would let either keyspace evict the other's record.
#[derive(Debug)]
pub struct NonceCache {
    seen: Mutex<HashMap<String, i64>>,
    ttl_secs: i64,
}

impl Default for NonceCache {
    fn default() -> Self {
        Self::with_ttl(SKEW_SECS)
    }
}

impl NonceCache {
    /// Creates an empty cache remembering a value for [`SKEW_SECS`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty cache remembering a value for `ttl_secs`.
    ///
    /// The TTL must be at least as long as the window in which the caller will
    /// still accept the value it guards, or a replay outlives the memory of it.
    pub fn with_ttl(ttl_secs: i64) -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
            ttl_secs,
        }
    }

    /// Records `key` as spent, pruning stale entries first.
    ///
    /// `now` is the verifier's current clock, used only to decide which
    /// existing entries have aged out. `record_ts` is the timestamp the entry
    /// is remembered under — the claimed timestamp of the thing being spent
    /// (an authorization's `timestamp`, a SIWX header's signing time), not
    /// `now`. Storing `record_ts` rather than `now` matters because a caller's
    /// own freshness check already tolerates skew between the two: if the
    /// entry were stamped with `now`, a replay landing just past the caller's
    /// freshness window could still land inside the prune window, since the
    /// two windows would be measured from different origins. Stamping with
    /// `record_ts` instead ties both checks to the same origin, so the entry
    /// is forgotten at the same moment the freshness check would refuse it
    /// again — never before, never after.
    ///
    /// `Ok(true)` if it was previously unseen (accept), `Ok(false)` on a replay
    /// (reject), `Err` if the cache cannot answer (reject). The guarded section
    /// touches only a `HashMap` and an `i64`, so a poisoned lock means a panic
    /// unwound through it and the record of what has been spent can no longer
    /// be trusted — the caller must refuse rather than recover, because a cache
    /// that cannot answer admits every replay.
    pub fn check_and_insert(&self, key: &str, now: i64, record_ts: i64) -> Result<bool> {
        let mut guard = self
            .seen
            .lock()
            .map_err(|_| OpenCompanyError::Store("replay-protection cache is unusable".into()))?;
        guard.retain(|_, ts| (now - *ts).abs() <= self.ttl_secs);
        match guard.entry(key.to_string()) {
            Entry::Occupied(_) => Ok(false),
            Entry::Vacant(slot) => {
                slot.insert(record_ts);
                Ok(true)
            }
        }
    }

    /// Poisons the lock so a caller's fail-closed path can be exercised. The
    /// panic it raises is caught here and is expected in test output.
    #[cfg(test)]
    pub(crate) fn poison_for_tests(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.seen.lock().expect("cache is not already poisoned");
            panic!("deliberately poisoning the replay-protection cache");
        }));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn payload<'a>(now: i64) -> SiwxPayload<'a> {
        SiwxPayload {
            method: "POST",
            path: "/a2a/acme",
            timestamp: now,
            body_hash: "abc123",
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(now)));
        let cache = NonceCache::new();

        let id = verify(&header, "POST", "/a2a/acme", "abc123", now, &cache).expect("verifies");
        assert_eq!(id, signer.agent_id());
    }

    #[test]
    fn tampered_body_hash_fails() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(now)));
        let cache = NonceCache::new();

        let err = verify(&header, "POST", "/a2a/acme", "DIFFERENT", now, &cache);
        assert!(
            err.is_err(),
            "signature must not verify over a changed body"
        );
    }

    #[test]
    fn skewed_timestamp_is_rejected() {
        let signer = LocalSigner::generate();
        let signed_at = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(signed_at)));
        let cache = NonceCache::new();

        let now = signed_at + SKEW_SECS + 100;
        let err = verify(&header, "POST", "/a2a/acme", "abc123", now, &cache);
        assert!(err.is_err(), "stale timestamp must be rejected");
    }

    #[test]
    fn replayed_signature_is_rejected() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(now)));
        let cache = NonceCache::new();

        verify(&header, "POST", "/a2a/acme", "abc123", now, &cache).expect("first accepted");
        let err = verify(&header, "POST", "/a2a/acme", "abc123", now, &cache);
        assert!(err.is_err(), "second presentation must be rejected");
    }

    #[test]
    fn wrong_path_fails_because_signature_binds_it() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(now)));
        let cache = NonceCache::new();

        let err = verify(&header, "POST", "/a2a/other", "abc123", now, &cache);
        assert!(err.is_err());
    }

    #[test]
    fn malformed_headers_are_rejected() {
        assert!(parse_header("Bearer xyz").is_err());
        assert!(parse_header("tiny.place only-one-part").is_err());
        assert!(parse_header("tiny.place a:b:notanint").is_err());
    }

    #[test]
    fn nonce_cache_prunes_stale_entries() {
        let cache = NonceCache::new();
        assert!(cache.check_and_insert("sig-a", 1_000, 1_000).unwrap());
        // Far in the future: the stale entry is pruned, so re-inserting is fine.
        assert!(
            cache
                .check_and_insert("sig-a", 1_000 + SKEW_SECS * 4, 1_000 + SKEW_SECS * 4)
                .unwrap()
        );
    }

    #[test]
    fn nonce_cache_honours_a_custom_ttl() {
        let cache = NonceCache::with_ttl(SKEW_SECS * 4);
        assert!(cache.check_and_insert("sig-a", 1_000, 1_000).unwrap());
        // Still inside the wider window, so still remembered as spent.
        assert!(
            !cache
                .check_and_insert("sig-a", 1_000 + SKEW_SECS * 3, 1_000 + SKEW_SECS * 3)
                .unwrap()
        );
    }

    #[test]
    fn nonce_cache_prunes_by_record_timestamp_not_insertion_time() {
        // A signature stamped at the far edge of tolerated skew (future-dated,
        // but still within the freshness window when first checked) must stay
        // remembered for as long as *its own claimed timestamp* would still
        // pass a freshness check — not merely for `ttl_secs` past the moment
        // it happened to be verified. Keying the stored entry off the
        // verifier's clock instead of the claimed timestamp would prune it
        // early, reopening the slot for a replay of the same signature while
        // the claimed timestamp is still "fresh" by the caller's own check.
        let cache = NonceCache::with_ttl(SKEW_SECS);
        let claimed_ts = 1_000 + SKEW_SECS; // maximally future-dated, still fresh at now=1_000
        assert!(cache.check_and_insert("sig-a", 1_000, claimed_ts).unwrap());
        // Past the point at which keying off insertion time (1_000) would have
        // pruned the entry, but still within `ttl_secs` of `claimed_ts`.
        let replay_now = 1_000 + SKEW_SECS + 1;
        assert!(
            !cache
                .check_and_insert("sig-a", replay_now, claimed_ts)
                .unwrap(),
            "entry must still be remembered because its claimed timestamp is \
             still within the freshness window, even though insertion time is not"
        );
    }

    #[test]
    fn an_unusable_cache_refuses_rather_than_admits() {
        let cache = NonceCache::new();
        cache.poison_for_tests();
        assert!(
            cache.check_and_insert("sig-a", 1_000, 1_000).is_err(),
            "a cache that cannot answer must not report a value as fresh"
        );
    }

    #[test]
    fn an_unusable_cache_refuses_the_siwx_signature() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let header = header_value(&build_header(&signer, &payload(now)));
        let cache = NonceCache::new();
        cache.poison_for_tests();

        assert!(
            verify(&header, "POST", "/a2a/acme", "abc123", now, &cache).is_err(),
            "a correctly signed header must not pass when replay protection is down"
        );
    }
}
