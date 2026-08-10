//! Proving a runner is who it says, and that somebody authorised it.
//!
//! Two separate questions, and conflating them is the mistake this module
//! exists to avoid:
//!
//! 1. **Is this the keypair it claims?** Answered by a signature over a
//!    server-issued challenge. Stops replay and impersonation.
//! 2. **Is it allowed to act for an owner?** Answered by an *owner
//!    attestation*: a signature by the **owner's** key over a statement naming
//!    the runner. Stops a valid-but-unauthorised runner attaching itself.
//!
//! A runner that answers only the first is authentic and entitled to nothing.
//!
//! ## Ed25519, not secp256k1
//!
//! Buzz's equivalent (NIP-OA) is Schnorr over secp256k1 because it lives in a
//! nostr world. Importing a second curve for fidelity to someone else's
//! ecosystem would mean a second signing implementation to keep correct.
//! `economy::signer` is already Ed25519, already reviewed, and already has the
//! base58 key handling — so this reuses it and only adds domain separation.
//!
//! ## Why the domain tags matter
//!
//! Every signature here is over bytes that begin with a tag naming what is
//! being signed. Without one, a signature collected in one context is a valid
//! signature in another: an owner attestation could be replayed as a runner
//! handshake, or a tiny.place payment signature as either. The tags make each
//! keyspace disjoint, which is the same reasoning behind the pairing-code hash
//! prefix in `users::devices`.

use crate::Result;
use crate::error::OpenCompanyError;

/// Domain tag for a runner proving possession of its key.
const RUNNER_DOMAIN: &str = "opencompany-runner-v1";
/// Domain tag for an owner authorising a runner.
const OWNER_DOMAIN: &str = "opencompany-owner-attestation-v1";

/// How far a runner's clock may be from the host's.
///
/// The same window SIWX uses. A tighter one breaks laptops whose clock drifts
/// between sleeps; a looser one widens the replay window a challenge already
/// closes.
pub const SKEW_SECS: i64 = 300;

/// An owner's statement that a runner may act for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerAttestation {
    /// The owner's public key, base58.
    pub owner: String,
    /// What the runner is permitted to do, as an opaque canonical string —
    /// companies, an expiry, a parallelism cap.
    pub conditions: String,
    /// The owner's signature over [`owner_canonical_bytes`].
    pub signature: String,
}

/// A runner's opening message.
#[derive(Clone, Debug)]
pub struct RunnerHello {
    /// The runner's public key, base58. Its identity everywhere.
    pub runner_id: String,
    /// The challenge this host issued for this connection.
    pub challenge: String,
    /// Seconds since the epoch, as the runner sees it.
    pub timestamp: i64,
    /// A hash of what the runner advertised, so its capabilities are signed
    /// rather than merely asserted alongside a signature.
    pub capabilities_hash: String,
    /// The runner's signature over [`runner_canonical_bytes`].
    pub signature: String,
    pub attestation: OwnerAttestation,
}

/// The bytes a runner signs.
///
/// Includes the challenge (so a signature cannot be replayed onto another
/// connection) and the capabilities hash (so what it claims to be able to do is
/// covered by the signature rather than sitting next to it, editable).
pub fn runner_canonical_bytes(
    runner_id: &str,
    challenge: &str,
    timestamp: i64,
    capabilities_hash: &str,
) -> Vec<u8> {
    format!("{RUNNER_DOMAIN}\n{runner_id}\n{challenge}\n{timestamp}\n{capabilities_hash}")
        .into_bytes()
}

/// The bytes an owner signs to authorise a runner.
///
/// Note what is *not* here: the owner's private key never leaves the owner, and
/// this grants no ability to sign as them. It is provenance — "this runner acts
/// for me" — not delegation.
pub fn owner_canonical_bytes(owner: &str, runner_id: &str, conditions: &str) -> Vec<u8> {
    format!("{OWNER_DOMAIN}\n{owner}\n{runner_id}\n{conditions}").into_bytes()
}

/// Verifies a hello: skew, runner signature, then owner attestation.
///
/// Ordered cheapest-first, and the order is also the useful one for an
/// operator: a clock problem is reported as a clock problem rather than as a
/// bad signature.
///
/// The caller must additionally reject a replayed `challenge` — this function
/// cannot, because it holds no state. `NonceCache` is the intended companion.
pub fn verify_hello(hello: &RunnerHello, now: i64) -> Result<()> {
    if (now - hello.timestamp).abs() > SKEW_SECS {
        return Err(OpenCompanyError::InvalidRequest(format!(
            "runner timestamp is outside the ±{SKEW_SECS}s window"
        )));
    }

    let runner_bytes = runner_canonical_bytes(
        &hello.runner_id,
        &hello.challenge,
        hello.timestamp,
        &hello.capabilities_hash,
    );
    crate::economy::signer::verify_b58(&hello.runner_id, &runner_bytes, &hello.signature)
        .map_err(|_| OpenCompanyError::InvalidRequest("runner signature does not verify".into()))?;

    // Authentic is not the same as authorised. A runner that passes the check
    // above and fails this one has proved it is itself and nothing more.
    let owner_bytes = owner_canonical_bytes(
        &hello.attestation.owner,
        &hello.runner_id,
        &hello.attestation.conditions,
    );
    crate::economy::signer::verify_b58(
        &hello.attestation.owner,
        &owner_bytes,
        &hello.attestation.signature,
    )
    .map_err(|_| OpenCompanyError::InvalidRequest("owner attestation does not verify".into()))?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::economy::signer::LocalSigner;

    struct Key {
        signer: LocalSigner,
        id: String,
    }

    fn key(seed: u8) -> Key {
        let signer = LocalSigner::from_seed(&[seed; 32]);
        let id = signer.agent_id();
        Key { signer, id }
    }

    fn hello(runner: &Key, owner: &Key, now: i64) -> RunnerHello {
        let conditions = "companies=acme&max_parallel=2";
        let attestation = OwnerAttestation {
            owner: owner.id.clone(),
            conditions: conditions.to_string(),
            signature: owner
                .signer
                .sign_b58(&owner_canonical_bytes(&owner.id, &runner.id, conditions)),
        };
        let bytes = runner_canonical_bytes(&runner.id, "chal-1", now, "caps-hash");
        RunnerHello {
            runner_id: runner.id.clone(),
            challenge: "chal-1".to_string(),
            timestamp: now,
            capabilities_hash: "caps-hash".to_string(),
            signature: runner.signer.sign_b58(&bytes),
            attestation,
        }
    }

    #[test]
    fn a_well_formed_hello_verifies() {
        let (runner, owner) = (key(1), key(2));
        assert!(verify_hello(&hello(&runner, &owner, 1_000), 1_000).is_ok());
    }

    #[test]
    fn a_stale_clock_is_reported_as_a_clock_problem() {
        // Ordered first so an operator whose laptop slept gets told that,
        // rather than "bad signature" — which would send them looking at keys.
        let (runner, owner) = (key(1), key(2));
        let h = hello(&runner, &owner, 1_000);
        let error = verify_hello(&h, 1_000 + SKEW_SECS + 1).unwrap_err();
        assert!(error.to_string().contains("window"), "{error}");
    }

    #[test]
    fn a_signature_for_another_challenge_is_refused() {
        // The replay this exists to stop: a hello captured on one connection
        // re-presented on another.
        let (runner, owner) = (key(1), key(2));
        let mut h = hello(&runner, &owner, 1_000);
        h.challenge = "a-different-challenge".to_string();
        assert!(verify_hello(&h, 1_000).is_err());
    }

    #[test]
    fn advertised_capabilities_are_covered_by_the_signature() {
        // Otherwise a runner's claims sit *next to* a valid signature and can be
        // edited in flight — a man in the middle could advertise harnesses the
        // machine does not have, and the host would schedule to them.
        let (runner, owner) = (key(1), key(2));
        let mut h = hello(&runner, &owner, 1_000);
        h.capabilities_hash = "tampered".to_string();
        assert!(verify_hello(&h, 1_000).is_err());
    }

    #[test]
    fn a_runner_signing_as_someone_else_is_refused() {
        let (runner, owner, impostor) = (key(1), key(2), key(3));
        let mut h = hello(&runner, &owner, 1_000);
        h.runner_id = impostor.id.clone();
        assert!(verify_hello(&h, 1_000).is_err());
    }

    #[test]
    fn an_authentic_runner_with_no_owner_authorisation_is_still_refused() {
        // THE distinction. This runner genuinely holds its key — it just has
        // nobody's permission to attach itself to this host.
        let (runner, owner, other) = (key(1), key(2), key(3));
        let mut h = hello(&runner, &owner, 1_000);
        // An attestation signed by a different key than it names.
        h.attestation.signature = other.signer.sign_b58(&owner_canonical_bytes(
            &owner.id,
            &runner.id,
            &h.attestation.conditions,
        ));

        let error = verify_hello(&h, 1_000).unwrap_err();
        assert!(error.to_string().contains("attestation"), "{error}");
    }

    #[test]
    fn an_attestation_for_a_different_runner_does_not_transfer() {
        // Lifting someone else's valid attestation is the obvious attack once
        // attestations exist at all.
        let (runner, owner, other_runner) = (key(1), key(2), key(4));
        let conditions = "companies=acme&max_parallel=2";
        let mut h = hello(&runner, &owner, 1_000);
        h.attestation.signature = owner.signer.sign_b58(&owner_canonical_bytes(
            &owner.id,
            &other_runner.id,
            conditions,
        ));
        assert!(verify_hello(&h, 1_000).is_err());
    }

    #[test]
    fn conditions_cannot_be_widened_after_signing() {
        // The conditions carry the companies and the parallelism cap; if they
        // were not signed, a runner could grant itself the whole host.
        let (runner, owner) = (key(1), key(2));
        let mut h = hello(&runner, &owner, 1_000);
        h.attestation.conditions = "companies=*&max_parallel=999".to_string();
        assert!(verify_hello(&h, 1_000).is_err());
    }

    #[test]
    fn the_two_domains_are_disjoint_keyspaces() {
        // Without the tags, a signature collected in one context verifies in
        // the other — an owner attestation replayed as a runner handshake.
        let runner_bytes = runner_canonical_bytes("r", "c", 1, "h");
        let owner_bytes = owner_canonical_bytes("o", "r", "c");
        assert_ne!(runner_bytes, owner_bytes);
        assert!(String::from_utf8_lossy(&runner_bytes).starts_with(RUNNER_DOMAIN));
        assert!(String::from_utf8_lossy(&owner_bytes).starts_with(OWNER_DOMAIN));
    }
}
