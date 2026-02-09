use super::Result;
use argon2::{
    Argon2, Params,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rootcause::prelude::*;

pub const MIN_PASSWORD_LENGTH: usize = 8;
pub const MAX_PASSWORD_LENGTH: usize = 1024;

pub fn validate_password_length(password: &str) -> Option<&'static str> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Some("Password must be at least 8 characters");
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Some("Password must be at most 1024 characters");
    }
    None
}

/// Hash a password using Argon2id with OWASP-recommended parameters
///
/// Parameters:
/// - Algorithm: Argon2id (hybrid mode)
/// - Memory: 19 MiB (19456 KiB)
/// - Iterations: 2
/// - Parallelism: 1
/// - Random salt per password
pub fn hash_password(password: &str) -> Result<String> {
    // OWASP recommended parameters for Argon2id
    let params = Params::new(
        19456, // 19 MiB memory cost
        2,     // 2 iterations
        1,     // parallelism
        None,  // output length (default)
    )
    .context_to()?;

    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let salt = SaltString::generate(&mut OsRng);

    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .context_to()?;

    Ok(password_hash.to_string())
}

/// Verify a password against an Argon2id hash using constant-time comparison
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash).context_to()?;

    match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(e).context_to(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password_produces_valid_argon2id() {
        let password = "TestPassword123!";
        let hash = hash_password(password).unwrap();

        // Check that hash starts with $argon2id$
        assert!(
            hash.starts_with("$argon2id$"),
            "Hash should use Argon2id algorithm"
        );

        // Check that hash can be parsed
        assert!(PasswordHash::new(&hash).is_ok(), "Hash should be valid");
    }

    #[test]
    fn test_verify_password_succeeds_with_correct_password() {
        let password = "CorrectPassword123!";
        let hash = hash_password(password).unwrap();

        assert!(
            verify_password(password, &hash).unwrap(),
            "Verification should succeed with correct password"
        );
    }

    #[test]
    fn test_verify_password_fails_with_wrong_password() {
        let password = "CorrectPassword123!";
        let wrong_password = "WrongPassword123!";
        let hash = hash_password(password).unwrap();

        assert!(
            !verify_password(wrong_password, &hash).unwrap(),
            "Verification should fail with wrong password"
        );
    }

    #[test]
    fn test_same_password_produces_different_hashes() {
        let password = "SamePassword123!";
        let hash1 = hash_password(password).unwrap();
        let hash2 = hash_password(password).unwrap();

        assert_ne!(
            hash1, hash2,
            "Same password should produce different hashes due to different salts"
        );

        // Both should still verify correctly
        assert!(verify_password(password, &hash1).unwrap());
        assert!(verify_password(password, &hash2).unwrap());
    }

    #[test]
    fn test_verify_invalid_hash_format() {
        let result = verify_password("password", "not-a-valid-hash");
        assert!(result.is_err(), "Should error on invalid hash format");
    }

    #[test]
    fn password_length_validation_rejects_too_short() {
        assert!(validate_password_length("short").is_some());
    }

    #[test]
    fn password_length_validation_rejects_too_long() {
        let too_long = "a".repeat(MAX_PASSWORD_LENGTH + 1);
        assert!(validate_password_length(&too_long).is_some());
    }

    #[test]
    fn password_length_validation_accepts_valid() {
        let valid = "a".repeat(MIN_PASSWORD_LENGTH);
        assert!(validate_password_length(&valid).is_none());
    }
}
