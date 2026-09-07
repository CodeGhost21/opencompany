//! x402 payment challenges and Ed25519-signed authorizations.
//!
//! When a counterparty gates a skill behind payment it answers `402` with a
//! challenge naming the `amount`, `recipient`, `asset`, and `network`. The payer
//! signs an **authorization** over a canonical payload with the same Ed25519
//! identity key it uses for SIWX, then posts it to the settlement endpoints.
//! This module only *builds and verifies* authorizations — no on-chain
//! submission happens here (that is a documented SDK gap).
//!
//! ## Canonical byte layout (golden, versioned)
//!
//! ```text
//! tiny.place-x402-v1\n
//! <agentId>\n
//! <amount>\n
//! <recipient>\n
//! <asset>\n
//! <network>\n
//! <nonce>\n
//! <timestamp>
//! ```
//!
//! Isolated in [`canonical_bytes`] so it is a one-function change to reconcile
//! with the real tiny.place server when reachable.
//!
//! ## Single use is enforced, not merely documented
//!
//! The signature covers the whole payload including the nonce, so a payer
//! cannot re-point one authorization at different work — but nothing about a
//! signature stops the *same* authorization being presented again. [`verify`]
//! therefore takes the spent-nonce set and the current time, and refuses both a
//! nonce it has already seen and an authorization older than
//! [`MAX_AGE_SECS`]. Neither is optional, because a verified-but-unspent
//! authorization is a bearer token: one signature buying unlimited work.
//!
//! Bounding acceptance by age is what makes forgetting a nonce safe. The spent
//! set prunes on the same constant, so a nonce is dropped only once the
//! authorization carrying it would be refused on age anyway, and there is no
//! window in which a replay outlives the memory of it.
//!
//! ## The nonce comes from the OS CSPRNG
//!
//! The nonce is signed into the payload above, so a counterparty's replay check
//! is only as good as the value's unpredictability and uniqueness. It is
//! therefore minted by [`mint_nonce`] from 256 bits of OS randomness through
//! the same
//! [`TokenSource`](crate::server::users::token::TokenSource) seam the user-auth
//! secrets use — **not** from
//! [`generate_id`](crate::ports::generate_id), whose epoch-millis-plus-counter
//! shape is guessable from a prior value and repeats across processes that
//! start in the same millisecond.

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::economy::signer::{LocalSigner, verify_b58};
use crate::economy::siwx::{NonceCache, SKEW_SECS};
use crate::error::OpenCompanyError;
use crate::server::platform_auth::b64url_encode;
use crate::server::users::token::{OsTokens, TokenSource};

/// The domain-separation tag pinning the x402 canonical layout version.
pub const X402_DOMAIN: &str = "tiny.place-x402-v1";

/// How long an authorization stays acceptable, and therefore how long its nonce
/// is remembered as spent. Ten minutes.
///
/// Twice the SIWX clock-skew tolerance. An authorization only ever arrives
/// inside a SIWX-signed request, and that request is already refused unless its
/// own timestamp is within [`SKEW_SECS`] of the verifier's clock, so this
/// covers a payer at the far edge of tolerated clock offset plus a full
/// challenge → authorize → resend round trip — a round trip that in practice
/// takes under a second. Anything older is a request the transport layer would
/// have turned away, so accepting it buys the payer nothing and costs the spent
/// set unbounded memory.
pub const MAX_AGE_SECS: i64 = 2 * SKEW_SECS;

/// A payment challenge parsed from a counterparty's `402` response body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct X402Challenge {
    /// The amount due, as a decimal string (e.g. `"25.00"`).
    pub amount: String,
    /// The recipient address to pay.
    pub recipient: String,
    /// The settlement asset (e.g. `"USDC"`).
    pub asset: String,
    /// The settlement network (e.g. `"solana"`).
    pub network: String,
}

impl X402Challenge {
    /// Parses a challenge from a `402` JSON body.
    ///
    /// Accepts either a flat object (`{amount, recipient, asset, network}`) or
    /// the x402 `{ "accepts": [ { … } ] }` envelope, and tolerates the common
    /// field aliases `maxAmountRequired`/`payTo`.
    pub fn from_body(v: &serde_json::Value) -> Result<Self> {
        let obj = v.get("accepts").and_then(|a| a.get(0)).unwrap_or(v);

        let amount = string_field(obj, &["amount", "maxAmountRequired"]).ok_or_else(|| {
            OpenCompanyError::InvalidRequest("x402 challenge is missing `amount`".into())
        })?;
        let recipient = string_field(obj, &["recipient", "payTo"]).ok_or_else(|| {
            OpenCompanyError::InvalidRequest("x402 challenge is missing `recipient`".into())
        })?;
        let asset = string_field(obj, &["asset"]).unwrap_or_else(|| "USDC".to_string());
        let network = string_field(obj, &["network"]).unwrap_or_else(|| "solana".to_string());

        Ok(Self {
            amount,
            recipient,
            asset,
            network,
        })
    }
}

/// A signed x402 payment authorization, ready to POST to `/payments/verify`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct X402Authorization {
    /// The payer's base58 `agentId`.
    #[serde(rename = "agentId")]
    pub agent_id: String,
    /// The amount authorized. May exceed the challenge amount for an `upto`
    /// delegated-signer grant.
    pub amount: String,
    /// The recipient address.
    pub recipient: String,
    /// The settlement asset.
    pub asset: String,
    /// The settlement network.
    pub network: String,
    /// A single-use nonce: 256 bits of OS randomness, base64url, 43 chars.
    ///
    /// Opaque to every reader — nothing parses or matches its shape — so the
    /// counterparty only needs it to be unpredictable and unique. [`verify`]
    /// spends it against a [`NonceCache`], which is what makes "single-use"
    /// true. See [`mint_nonce`].
    pub nonce: String,
    /// The authorization timestamp, epoch seconds.
    pub timestamp: i64,
    /// The base58 Ed25519 signature over [`canonical_bytes`].
    #[serde(rename = "signature")]
    pub signature_b58: String,
}

/// How many random bytes back an authorization nonce. 32 bytes = 256 bits,
/// matching the user-auth secrets, so two mints colliding is not a scenario.
const NONCE_BYTES: usize = 32;

/// Mints an authorization nonce: 256 bits from `src`, base64url, 43 chars.
///
/// A pure function of the source bytes — no clock, no counter, no process
/// state — which is the property that makes one nonce say nothing about the
/// next, and makes two processes minting in the same millisecond differ.
pub fn mint_nonce(src: &dyn TokenSource) -> String {
    let mut bytes = [0u8; NONCE_BYTES];
    src.fill(&mut bytes);
    b64url_encode(&bytes)
}

/// Builds the canonical bytes an x402 authorization signs. See module docs.
pub fn canonical_bytes(
    agent_id: &str,
    amount: &str,
    recipient: &str,
    asset: &str,
    network: &str,
    nonce: &str,
    timestamp: i64,
) -> Vec<u8> {
    format!(
        "{X402_DOMAIN}\n{agent_id}\n{amount}\n{recipient}\n{asset}\n{network}\n{nonce}\n{timestamp}"
    )
    .into_bytes()
}

/// Signs an authorization paying exactly the challenged amount.
pub fn authorize(signer: &LocalSigner, ch: &X402Challenge, now: i64) -> X402Authorization {
    authorize_amount(signer, ch, ch.amount.clone(), now)
}

/// Signs a delegated-signer `upto` authorization capped at `cap`, letting the
/// counterparty settle any amount up to the cap.
pub fn authorize_upto(
    signer: &LocalSigner,
    ch: &X402Challenge,
    cap: &str,
    now: i64,
) -> X402Authorization {
    authorize_amount(signer, ch, cap.to_string(), now)
}

fn authorize_amount(
    signer: &LocalSigner,
    ch: &X402Challenge,
    amount: String,
    now: i64,
) -> X402Authorization {
    let agent_id = signer.agent_id();
    let nonce = mint_nonce(&OsTokens);
    let msg = canonical_bytes(
        &agent_id,
        &amount,
        &ch.recipient,
        &ch.asset,
        &ch.network,
        &nonce,
        now,
    );
    let signature_b58 = signer.sign_b58(&msg);
    X402Authorization {
        agent_id,
        amount,
        recipient: ch.recipient.clone(),
        asset: ch.asset.clone(),
        network: ch.network.clone(),
        nonce,
        timestamp: now,
        signature_b58,
    }
}

/// Verifies an authorization and spends its nonce, so one signature buys one
/// task.
///
/// Enforces, in order: the signature against the declared `agentId`, freshness
/// within [`MAX_AGE_SECS`], and single use of the nonce against `spent`.
///
/// `spent` and `now` are parameters rather than something a caller may choose
/// to consult. A signature proves who authorized the payment, not that the
/// payment has not already been collected; a caller that could verify without
/// spending would be treating the authorization as a bearer token, which is
/// precisely the bug this signature shape exists to prevent.
///
/// The signature is checked before the nonce is spent, so an unverifiable
/// authorization cannot burn a nonce — otherwise anyone who observed a payer's
/// nonce could spend it on their behalf with a forged signature.
pub fn verify(auth: &X402Authorization, spent: &NonceCache, now: i64) -> Result<()> {
    let msg = canonical_bytes(
        &auth.agent_id,
        &auth.amount,
        &auth.recipient,
        &auth.asset,
        &auth.network,
        &auth.nonce,
        auth.timestamp,
    );
    verify_b58(&auth.agent_id, &msg, &auth.signature_b58)?;

    if (now - auth.timestamp).abs() > MAX_AGE_SECS {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "x402 authorization timestamp is outside the ±{MAX_AGE_SECS}s window"
        )));
    }

    if !spent.check_and_insert(&auth.nonce, now)? {
        return Err(OpenCompanyError::InvalidRequest(
            "x402 authorization nonce has already been spent (replay)".into(),
        ));
    }

    Ok(())
}

fn string_field(obj: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = obj.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use super::*;

    fn sample_challenge() -> X402Challenge {
        X402Challenge {
            amount: "25.00".into(),
            recipient: "RecipientAddr".into(),
            asset: "USDC".into(),
            network: "solana".into(),
        }
    }

    #[test]
    fn parses_flat_challenge_body() {
        let body = serde_json::json!({
            "amount": "25.00",
            "recipient": "RecipientAddr",
            "asset": "USDC",
            "network": "solana"
        });
        assert_eq!(X402Challenge::from_body(&body).unwrap(), sample_challenge());
    }

    #[test]
    fn parses_accepts_envelope_with_aliases() {
        let body = serde_json::json!({
            "accepts": [ { "maxAmountRequired": "10.00", "payTo": "Somebody" } ]
        });
        let ch = X402Challenge::from_body(&body).unwrap();
        assert_eq!(ch.amount, "10.00");
        assert_eq!(ch.recipient, "Somebody");
        assert_eq!(ch.asset, "USDC");
        assert_eq!(ch.network, "solana");
    }

    #[test]
    fn missing_amount_is_an_error() {
        let body = serde_json::json!({ "recipient": "x" });
        assert!(X402Challenge::from_body(&body).is_err());
    }

    #[test]
    fn authorize_signs_a_verifiable_payload() {
        let signer = LocalSigner::generate();
        let ch = sample_challenge();
        let auth = authorize(&signer, &ch, 1_700_000_000);

        assert_eq!(auth.agent_id, signer.agent_id());
        assert_eq!(auth.amount, "25.00");
        assert_eq!(auth.recipient, "RecipientAddr");
        verify(&auth, &NonceCache::new(), 1_700_000_000)
            .expect("authorization verifies against its own key");
    }

    #[test]
    fn authorize_upto_carries_the_cap() {
        let signer = LocalSigner::generate();
        let ch = sample_challenge();
        let auth = authorize_upto(&signer, &ch, "100.00", 1_700_000_000);
        assert_eq!(auth.amount, "100.00");
        verify(&auth, &NonceCache::new(), 1_700_000_000).expect("upto authorization verifies");
    }

    #[test]
    fn tampered_authorization_fails_verification() {
        let signer = LocalSigner::generate();
        let ch = sample_challenge();
        let mut auth = authorize(&signer, &ch, 1_700_000_000);
        auth.amount = "0.01".into();
        assert!(
            verify(&auth, &NonceCache::new(), 1_700_000_000).is_err(),
            "changed amount must break the signature"
        );
    }

    /// A deterministic source, for asserting minting is a pure function of its
    /// bytes. Never use anything like this outside tests.
    struct FixedTokens(u8);

    impl TokenSource for FixedTokens {
        fn fill(&self, out: &mut [u8]) {
            out.fill(self.0);
        }
    }

    #[test]
    fn nonce_is_a_pure_function_of_the_source_bytes() {
        // The property, not the encoding: the nonce is the CSPRNG's output and
        // nothing else. If a clock or a counter were mixed in, two mints from
        // the same bytes would differ.
        assert_eq!(
            mint_nonce(&FixedTokens(0xAB)),
            mint_nonce(&FixedTokens(0xAB))
        );
        assert_ne!(
            mint_nonce(&FixedTokens(0xAB)),
            mint_nonce(&FixedTokens(0xCD))
        );
    }

    #[test]
    fn nonces_minted_in_the_same_millisecond_differ() {
        let signer = LocalSigner::generate();
        let ch = sample_challenge();
        let mut seen = HashSet::new();
        // A tight loop lands many mints inside one millisecond, which is
        // exactly where a clock-prefixed id has only its counter left.
        for _ in 0..1000 {
            let auth = authorize(&signer, &ch, 1_700_000_000);
            assert!(seen.insert(auth.nonce), "a nonce repeated");
        }
    }

    #[test]
    fn nonces_carry_no_monotonic_counter() {
        let signer = LocalSigner::generate();
        let ch = sample_challenge();
        let minted: Vec<String> = (0..64)
            .map(|_| authorize(&signer, &ch, 1_700_000_000).nonce)
            .collect();

        // An id built from a timestamp plus an incrementing counter sorts in
        // mint order. Random values do not: 64 draws land sorted with
        // probability 1/64!, so this failing means order leaked back in.
        assert!(
            minted.windows(2).any(|w| w[0] > w[1]),
            "nonces arrived in ascending order, which implies a counter"
        );

        // And no shared structure: a common prefix is what a clock component
        // would leave behind across mints in the same millisecond.
        let first = minted[0].as_bytes();
        assert!(
            minted[1..]
                .iter()
                .any(|n| n.as_bytes().first() != first.first()),
            "every nonce shared a leading byte, which implies a fixed prefix"
        );
    }

    #[test]
    fn nonce_is_url_safe_and_full_width() {
        let auth = authorize(&LocalSigner::generate(), &sample_challenge(), 1_700_000_000);
        // 32 bytes unpadded base64url.
        assert_eq!(auth.nonce.len(), 43, "unexpected nonce: {}", auth.nonce);
        assert!(
            auth.nonce
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "nonce is not base64url: {}",
            auth.nonce
        );
    }

    #[test]
    fn a_changed_nonce_breaks_the_signature() {
        // The nonce is inside the signed payload, so replaying an
        // authorization under a fresh nonce is not something a payer can do
        // without the key.
        let signer = LocalSigner::generate();
        let mut auth = authorize(&signer, &sample_challenge(), 1_700_000_000);
        auth.nonce = mint_nonce(&OsTokens);
        assert!(
            verify(&auth, &NonceCache::new(), 1_700_000_000).is_err(),
            "changed nonce must break the signature"
        );
    }

    #[test]
    fn a_replayed_authorization_is_refused() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let auth = authorize(&signer, &sample_challenge(), now);
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);

        verify(&auth, &spent, now).expect("first presentation is the payment");
        assert!(
            verify(&auth, &spent, now).is_err(),
            "one signed authorization must not buy a second task"
        );
    }

    #[test]
    fn a_second_authorization_is_still_admitted() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);

        for _ in 0..3 {
            let auth = authorize(&signer, &sample_challenge(), now);
            verify(&auth, &spent, now).expect("each fresh nonce pays its own way");
        }
    }

    #[test]
    fn a_stale_authorization_is_refused() {
        let signer = LocalSigner::generate();
        let signed_at = 1_700_000_000;
        let auth = authorize(&signer, &sample_challenge(), signed_at);
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);

        assert!(
            verify(&auth, &spent, signed_at + MAX_AGE_SECS + 1).is_err(),
            "an authorization older than the spent set's memory must not verify"
        );
    }

    #[test]
    fn a_stale_authorization_is_refused_before_its_nonce_is_forgotten() {
        // The two windows are the same constant, so at the moment the spent set
        // would prune a nonce the authorization carrying it is already too old.
        // This is the property that makes a bounded store sufficient.
        let signer = LocalSigner::generate();
        let signed_at = 1_700_000_000;
        let auth = authorize(&signer, &sample_challenge(), signed_at);
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);

        verify(&auth, &spent, signed_at).expect("fresh");
        // Late enough for the prune to drop the nonce — and late enough for the
        // age check to refuse the authorization anyway.
        assert!(verify(&auth, &spent, signed_at + MAX_AGE_SECS * 2).is_err());
    }

    #[test]
    fn an_unusable_spent_set_refuses_the_payment() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let auth = authorize(&signer, &sample_challenge(), now);
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);
        spent.poison_for_tests();

        assert!(
            verify(&auth, &spent, now).is_err(),
            "a spent set that cannot answer must refuse, not admit"
        );
    }

    #[test]
    fn an_unverifiable_authorization_cannot_burn_a_nonce() {
        let signer = LocalSigner::generate();
        let now = 1_700_000_000;
        let auth = authorize(&signer, &sample_challenge(), now);
        let spent = NonceCache::with_ttl(MAX_AGE_SECS);

        let mut forged = auth.clone();
        forged.amount = "0.01".into();
        assert!(verify(&forged, &spent, now).is_err(), "forgery is refused");

        verify(&auth, &spent, now).expect("the payer's own nonce is still unspent");
    }

    #[test]
    fn authorization_json_round_trips() {
        let signer = LocalSigner::generate();
        let auth = authorize(&signer, &sample_challenge(), 1_700_000_000);
        let json = serde_json::to_string(&auth).expect("serialize");
        assert!(json.contains("\"agentId\""));
        assert!(json.contains("\"signature\""));
        let back: X402Authorization = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, auth);
    }
}
