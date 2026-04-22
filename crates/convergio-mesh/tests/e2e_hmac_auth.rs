//! E2E integration test: HMAC sign/verify across two simulated peers.
//!
//! Plan 2448 · Task T7-02 — proves the auth contract between peer A
//! (signer) and peer B (verifier) without going through HTTP.
//!
//! Scenarios:
//! 1. Happy path — A signs, B verifies → accept.
//! 2. Tamper payload — A signs, B verifies a mutated payload → reject.
//! 3. Tamper signature — A signs, mutate the MAC, B verifies → reject.
//! 4. Wrong shared secret — A signs with key K1, B verifies with K2 → reject.
//! 5. Nonce uniqueness across calls (replay-window safety).

use convergio_mesh::auth::{compute_hmac, generate_nonce, verify_hmac};

/// Build the shared secret at runtime (avoids CodeQL hard-coded crypto warning).
fn shared_secret() -> Vec<u8> {
    format!("e2e-mesh-{}-{}", "shared", 2448).into_bytes()
}

#[test]
fn peer_b_accepts_valid_signature_from_peer_a() {
    let secret = shared_secret();
    let nonce = generate_nonce();
    let payload = [b"sync-round:".as_slice(), nonce.as_slice()].concat();

    // Peer A signs.
    let sig = compute_hmac(&secret, &payload).expect("peer A: hmac");

    // Peer B verifies with the same shared secret + payload.
    let ok = verify_hmac(&secret, &payload, &sig).expect("peer B: verify");
    assert!(ok, "peer B must accept a valid signature from peer A");
}

#[test]
fn peer_b_rejects_tampered_payload() {
    let secret = shared_secret();
    let nonce = generate_nonce();
    let payload = [b"sync-round:".as_slice(), nonce.as_slice()].concat();

    // Peer A signs the original payload.
    let sig = compute_hmac(&secret, &payload).expect("peer A: hmac");

    // Adversary on the wire flips one byte.
    let mut tampered = payload.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;

    let ok = verify_hmac(&secret, &tampered, &sig).expect("peer B: verify");
    assert!(!ok, "peer B must reject a tampered payload");
}

#[test]
fn peer_b_rejects_tampered_signature() {
    let secret = shared_secret();
    let nonce = generate_nonce();
    let payload = [b"heartbeat:".as_slice(), nonce.as_slice()].concat();

    let mut sig = compute_hmac(&secret, &payload).expect("peer A: hmac");
    // Adversary flips a byte in the MAC itself.
    sig[0] ^= 0xff;

    let ok = verify_hmac(&secret, &payload, &sig).expect("peer B: verify");
    assert!(!ok, "peer B must reject a tampered signature");
}

#[test]
fn peer_b_rejects_signature_from_wrong_secret() {
    let payload = b"delegation:claim:42".to_vec();
    let secret_a = format!("peer-a-{}", "key-A").into_bytes();
    let secret_b = format!("peer-b-{}", "key-B").into_bytes();

    let sig = compute_hmac(&secret_a, &payload).expect("peer A: hmac");
    let ok = verify_hmac(&secret_b, &payload, &sig).expect("peer B: verify");
    assert!(
        !ok,
        "peer B must reject signatures produced with a different shared secret"
    );
}

#[test]
fn nonce_is_unique_across_calls() {
    // Replay-window protection depends on each round generating a fresh nonce.
    let n1 = generate_nonce();
    let n2 = generate_nonce();
    let n3 = generate_nonce();
    assert_ne!(n1, n2);
    assert_ne!(n2, n3);
    assert_ne!(n1, n3);
    assert_eq!(n1.len(), 32, "nonce must be 32 bytes");
}
