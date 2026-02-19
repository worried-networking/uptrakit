//! Encryption at rest for sensitive database fields.
//!
//! Uses AES-256-GCM with a global master key to encrypt/decrypt values
//! transparently via the [`EncryptedString`] SeaORM custom type.

use std::fmt;
use std::sync::OnceLock;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::SecretString;
use zeroize::Zeroizing;

/// Errors originating from encryption/decryption operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("master key already initialized")]
    AlreadyInitialized,

    #[error("master key not initialized")]
    NotInitialized,

    #[error("encryption key creation failed: {0}")]
    KeyCreation(String),

    #[error("encryption failed: {0}")]
    Encryption(String),

    #[error("decryption failed: {0}")]
    Decryption(String),

    #[error("hex decode failed")]
    HexDecode(#[from] uptrakit_shared_types::hex::DecodeError),

    #[error("ciphertext too short")]
    CiphertextTooShort,

    #[error("invalid nonce length")]
    InvalidNonce,

    #[error("decrypted value is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),

    #[error("master key does not match existing encrypted data")]
    MasterKeyMismatch,
}

pub type Result<T> = std::result::Result<T, Report<CryptoError>>;

impl_report_conversion! {
    uptrakit_shared_types::hex::DecodeError => CryptoError::HexDecode,
    std::string::FromUtf8Error => CryptoError::InvalidUtf8,
}

/// Global master encryption key (32 bytes for AES-256).
///
/// Wrapped in `Zeroizing` so that the key material is scrubbed from memory
/// if the value is ever dropped (defense-in-depth — `OnceLock` statics have
/// `'static` lifetime and are not normally dropped).
static MASTER_KEY: OnceLock<Zeroizing<[u8; 32]>> = OnceLock::new();

/// Initialize the global master key. Must be called once at startup.
///
/// The key bytes are wrapped in [`Zeroizing`] so that any intermediate copy
/// is scrubbed when dropped.
///
/// Returns `Err` if the key has already been initialized.
pub fn init_master_key(key: Zeroizing<[u8; 32]>) -> Result<()> {
    MASTER_KEY
        .set(key)
        .map_err(|_| report!(CryptoError::AlreadyInitialized))
}

/// Returns `true` if the master key has been initialized.
pub fn master_key_available() -> bool {
    MASTER_KEY.get().is_some()
}

// ── Master key verification ──────────────────────────────────────────

/// Sentinel plaintext used to verify master key consistency across HA instances.
const KEY_VERIFICATION_SENTINEL: &str = "uptrakit-master-key-ok-v1";

/// Create an encrypted verification token from the sentinel value.
///
/// The returned string should be stored in the settings table.
/// On subsequent startups, call [`verify_key_verification_token`] with
/// the stored value to ensure the same master key is in use.
pub fn create_key_verification_token() -> Result<String> {
    encrypt_value(KEY_VERIFICATION_SENTINEL)
}

/// Verify a stored key-verification token against the current master key.
///
/// Decrypts the token and checks that the plaintext matches the expected
/// sentinel. Returns `Err(MasterKeyMismatch)` if decryption succeeds but
/// the plaintext differs, or if decryption itself fails (wrong key).
pub fn verify_key_verification_token(stored: &str) -> Result<()> {
    match decrypt_value(stored) {
        Ok(plaintext) if plaintext == KEY_VERIFICATION_SENTINEL => Ok(()),
        Ok(_) => bail!(CryptoError::MasterKeyMismatch),
        Err(e) => {
            tracing::debug!(error = %e, "key verification decryption failed");
            bail!(CryptoError::MasterKeyMismatch)
        }
    }
}

/// Prefix for encrypted values stored in the database.
const ENC_PREFIX: &str = "ENC:v1:";

/// Check whether a stored string is already encrypted (has the `ENC:v1:` prefix).
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(ENC_PREFIX)
}

/// Encrypt a plaintext string.
///
/// Returns `"ENC:v1:<hex(nonce || ciphertext || tag)>"`.
fn encrypt_value(plaintext: &str) -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    // Generate random 12-byte nonce
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Encrypt in-place: plaintext bytes + space for tag
    let mut in_out = plaintext.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    // Build output: nonce || ciphertext || tag
    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!(
        "{ENC_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&output)
    ))
}

/// Decrypt a stored encrypted string.
///
/// Strips the `"ENC:v1:"` prefix, hex-decodes, extracts the nonce,
/// and decrypts the ciphertext.
fn decrypt_value(stored: &str) -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let hex_part = stored
        .strip_prefix(ENC_PREFIX)
        .ok_or_else(|| report!(CryptoError::Decryption("missing ENC:v1: prefix".into())))?;

    let raw = uptrakit_shared_types::hex::decode(hex_part).context_to()?;

    // AES-256-GCM: 12-byte nonce + ciphertext + 16-byte tag
    if raw.len() < 12 + 16 {
        bail!(CryptoError::CiphertextTooShort);
    }

    let nonce_bytes: [u8; 12] = raw[..12]
        .try_into()
        .map_err(|_| report!(CryptoError::InvalidNonce))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut ciphertext_and_tag = raw[12..].to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), &mut ciphertext_and_tag)
        .map_err(|_| report!(CryptoError::Decryption("wrong key or tampered data".into())))?;

    String::from_utf8(plaintext.to_vec()).context_to()
}

/// A string that is transparently encrypted when written to the database
/// and decrypted when read back.
///
/// Encryption is performed eagerly at construction time. The pre-computed
/// database representation is stored alongside the plaintext so that
/// `From<EncryptedString> for sea_orm::Value` is infallible.
///
/// When no master key is configured (development mode), the plaintext is
/// stored as the DB value. Construction fails if the master key is present
/// but encryption fails — this prevents silent plaintext fallback in
/// production.
pub struct EncryptedString {
    /// Plaintext value (for `expose_secret`).
    plaintext: SecretString,
    /// Pre-computed value for database storage (encrypted, or plaintext in dev mode).
    db_value: String,
}

impl Clone for EncryptedString {
    fn clone(&self) -> Self {
        Self {
            plaintext: self.plaintext.clone(),
            db_value: self.db_value.clone(),
        }
    }
}

impl PartialEq for EncryptedString {
    fn eq(&self, other: &Self) -> bool {
        // Compare only plaintext — encrypted values include random nonces
        // so two encryptions of the same value differ.
        self.plaintext.expose_secret() == other.plaintext.expose_secret()
    }
}

impl EncryptedString {
    /// Create a new `EncryptedString` from a plaintext value.
    ///
    /// Encrypts immediately if a master key is available. Returns `Err` if
    /// encryption fails. When no master key is configured, stores plaintext
    /// as the DB value (development mode).
    pub fn new(plaintext: String) -> Result<Self> {
        let db_value = if master_key_available() {
            encrypt_value(&plaintext)?
        } else {
            tracing::warn!(
                "master key not configured; storing value as plaintext (development mode)"
            );
            plaintext.clone()
        };
        Ok(Self {
            plaintext: SecretString::new(plaintext),
            db_value,
        })
    }

    /// Construct from a decrypted DB value on the read path.
    ///
    /// Used by `ValueType` / `TryGetable` impls to construct from a decrypted
    /// value while preserving the original DB representation.
    fn from_db(plaintext: String, db_repr: String) -> Self {
        Self {
            plaintext: SecretString::new(plaintext),
            db_value: db_repr,
        }
    }

    /// Expose the plaintext secret.
    pub fn expose_secret(&self) -> &str {
        self.plaintext.expose_secret()
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
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => {
                if is_encrypted(&s) {
                    let plaintext =
                        decrypt_value(&s).map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(EncryptedString::from_db(plaintext, s))
                } else {
                    // Legacy plaintext — accept as-is
                    Ok(EncryptedString::from_db(s.clone(), s))
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
        sea_orm::Value::String(Some(val.db_value))
    }
}

impl sea_orm::TryGetable for EncryptedString {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> std::result::Result<Self, sea_orm::TryGetError> {
        let s: Option<String> = res.try_get_by(index).map_err(sea_orm::TryGetError::DbErr)?;
        let s = match s {
            Some(s) => s,
            None => {
                let column_name = match index.as_str() {
                    Some(name) => name.to_string(),
                    None => "encrypted_string".to_string(),
                };
                return Err(sea_orm::TryGetError::Null(column_name));
            }
        };

        if is_encrypted(&s) {
            let plaintext = decrypt_value(&s).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "EncryptedString decryption failed: {e}"
                )))
            })?;
            Ok(EncryptedString::from_db(plaintext, s))
        } else {
            // Legacy plaintext — accept as-is
            Ok(EncryptedString::from_db(s.clone(), s))
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
        let key = Zeroizing::new([0x42u8; 32]);
        let _ = init_master_key(key);
    }

    #[test]
    fn test_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        for plaintext in [
            "hello world",
            "",
            "\u{1f980} Rust",
            "a".repeat(10_000).as_str(),
        ] {
            let encrypted = encrypt_value(plaintext).expect("encryption should succeed");
            assert!(is_encrypted(&encrypted));
            let decrypted = decrypt_value(&encrypted).expect("decryption should succeed");
            assert_eq!(decrypted, plaintext);
        }
    }

    #[test]
    fn test_nonce_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let enc1 = encrypt_value("same input").expect("encryption should succeed");
        let enc2 = encrypt_value("same input").expect("encryption should succeed");
        assert_ne!(
            enc1, enc2,
            "two encryptions of the same value should produce different ciphertext"
        );

        // Both should still decrypt to the same value
        assert_eq!(
            decrypt_value(&enc1).expect("decryption should succeed"),
            "same input"
        );
        assert_eq!(
            decrypt_value(&enc2).expect("decryption should succeed"),
            "same input"
        );
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
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let s = EncryptedString::new("my secret".to_string()).expect("test key set");
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

        let encrypted = encrypt_value("sensitive data").expect("encryption should succeed");
        // Tamper with one byte in the hex payload
        let hex_part = encrypted
            .strip_prefix(ENC_PREFIX)
            .expect("should have prefix");
        let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!("{ENC_PREFIX}{}", uptrakit_shared_types::hex::encode(&raw));
        assert!(decrypt_value(&tampered).is_err());
    }

    #[test]
    fn test_seaorm_value_roundtrip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let original = EncryptedString::new("database secret".to_string()).expect("test key set");
        let value: sea_orm::Value = original.clone().into();

        // The Value should contain an encrypted string
        if let sea_orm::Value::String(Some(ref s)) = value {
            assert!(is_encrypted(s), "Value should be encrypted");
        } else {
            panic!("Expected String value");
        }

        // Round-trip via ValueType
        let restored =
            <EncryptedString as sea_orm::sea_query::ValueType>::try_from(value).expect("roundtrip");
        assert_eq!(restored.expose_secret(), "database secret");
    }

    #[test]
    fn test_new_encrypts_when_key_available() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es = EncryptedString::new("secret".to_string()).expect("test key set");
        assert!(
            is_encrypted(&es.db_value),
            "db_value should be encrypted when master key is available"
        );
        assert_eq!(es.expose_secret(), "secret");
    }

    #[test]
    fn test_from_impl_uses_precomputed_value() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es = EncryptedString::new("precomputed".to_string()).expect("test key set");
        let precomputed = es.db_value.clone();
        let value: sea_orm::Value = es.into();

        // The Value must contain exactly the pre-computed db_value
        if let sea_orm::Value::String(Some(s)) = value {
            assert_eq!(s, precomputed);
        } else {
            panic!("Expected String value");
        }
    }

    #[test]
    fn test_legacy_plaintext_accepted() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Simulate reading a legacy plaintext value from DB
        let legacy_value = sea_orm::Value::String(Some("old_password".to_string()));
        let result = <EncryptedString as sea_orm::sea_query::ValueType>::try_from(legacy_value);
        assert!(result.is_ok());
        assert_eq!(
            result.expect("should be ok").expose_secret(),
            "old_password"
        );
    }

    #[test]
    fn test_key_verification_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let token = create_key_verification_token().expect("should create token");
        assert!(is_encrypted(&token));
        assert!(verify_key_verification_token(&token).is_ok());
    }

    #[test]
    fn test_key_verification_rejects_tampered_token() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let token = create_key_verification_token().expect("should create token");
        // Tamper with the ciphertext
        let hex_part = token.strip_prefix(ENC_PREFIX).expect("has prefix");
        let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!("{ENC_PREFIX}{}", uptrakit_shared_types::hex::encode(&raw));
        let result = verify_key_verification_token(&tampered);
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err().current_context(),
                CryptoError::MasterKeyMismatch
            ),
            "expected MasterKeyMismatch"
        );
    }

    #[tokio::test]
    async fn test_nullable_encrypted_string_decodes_to_none() {
        use crate::entity::mqtt_client;
        use crate::entity::prelude::MqttClient;
        use sea_orm::{DbBackend, EntityTrait, MockDatabase};
        use time::OffsetDateTime;
        use uptrakit_shared_types::{MqttClientConnectionStatus, MqttTransport};
        use uuid::Uuid;

        let model = mqtt_client::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            enabled: true,
            transport: MqttTransport::Tcp,
            host: "broker".to_string(),
            port: 1883,
            client_id: "uptrakit-controller".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            connection_status: MqttClientConnectionStatus::Offline,
            status_updated_at: OffsetDateTime::now_utc(),
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        };

        let db = MockDatabase::new(DbBackend::Postgres)
            .append_query_results([[model.clone()]])
            .into_connection();

        let loaded = MqttClient::find_by_id(model.id).one(&db).await;
        let password_none = matches!(loaded, Ok(Some(ref found)) if found.password.is_none());
        assert!(password_none);
    }
}
