/// Integration test that verifies `EncryptedString::new` and `encrypt_str` return
/// `Err(CryptoError::NotInitialized)` when the master key has never been set.
///
/// This test **must** run in a process where `init_master_key` has never been called.
/// It is compiled into a separate integration-test binary, so both `cargo test` and
/// `cargo nextest run` run it in an isolated process — guaranteeing the `OnceLock`
/// starts unset.
use uptrakit_crypto::{CryptoError, EncryptedString, encrypt_str};

#[test]
fn encrypted_string_new_without_key_returns_not_initialized() {
    let result = EncryptedString::new("secret".to_string(), "test-aad");
    assert!(result.is_err(), "expected Err when master key is absent");
    assert!(
        matches!(
            result.unwrap_err().current_context(),
            CryptoError::NotInitialized
        ),
        "expected CryptoError::NotInitialized"
    );
}

#[test]
fn encrypt_str_without_key_returns_not_initialized() {
    let result = encrypt_str("secret", "test-aad");
    assert!(result.is_err(), "expected Err when master key is absent");
    assert!(
        matches!(
            result.unwrap_err().current_context(),
            CryptoError::NotInitialized
        ),
        "expected CryptoError::NotInitialized"
    );
}
