//! Encryption at rest for sensitive database fields.
//!
//! Uses AES-256-GCM with a global master key to encrypt/decrypt values
//! transparently via the [`EncryptedString`] SeaORM custom type.

use std::fmt;
use std::sync::OnceLock;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;

/// Global master encryption key (32 bytes for AES-256).
static MASTER_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Initialize the global master key. Must be called once at startup.
///
/// Returns `Err` if the key has already been initialized.
pub fn init_master_key(key: [u8; 32]) -> Result<(), String> {
    MASTER_KEY
        .set(key)
        .map_err(|_| "master key already initialized".to_string())
}

/// Returns `true` if the master key has been initialized.
pub fn master_key_available() -> bool {
    MASTER_KEY.get().is_some()
}

/// Prefix for encrypted values stored in the database.
const ENC_PREFIX: &str = "ENC:v1:";

/// Check whether a stored string is already encrypted.
fn is_encrypted(s: &str) -> bool {
    s.starts_with(ENC_PREFIX)
}

/// Encrypt a plaintext string.
///
/// Returns `"ENC:v1:<hex(nonce || ciphertext || tag)>"`.
fn encrypt_value(plaintext: &str) -> Result<String, String> {
    let key_bytes = MASTER_KEY.get().ok_or("master key not initialized")?;

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|e| format!("failed to create encryption key: {e}"))?;
    let key = LessSafeKey::new(unbound);

    // Generate random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Encrypt in-place: plaintext bytes + space for tag
    let mut in_out = plaintext.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| format!("encryption failed: {e}"))?;

    // Build output: nonce || ciphertext || tag
    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!("{ENC_PREFIX}{}", hex::encode(&output)))
}

/// Decrypt a stored encrypted string.
///
/// Strips the `"ENC:v1:"` prefix, hex-decodes, extracts the nonce,
/// and decrypts the ciphertext.
fn decrypt_value(stored: &str) -> Result<String, String> {
    let key_bytes = MASTER_KEY.get().ok_or("master key not initialized")?;

    let hex_part = stored
        .strip_prefix(ENC_PREFIX)
        .ok_or("missing ENC:v1: prefix")?;

    let raw = hex::decode(hex_part).map_err(|e| format!("hex decode failed: {e}"))?;

    // AES-256-GCM: 12-byte nonce + ciphertext + 16-byte tag
    if raw.len() < 12 + 16 {
        return Err("ciphertext too short".to_string());
    }

    let nonce_bytes: [u8; 12] = raw[..12].try_into().map_err(|_| "invalid nonce length")?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|e| format!("failed to create decryption key: {e}"))?;
    let key = LessSafeKey::new(unbound);

    let mut ciphertext_and_tag = raw[12..].to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), &mut ciphertext_and_tag)
        .map_err(|_| "decryption failed (wrong key or tampered data)".to_string())?;

    String::from_utf8(plaintext.to_vec())
        .map_err(|e| format!("decrypted value is not valid UTF-8: {e}"))
}

/// A string that is transparently encrypted when written to the database
/// and decrypted when read back.
///
/// In memory, holds the plaintext value. On conversion to/from `sea_orm::Value`,
/// encryption/decryption is applied.
#[derive(Clone, PartialEq)]
pub struct EncryptedString(String);

impl EncryptedString {
    /// Create a new `EncryptedString` from a plaintext value.
    pub fn new(plaintext: String) -> Self {
        Self(plaintext)
    }

    /// Expose the plaintext secret.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EncryptedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EncryptedString(***)")
    }
}

impl fmt::Display for EncryptedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***REDACTED***")
    }
}

// ── SeaORM integration ──────────────────────────────────────────────

impl sea_orm::sea_query::ValueType for EncryptedString {
    fn try_from(v: sea_orm::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => {
                if is_encrypted(&s) {
                    let plaintext =
                        decrypt_value(&s).map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(EncryptedString(plaintext))
                } else {
                    // Legacy plaintext — accept as-is
                    Ok(EncryptedString(s))
                }
            }
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "EncryptedString".to_string()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> sea_orm::sea_query::ColumnType {
        sea_orm::sea_query::ColumnType::Text
    }
}

impl sea_orm::sea_query::Nullable for EncryptedString {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

impl From<EncryptedString> for sea_orm::Value {
    fn from(val: EncryptedString) -> Self {
        if master_key_available() {
            match encrypt_value(&val.0) {
                Ok(encrypted) => sea_orm::Value::String(Some(encrypted)),
                Err(e) => {
                    tracing::error!(error = %e, "EncryptedString encryption failed, storing plaintext");
                    sea_orm::Value::String(Some(val.0))
                }
            }
        } else {
            // No master key — store plaintext (should not happen in production)
            sea_orm::Value::String(Some(val.0))
        }
    }
}

impl sea_orm::TryGetable for EncryptedString {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, sea_orm::TryGetError> {
        let s: String = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
        if is_encrypted(&s) {
            let plaintext = decrypt_value(&s).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "EncryptedString decryption failed: {e}"
                )))
            })?;
            Ok(EncryptedString(plaintext))
        } else {
            // Legacy plaintext — accept as-is
            Ok(EncryptedString(s))
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests that touch the global MASTER_KEY must run serially.
    // We use a mutex to coordinate since OnceLock can only be set once per process.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Initialize a test key. Since OnceLock can only be set once,
    /// subsequent calls in the same process are no-ops.
    fn ensure_test_key() {
        let key = [0x42u8; 32];
        let _ = init_master_key(key);
    }

    #[test]
    fn test_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        for plaintext in ["hello world", "", "🦀 Rust", "a".repeat(10_000).as_str()] {
            let encrypted = encrypt_value(plaintext).unwrap();
            assert!(is_encrypted(&encrypted));
            let decrypted = decrypt_value(&encrypted).unwrap();
            assert_eq!(decrypted, plaintext);
        }
    }

    #[test]
    fn test_nonce_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let enc1 = encrypt_value("same input").unwrap();
        let enc2 = encrypt_value("same input").unwrap();
        assert_ne!(
            enc1, enc2,
            "two encryptions of the same value should produce different ciphertext"
        );

        // Both should still decrypt to the same value
        assert_eq!(decrypt_value(&enc1).unwrap(), "same input");
        assert_eq!(decrypt_value(&enc2).unwrap(), "same input");
    }

    #[test]
    fn test_is_encrypted_detection() {
        assert!(is_encrypted("ENC:v1:aabbcc"));
        assert!(!is_encrypted("plaintext"));
        assert!(!is_encrypted("ENC:v2:aabbcc"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn test_debug_display_redact() {
        let s = EncryptedString::new("my secret".to_string());
        let debug = format!("{s:?}");
        let display = format!("{s}");
        assert!(
            !debug.contains("my secret"),
            "Debug should not contain plaintext"
        );
        assert!(
            !display.contains("my secret"),
            "Display should not contain plaintext"
        );
        assert!(debug.contains("***"));
        assert!(display.contains("REDACTED"));
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let encrypted = encrypt_value("sensitive data").unwrap();
        // Tamper with one byte in the hex payload
        let hex_part = encrypted.strip_prefix(ENC_PREFIX).unwrap();
        let mut raw = hex::decode(hex_part).unwrap();
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!("{ENC_PREFIX}{}", hex::encode(&raw));
        assert!(decrypt_value(&tampered).is_err());
    }

    #[test]
    fn test_seaorm_value_roundtrip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let original = EncryptedString::new("database secret".to_string());
        let value: sea_orm::Value = original.clone().into();

        // The Value should contain an encrypted string
        if let sea_orm::Value::String(Some(ref s)) = value {
            assert!(is_encrypted(s), "Value should be encrypted");
        } else {
            panic!("Expected String value");
        }

        // Round-trip via ValueType
        let restored = <EncryptedString as sea_orm::sea_query::ValueType>::try_from(value).unwrap();
        assert_eq!(restored.expose_secret(), "database secret");
    }

    #[test]
    fn test_legacy_plaintext_accepted() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Simulate reading a legacy plaintext value from DB
        let legacy_value = sea_orm::Value::String(Some("old_password".to_string()));
        let result = <EncryptedString as sea_orm::sea_query::ValueType>::try_from(legacy_value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().expose_secret(), "old_password");
    }
}
