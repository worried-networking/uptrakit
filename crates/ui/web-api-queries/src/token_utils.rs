//! Lightweight token utilities needed by query modules.
//!
//! These are duplicated from `uptrakit-web-api-auth` to avoid a dependency
//! between the two sibling crates. Both implementations are trivial wrappers
//! around standard library functions.

use sha2::{Digest, Sha256};

/// Generate a new UUID v7 (time-ordered).
pub fn generate_uuid() -> uuid::Uuid {
    uuid::Uuid::now_v7()
}

/// Hash a token using SHA-256 for database storage.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    uptrakit_shared_types::hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_uuid_v7() {
        let uuid = generate_uuid();
        assert_eq!(uuid.get_version_num(), 7);
    }

    /// Known-answer test, deliberately using the same vector as
    /// `uptrakit_web_api_auth::auth::token`'s copy of `hash_token`. The two
    /// implementations are duplicated (see the module docs) but must agree byte for
    /// byte — a token hashed by one is looked up by the other, and the result is
    /// persisted in `api_tokens.token_hash`. Pinning both against the same digest
    /// catches drift between the copies and any change to the hex encoding.
    #[test]
    fn hash_token_matches_known_sha256_vector() {
        assert_eq!(
            hash_token("test-token-123"),
            "19b6b086eebb807f54e6327309dec0ff347a6c3c30bf3bb396f167513eba3475"
        );
    }

    #[test]
    fn hash_token_consistent() {
        let hash1 = hash_token("test-token");
        let hash2 = hash_token("test-token");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn hash_token_different_inputs() {
        assert_ne!(hash_token("a"), hash_token("b"));
    }
}
