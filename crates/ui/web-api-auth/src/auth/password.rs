use super::Result;
use argon2::{Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier};
use rootcause::prelude::*;
use uptrakit_shared_types::SecretString;

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
pub fn hash_password(password: &str) -> Result<SecretString> {
    // OWASP recommended parameters for Argon2id
    let params = Params::new(
        19456, // 19 MiB memory cost
        2,     // 2 iterations
        1,     // parallelism
        None,  // output length (default)
    )
    .context_to()?;

    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    // `hash_password` draws a fresh 16-byte salt from the system RNG per call.
    let password_hash: PasswordHash = argon2.hash_password(password.as_bytes()).context_to()?;

    Ok(SecretString::new(password_hash.to_string()))
}

/// Verify a password against an Argon2id hash using constant-time comparison
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    // The `&str` verifier parses the PHC string itself and reports a malformed
    // hash as an error, keeping the parse failure distinct from a wrong password.
    match Argon2::default().verify_password(password.as_bytes(), hash) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::PasswordInvalid) => Ok(false),
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
        let exposed = hash.expose_secret();

        // Check that hash starts with $argon2id$
        assert!(
            exposed.starts_with("$argon2id$"),
            "Hash should use Argon2id algorithm"
        );

        // Check that hash can be parsed
        assert!(PasswordHash::new(exposed).is_ok(), "Hash should be valid");
    }

    #[test]
    fn test_verify_password_succeeds_with_correct_password() {
        let password = "CorrectPassword123!";
        let hash = hash_password(password).unwrap();

        assert!(
            verify_password(password, hash.expose_secret()).unwrap(),
            "Verification should succeed with correct password"
        );
    }

    #[test]
    fn test_verify_password_fails_with_wrong_password() {
        let password = "CorrectPassword123!";
        let wrong_password = "WrongPassword123!";
        let hash = hash_password(password).unwrap();

        assert!(
            !verify_password(wrong_password, hash.expose_secret()).unwrap(),
            "Verification should fail with wrong password"
        );
    }

    #[test]
    fn test_verify_invalid_hash_format() {
        let result = verify_password("password", "not-a-valid-hash");
        assert!(result.is_err(), "Should error on invalid hash format");
    }

    /// PHC strings written by argon2 0.5 must still verify after the 0.6 bump.
    ///
    /// These two hashes were produced by argon2 0.5.3 with the same OWASP
    /// parameters `hash_password` uses. Stored `users.password_hash` values
    /// predate the bump, so a verification break here is a silent credential
    /// break for every existing account.
    #[test]
    fn verify_password_accepts_hashes_written_by_argon2_0_5() {
        const LEGACY: [(&str, &str); 2] = [
            (
                "correct horse battery staple",
                "$argon2id$v=19$m=19456,t=2,p=1$MuOxdk6hLDGGrvAbv9jfPg$ltt0Ho0sZt2kac3bIGgyFCUqcMTL3nfCHWTXO2bIgT4",
            ),
            (
                "hunter2",
                "$argon2id$v=19$m=19456,t=2,p=1$rG5S/QEFzI2BWW338jv0oQ$PcuedpVOqtlZEj4lOHfKrzFUa7UbGazmnqID1ssiKNI",
            ),
        ];

        for (password, hash) in LEGACY {
            assert!(
                verify_password(password, hash).unwrap(),
                "argon2 0.5 hash for {password:?} must verify under argon2 0.6"
            );
            assert!(
                !verify_password("some other password", hash).unwrap(),
                "argon2 0.5 hash for {password:?} must reject a wrong password"
            );
        }
    }

    /// The change-password routes call `verify_password` with a fixed dummy
    /// string to equalise timing for unknown users. That string is not a
    /// complete PHC hash, so it must produce an error, never a panic.
    #[test]
    fn verify_password_errors_on_the_timing_dummy_hash() {
        verify_password("dummy", "$argon2id$dummy").unwrap_err();
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
