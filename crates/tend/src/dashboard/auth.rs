//! Bearer-token authentication for the dashboard HTTP server.
//!
//! [`check_bearer`] is framework-agnostic — it takes the raw
//! `Authorization` header value (or `None`) and an expected token, and
//! returns whether the request should pass. The axum middleware that
//! wraps it lives in this module's caller in a later commit.
//!
//! Comparison is constant-time and length-padded so an attacker can't
//! infer either the token's length or any prefix-match progress from
//! response timing. Length information is technically not secret for
//! fixed-length tokens (e.g. an `openssl rand`'d 32-byte hex), but the
//! padding is cheap and we get to make this guarantee unconditionally.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// Domain separation for [`server_proof`]: nothing else is signed with the
/// token, and a later version of the proof gets a new label.
pub const SERVER_PROOF_LABEL: &str = "tend-dashboard-proof/v1\n";

/// A client-chosen nonce for `/api/proof`: 32 to 128 hex digits, so a
/// caller cannot make the server sign arbitrary text.
#[must_use]
pub fn valid_proof_nonce(nonce: &str) -> bool {
    (32..=128).contains(&nonce.len()) && nonce.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The dashboard's proof that it knows `token`, for a client that has not
/// sent it yet: lowercase hex `HMAC-SHA256(token, SERVER_PROOF_LABEL ||
/// node || "\n" || bind || "\n" || nonce)`.
///
/// A listener that merely holds the port cannot answer. `node` (this host's
/// Fleet node id, from the owner-only `host-id` file) and `bind` (the
/// address this dashboard listens on) tie the answer to this daemon: an
/// impostor that relays the nonce to another tend sharing the token (a
/// Fleet host, a published dashboard) gets an answer for that one's node
/// or address, which the client rejects. The Windows app checks it on
/// every connection before the first request that carries the token.
/// With a high-entropy token (`tend init` makes 256 bits) a (nonce, proof)
/// pair is no practical help in guessing it; a short hand-picked token could
/// be attacked offline, which is why the app refuses tokens under 32
/// characters.
#[must_use]
pub fn server_proof(token: &str, node: &str, bind: &str, nonce: &str) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(token.as_bytes()).expect("HMAC accepts a key of any length");
    mac.update(SERVER_PROOF_LABEL.as_bytes());
    for part in [node, "\n", bind, "\n", nonce] {
        mac.update(part.as_bytes());
    }
    hex(&mac.finalize().into_bytes())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Constant-time check: does `header` carry a `Bearer <token>` that
/// matches `expected`? Returns `false` for `None`, missing prefix, or
/// any mismatch.
///
/// If `expected` is empty, the function returns `false` for every
/// possible input — callers should not invoke this at all when no token
/// is configured (the [`DashboardConfig`](super::DashboardConfig) layer
/// already gates auth on `Option<String>`).
#[must_use]
pub fn check_bearer(header: Option<&str>, expected: &str) -> bool {
    let Some(h) = header else {
        return false;
    };
    let Some(got) = h.strip_prefix("Bearer ") else {
        return false;
    };

    // Pad both sides to the longer length with NUL so the byte-wise
    // comparison runs in time independent of the inputs' shapes. We
    // additionally compare lengths in constant time and AND the two
    // results — content-equal-after-pad alone could spuriously match
    // shorter strings that happen to be NUL-suffixed.
    let max = got.len().max(expected.len());
    let mut a = vec![0u8; max];
    let mut b = vec![0u8; max];
    a[..got.len()].copy_from_slice(got.as_bytes());
    b[..expected.len()].copy_from_slice(expected.as_bytes());

    let same_content = a.ct_eq(&b);
    let same_len = (got.len() as u64).ct_eq(&(expected.len() as u64));
    bool::from(same_content & same_len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_proof_matches_hmac_sha256_over_the_label_node_bind_and_nonce() {
        let nonce = "00112233445566778899aabbccddeeff";
        let (node, bind) = ("6f1c0a52-0000-4000-8000-000000000001", "127.0.0.1:7878");
        let proof = server_proof("Jefe", node, bind, nonce);
        assert_eq!(proof.len(), 64);
        assert_eq!(
            proof,
            server_proof("Jefe", node, bind, nonce),
            "deterministic"
        );
        assert_ne!(
            proof,
            server_proof("Jefe!", node, bind, nonce),
            "keyed by the token"
        );
        assert_ne!(
            proof,
            server_proof("Jefe", node, bind, "00112233445566778899aabbccddeef0"),
            "bound to the nonce"
        );
        assert_ne!(
            proof,
            server_proof("Jefe", "other", bind, nonce),
            "bound to the node"
        );
        assert_ne!(
            proof,
            server_proof("Jefe", node, "127.0.0.1:7879", nonce),
            "bound to the address"
        );
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(
            b"tend-dashboard-proof/v1\n6f1c0a52-0000-4000-8000-000000000001\n127.0.0.1:7878\n00112233445566778899aabbccddeeff",
        );
        assert_eq!(proof, hex(&mac.finalize().into_bytes()));
    }

    #[test]
    fn proof_nonces_are_bounded_hex() {
        assert!(valid_proof_nonce(&"a".repeat(32)));
        assert!(valid_proof_nonce(&"F".repeat(128)));
        assert!(!valid_proof_nonce(&"a".repeat(31)));
        assert!(!valid_proof_nonce(&"a".repeat(129)));
        assert!(!valid_proof_nonce(&format!("{}g", "a".repeat(31))));
        assert!(!valid_proof_nonce(
            "sign this text please, it is long enough"
        ));
    }

    #[test]
    fn correct_token_returns_true() {
        assert!(check_bearer(Some("Bearer s3cret"), "s3cret"));
    }

    #[test]
    fn wrong_token_returns_false() {
        assert!(!check_bearer(Some("Bearer s3cret"), "differ"));
    }

    #[test]
    fn shorter_got_returns_false() {
        assert!(!check_bearer(Some("Bearer abc"), "abcdef"));
    }

    #[test]
    fn longer_got_returns_false() {
        assert!(!check_bearer(Some("Bearer abcdef"), "abc"));
    }

    #[test]
    fn missing_bearer_prefix_returns_false() {
        assert!(!check_bearer(Some("s3cret"), "s3cret"));
        assert!(!check_bearer(Some("Token s3cret"), "s3cret"));
    }

    #[test]
    fn no_header_returns_false() {
        assert!(!check_bearer(None, "s3cret"));
    }

    #[test]
    fn empty_token_after_bearer_returns_false() {
        assert!(!check_bearer(Some("Bearer "), "s3cret"));
    }

    #[test]
    fn padded_input_does_not_spuriously_match() {
        // got = "abc\0", expected = "abc" — different lengths, must reject.
        assert!(!check_bearer(Some("Bearer abc\0"), "abc"));
    }
}
