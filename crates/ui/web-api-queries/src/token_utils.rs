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
    format!("{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_uuid_v7() {
        let uuid = generate_uuid();
        assert_eq!(uuid.get_version_num(), 7);
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
