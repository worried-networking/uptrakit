use super::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};

/// Generate a cryptographically secure random token (32 bytes, base64url encoded)
pub fn generate_secure_token() -> Result<String> {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes);

    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Hash a token using SHA-256 for database storage
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Generate a new UUID v7 (time-ordered)
pub fn generate_uuid() -> uuid::Uuid {
    uuid::Uuid::now_v7()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_secure_token_produces_unique_tokens() {
        let token1 = generate_secure_token().unwrap();
        let token2 = generate_secure_token().unwrap();

        assert_ne!(token1, token2, "Tokens should be unique");
    }

    #[test]
    fn test_generate_secure_token_correct_length() {
        let token = generate_secure_token().unwrap();

        // 32 bytes base64url encoded without padding should be 43 characters
        assert_eq!(token.len(), 43, "Token should be 43 characters");

        // Should be valid base64url (no padding, URL-safe characters)
        assert!(
            token
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
            "Token should only contain base64url characters"
        );
    }

    #[test]
    fn test_hash_token_produces_consistent_hash() {
        let token = "test-token-123";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);

        assert_eq!(hash1, hash2, "Same token should produce same hash");
    }

    #[test]
    fn test_hash_token_different_tokens_different_hashes() {
        let token1 = "test-token-1";
        let token2 = "test-token-2";

        let hash1 = hash_token(token1);
        let hash2 = hash_token(token2);

        assert_ne!(
            hash1, hash2,
            "Different tokens should produce different hashes"
        );
    }

    #[test]
    fn test_hash_token_produces_hex_string() {
        let token = "test-token";
        let hash = hash_token(token);

        // SHA-256 produces 32 bytes = 64 hex characters
        assert_eq!(hash.len(), 64, "Hash should be 64 hex characters");
        assert!(
            hash.chars().all(|c| c.is_ascii_hexdigit()),
            "Hash should only contain hex digits"
        );
    }

    #[test]
    fn test_generate_uuid_v7() {
        let uuid1 = generate_uuid();
        let uuid2 = generate_uuid();

        // UUIDs should be unique
        assert_ne!(uuid1, uuid2, "UUIDs should be unique");

        // UUID v7 should have version bits set correctly
        assert_eq!(uuid1.get_version_num(), 7, "Should be UUID v7");
        assert_eq!(uuid2.get_version_num(), 7, "Should be UUID v7");
    }
}
