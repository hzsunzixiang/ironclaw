//! # OAuth 2.0 — PKCE (Proof Key for Code Exchange)
//!
//! Implements RFC 7636: Proof Key for Code Exchange by OAuth Public Clients.
//!
//! PKCE prevents authorization code interception attacks by binding the
//! authorization request to the token exchange request via a cryptographic
//! challenge. This is REQUIRED for public clients (no client_secret) and
//! RECOMMENDED for all clients per RFC 9700 (OAuth 2.0 Security BCP).
//!
//! Flow:
//!   1. Client generates a random `code_verifier` (43-128 chars, unreserved URI chars)
//!   2. Client computes `code_challenge = BASE64URL(SHA256(code_verifier))`
//!   3. Authorization request includes `code_challenge` + `code_challenge_method=S256`
//!   4. Token exchange includes `code_verifier` — server verifies the hash matches
//!
//! Corresponds to: `src/cli/oauth_defaults.rs` → PKCE generation in `build_oauth_url()`

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// A PKCE code verifier and its corresponding challenge.
///
/// The verifier is a high-entropy random string (32 bytes → 43 base64url chars).
/// The challenge is SHA-256(verifier) encoded as base64url-no-pad.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// The code_verifier — sent in the token exchange request.
    pub verifier: String,
    /// The code_challenge — sent in the authorization request.
    pub challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair using S256 method.
    ///
    /// Per RFC 7636 §4.1: code_verifier is 32 random bytes → 43 base64url chars.
    /// Per RFC 7636 §4.2: code_challenge = BASE64URL(SHA256(code_verifier)).
    pub fn generate() -> Self {
        let mut verifier_bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);
        let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

        Self {
            verifier,
            challenge,
        }
    }
}

/// Generate a cryptographically random state parameter for CSRF protection.
///
/// Per RFC 6749 §10.12: The state parameter SHOULD be used to prevent CSRF.
/// Per RFC 9700 §4.4.1.8: state MUST be unpredictable and tied to the session.
///
/// We use 32 random bytes → 43 base64url chars, providing 256 bits of entropy.
pub fn generate_state() -> String {
    let mut state_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    URL_SAFE_NO_PAD.encode(state_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_valid() {
        let pkce = PkceChallenge::generate();

        // Verifier should be 43 chars (32 bytes base64url-no-pad)
        assert_eq!(pkce.verifier.len(), 43);
        // Challenge should be 43 chars (32 bytes SHA-256 → base64url-no-pad)
        assert_eq!(pkce.challenge.len(), 43);
        // Verifier and challenge should be different
        assert_ne!(pkce.verifier, pkce.challenge);

        // Verify the challenge is correct SHA-256 of verifier
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(pkce.challenge, expected);
    }

    #[test]
    fn pkce_challenges_are_unique() {
        let a = PkceChallenge::generate();
        let b = PkceChallenge::generate();
        assert_ne!(a.verifier, b.verifier);
    }

    #[test]
    fn state_is_random() {
        let a = generate_state();
        let b = generate_state();
        assert_eq!(a.len(), 43);
        assert_ne!(a, b);
    }
}
