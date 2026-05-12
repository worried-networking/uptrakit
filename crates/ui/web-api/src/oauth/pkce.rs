//! PKCE S256 verifier per RFC 7636 §4.6.

use base64::Engine;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Error type for PKCE verification failures.
#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PkceError {
    /// The code_verifier does not match the code_challenge.
    #[error("code_verifier does not match code_challenge")]
    Mismatch,
}

/// Verifies a PKCE code_verifier against a stored code_challenge.
///
/// Per RFC 7636 §4.6, the challenge is the base64url-encoded SHA-256 hash of the verifier.
#[derive(Clone, Debug)]
pub struct PkceVerifier {
    expected_challenge: String,
}

impl PkceVerifier {
    /// Create a new PKCE verifier with the expected code_challenge.
    #[must_use]
    pub fn new(expected_challenge: String) -> Self {
        Self { expected_challenge }
    }

    /// Verify the supplied code_verifier SHA-256s to the expected challenge (base64url, no padding).
    ///
    /// # Errors
    ///
    /// Returns `PkceError::Mismatch` if the computed challenge differs from the expected.
    pub fn verify(&self, code_verifier: &str) -> Result<(), PkceError> {
        let mut hasher = Sha256::new();
        hasher.update(code_verifier.as_bytes());
        let digest = hasher.finalize();
        let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
        if computed == self.expected_challenge {
            Ok(())
        } else {
            Err(PkceError::Mismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b_example() {
        // RFC 7636 §4.6 worked example
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        let v = PkceVerifier::new(challenge.to_string());
        v.verify(verifier)
            .expect("RFC 7636 §4.6 example must verify");
    }

    #[test]
    fn mismatched_verifier_rejected() {
        let v = PkceVerifier::new("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into());
        assert_eq!(v.verify("wrong-verifier"), Err(PkceError::Mismatch));
    }
}
