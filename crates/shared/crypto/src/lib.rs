//! Encryption at rest for sensitive database fields.
//!
//! Uses AES-256-GCM with envelope encryption: a master key (KEK) wraps
//! data encryption keys (DEKs) stored in the database, and data is encrypted
//! with DEKs — never directly with the KEK.  This enables O(1) master key
//! rotation (re-wrap DEKs only, no data re-encryption).
//!
//! Values are encrypted/decrypted transparently via the [`EncryptedString`]
//! SeaORM custom type.
//!
//! ## Ciphertext formats
//!
//! Three wire formats coexist for backward compatibility:
//!
//! | Format | Prefix | AAD | Key | Used by |
//! |---|---|---|---|---|
//! | v1 | `ENC:v1:` | empty | KEK direct | Legacy (read-only) |
//! | v2 | `ENC:v2:` | caller-supplied | KEK direct | Migration compat, key verification |
//! | v3 | `ENC:v3:<key_id>:` | caller-supplied | DEK (envelope) | Current default |
//!
//! `ENC:v3:` ciphertexts embed the DEK's `key_id` (first 8 hex chars of
//! SHA-256 of the DEK), enabling lookup in the [`DataKeyRing`].  When the
//! ring is not yet initialized (e.g. fresh DB before first DEK is created),
//! `ENC:v2:` is used as a fallback.
//!
//! See `docs/security/secrets-and-encryption.md` for operational details.

pub mod data_key_ring;
pub mod ecies;
pub mod encrypted_string;
mod v1;
mod v2;
mod v3;

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
use zeroize::Zeroizing;

// Re-export public types from submodules.
pub use data_key_ring::{
    DataKey, DataKeyRing, compute_key_id, generate_data_key, unwrap_data_key_with,
    wrap_data_key_with,
};
pub use encrypted_string::EncryptedString;

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
    HexDecode(uptrakit_shared_types::hex::DecodeError),

    #[error("ciphertext too short")]
    CiphertextTooShort,

    #[error("invalid nonce length")]
    InvalidNonce,

    #[error("decrypted value is not valid UTF-8: {0}")]
    InvalidUtf8(std::string::FromUtf8Error),

    #[error("master key does not match existing encrypted data")]
    MasterKeyMismatch,

    #[error("active_key_id is not present in the provided keys map")]
    MissingActiveKey,

    #[error(
        "duplicate column AAD: column '{column}' in table '{new_table}' conflicts with existing AAD '{existing_aad}'"
    )]
    DuplicateColumnAad {
        column: String,
        existing_aad: String,
        new_table: String,
    },
}

pub type Result<T> = std::result::Result<T, Report<CryptoError>>;

impl_report_conversion! {
    uptrakit_shared_types::hex::DecodeError => CryptoError::HexDecode,
    std::string::FromUtf8Error => CryptoError::InvalidUtf8,
}

// ── Global state ─────────────────────────────────────────────────────

/// Global master encryption key (32 bytes for AES-256).
///
/// Wrapped in `Zeroizing` so that the key material is scrubbed from memory
/// if the value is ever dropped (defense-in-depth — `OnceLock` statics have
/// `'static` lifetime and are not normally dropped).
static MASTER_KEY: OnceLock<Zeroizing<[u8; 32]>> = OnceLock::new();

/// When `true`, encryption is disabled and values are stored as plaintext.
///
/// Only set by [`enable_plaintext_mode`] at startup when
/// `--allow-plaintext-secrets` is passed without a master key.
/// Must never be set in production.
static PLAINTEXT_MODE: AtomicBool = AtomicBool::new(false);

/// Global ring of data encryption keys (DEKs) used for envelope encryption.
///
/// When initialized, [`encrypt_str`] produces `ENC:v3:` ciphertexts
/// using the active DEK from this ring.  The KEK (master key) is only used to
/// wrap/unwrap DEKs — never to encrypt data directly.
static DATA_KEY_RING: OnceLock<DataKeyRing> = OnceLock::new();

// ── Ciphertext format prefixes ───────────────────────────────────────

/// Prefix for v1 ciphertexts (empty AAD, used by `EncryptedString` columns).
const ENC_V1_PREFIX: &str = "ENC:v1:";

/// Prefix for v2 ciphertexts (caller-supplied AAD, context-bound).
const ENC_V2_PREFIX: &str = "ENC:v2:";

/// Prefix for v3 ciphertexts (envelope encryption with embedded key_id).
///
/// Full format: `ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>`
/// where `<key_id>` is 8 hex chars identifying the DEK.
const ENC_V3_PREFIX: &str = "ENC:v3:";

// ── Data key ring initialization ─────────────────────────────────────

/// Initialize the global data key ring.
///
/// Must be called once at startup after DEKs have been loaded and unwrapped
/// from the database.  Returns `Err` if the ring has already been initialized.
pub fn init_data_key_ring(ring: DataKeyRing) -> Result<()> {
    let active_key_id = ring.active_key_id().to_string();
    let key_count = ring.len();
    DATA_KEY_RING
        .set(ring)
        .map_err(|_| report!(CryptoError::AlreadyInitialized))?;
    tracing::debug!(active_key_id, key_count, "data key ring initialized");
    Ok(())
}

/// Returns `true` if the data key ring has been initialized.
pub fn data_key_ring_available() -> bool {
    DATA_KEY_RING.get().is_some()
}

// ── Master key management ────────────────────────────────────────────

/// Compute the master key fingerprint: first 16 hex chars of SHA-256(KEK).
///
/// Used to tag DEK rows with the KEK that wrapped them, enabling detection
/// of KEK mismatches during startup.
pub fn master_key_fingerprint() -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    let hash = Sha256::digest(key_bytes.as_slice());
    Ok(uptrakit_shared_types::hex::encode(&hash[..8]))
}

/// Initialize the global master key. Must be called once at startup.
///
/// The key bytes are wrapped in [`Zeroizing`] so that any intermediate copy
/// is scrubbed when dropped.
///
/// Returns `Err` if the key has already been initialized.
pub fn init_master_key(key: Zeroizing<[u8; 32]>) -> Result<()> {
    MASTER_KEY
        .set(key)
        .map_err(|_| report!(CryptoError::AlreadyInitialized))?;
    tracing::debug!("master encryption key initialized");
    Ok(())
}

/// Enable plaintext mode for development use.
///
/// When active, [`encrypt_str`] and [`EncryptedString::new`] store values as
/// plaintext (no encryption). This is safe to call multiple times.
///
/// **Never call this in production.** It is intended solely for development
/// runs started with `--allow-plaintext-secrets` and no master key.
pub fn enable_plaintext_mode() {
    PLAINTEXT_MODE.store(true, Ordering::Release);
}

/// Returns `true` if plaintext mode is enabled (no master key, dev only).
pub fn is_plaintext_mode() -> bool {
    PLAINTEXT_MODE.load(Ordering::Acquire)
}

/// Returns `true` if the master key has been initialized.
pub fn master_key_available() -> bool {
    MASTER_KEY.get().is_some()
}

// ── DEK wrapping convenience functions (use global MASTER_KEY) ───────

/// Wrap (encrypt) a DEK with the current master key (KEK).
///
/// Uses AES-256-GCM with AAD `"uptrakit:dek:<key_id>"` to bind the
/// ciphertext to the specific DEK identity.  Returns the hex-encoded
/// `nonce || ciphertext || tag`.
pub fn wrap_data_key(dek: &DataKey) -> Result<String> {
    let kek = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    let result = wrap_data_key_with(kek, dek)?;
    tracing::debug!(key_id = dek.key_id, "wrapped data encryption key");
    Ok(result)
}

/// Unwrap (decrypt) a DEK using the current master key (KEK).
///
/// The `wrapped_hex` is the hex-encoded `nonce || ciphertext || tag` produced
/// by [`wrap_data_key`].  The `key_id` is used to reconstruct the AAD.
pub fn unwrap_data_key(wrapped_hex: &str, key_id: &str) -> Result<DataKey> {
    let kek = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    let result = unwrap_data_key_with(kek, wrapped_hex, key_id)?;
    tracing::debug!(key_id, "unwrapped data encryption key");
    Ok(result)
}

// ── Column AAD registry ──────────────────────────────────────────────

/// Global registry mapping column names to their AAD strings.
///
/// Used by [`EncryptedString`]'s `TryGetable` implementation to look up the
/// correct AAD for `ENC:v2:`/`ENC:v3:` decryption based on the column name
/// in the query result.
///
/// Initialized at controller startup via [`register_column_aad`].
static COLUMN_AAD_REGISTRY: OnceLock<std::collections::HashMap<String, String>> = OnceLock::new();

/// A column-level AAD registration entry.
///
/// Each entry maps a `(table, column)` pair to an AAD string.
/// The runtime lookup uses only `column` (bare column name) because
/// SeaORM's `TryGetable` does not provide table context. Column
/// names MUST be unique across all encrypted columns; registration
/// fails with [`CryptoError::DuplicateColumnAad`] if a collision is
/// detected.
pub struct ColumnAadEntry {
    /// Table name (e.g., `"ca_certificates"`).
    pub table: &'static str,
    /// Column name (e.g., `"key_pem"`). Must be unique across all tables.
    pub column: &'static str,
    /// The full AAD string (e.g., `"uptrakit:ca_certificates:key_pem"`).
    pub aad: &'static str,
}

/// Register column-name-to-AAD mappings used for `ENC:v2:`/`ENC:v3:`
/// decryption.
///
/// Must be called once at startup, before any database queries that read
/// `EncryptedString` columns. Subsequent calls return an error.
///
/// Each [`ColumnAadEntry`] records a `(table, column, aad)` triple. The
/// runtime lookup key is the bare column name (SeaORM limitation), so
/// column names must be unique across all tables. If a duplicate column
/// name is detected, the function returns
/// [`CryptoError::DuplicateColumnAad`].
///
/// # Errors
///
/// - [`CryptoError::DuplicateColumnAad`] if two entries share a column name.
/// - [`CryptoError::AlreadyInitialized`] if the registry has already been
///   initialized.
pub fn register_column_aad(entries: &[ColumnAadEntry]) -> Result<()> {
    let mut map = HashMap::with_capacity(entries.len());
    for entry in entries {
        if let Some(existing_aad) = map.insert(entry.column.to_string(), entry.aad.to_string()) {
            bail!(CryptoError::DuplicateColumnAad {
                column: entry.column.to_string(),
                existing_aad,
                new_table: entry.table.to_string(),
            });
        }
    }
    COLUMN_AAD_REGISTRY
        .set(map)
        .map_err(|_| report!(CryptoError::AlreadyInitialized))
}

/// Look up the registered AAD for a given column name.
///
/// Returns `None` if the column is not registered (e.g. the registry was not
/// initialized, or the column was not included in the mappings).
#[cfg(any(feature = "sea-orm", test))]
fn column_aad(column_name: &str) -> Option<&str> {
    COLUMN_AAD_REGISTRY
        .get()
        .and_then(|m| m.get(column_name).map(String::as_str))
}

// ── Public encryption / decryption API ───────────────────────────────

/// Check whether a stored string is already encrypted (`ENC:v1:`, `ENC:v2:`,
/// or `ENC:v3:` prefix).
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(ENC_V1_PREFIX) || s.starts_with(ENC_V2_PREFIX) || s.starts_with(ENC_V3_PREFIX)
}

/// Encrypt a plaintext string with caller-supplied AAD.
///
/// Produces `ENC:v3:` (envelope encryption with DEK) when the data key ring
/// is initialized, or `ENC:v2:` (KEK-direct) as fallback.
///
/// The `aad` string is mixed into the GCM authentication tag. A ciphertext
/// encrypted with a given `aad` can only be decrypted with the same `aad`,
/// preventing relocation of the ciphertext to a different context.
///
/// Use a unique, stable, descriptive string for `aad`; for example:
/// `"uptrakit:settings:jwt_signing_key"`.
///
/// In plaintext mode, returns the plaintext unchanged (no encryption).
pub fn encrypt_str(plaintext: &str, aad: &str) -> Result<String> {
    if PLAINTEXT_MODE.load(Ordering::Acquire) {
        return Ok(plaintext.to_string());
    }
    if let Some(ring) = DATA_KEY_RING.get() {
        v3::encrypt_value_v3(ring, plaintext, aad)
    } else {
        let key_bytes = MASTER_KEY
            .get()
            .ok_or_else(|| report!(CryptoError::NotInitialized))?;
        v2::encrypt_value_v2(key_bytes, plaintext, aad)
    }
}

/// Decrypt a stored encrypted string, accepting `ENC:v3:`, `ENC:v2:`, and
/// `ENC:v1:` formats.
///
/// - For `ENC:v3:` ciphertexts: uses the DEK identified by the embedded key_id;
///   the provided `aad` must match the AAD used during encryption.
/// - For `ENC:v2:` ciphertexts: the provided `aad` must match the AAD used
///   during encryption. Decryption fails if the AAD does not match.
/// - For `ENC:v1:` ciphertexts: the `aad` argument is ignored and decryption
///   proceeds with empty AAD (backward compatibility during migration).
/// - Plaintext values: returned as-is when plaintext mode is enabled or when
///   the value has no `ENC:` prefix.
pub fn decrypt_str(stored: &str, aad: &str) -> Result<String> {
    if stored.starts_with(ENC_V3_PREFIX) {
        let ring = DATA_KEY_RING
            .get()
            .ok_or_else(|| report!(CryptoError::NotInitialized))?;
        v3::decrypt_value_v3(ring, stored, aad)
    } else if stored.starts_with(ENC_V2_PREFIX) {
        let key_bytes = MASTER_KEY
            .get()
            .ok_or_else(|| report!(CryptoError::NotInitialized))?;
        v2::decrypt_value_v2(key_bytes, stored, aad)
    } else if stored.starts_with(ENC_V1_PREFIX) {
        // Backward compat: ENC:v1: ciphertexts were produced with empty AAD.
        // Needed during v1->v3 migration.
        let key_bytes = MASTER_KEY
            .get()
            .ok_or_else(|| report!(CryptoError::NotInitialized))?;
        v1::decrypt_value_v1_legacy(key_bytes, stored)
    } else if is_plaintext_mode() || !is_encrypted(stored) {
        Ok(stored.to_string())
    } else {
        bail!(CryptoError::Decryption("unrecognised prefix".into()))
    }
}

// ── Master key verification ──────────────────────────────────────────

/// Sentinel plaintext used to verify master key consistency across HA instances.
const KEY_VERIFICATION_SENTINEL: &str = "uptrakit-master-key-ok-v1";

/// AAD string bound to the key-verification ciphertext.
///
/// Using a dedicated AAD ensures this ciphertext cannot be reused as a valid
/// ciphertext in any other context, even if an attacker obtains the master key
/// and attempts a ciphertext relocation attack.
const KEY_VERIFICATION_AAD: &str = "uptrakit:master-key-verification";

/// Create an encrypted verification token from the sentinel value.
///
/// Uses `ENC:v2:` format with [`KEY_VERIFICATION_AAD`] so the token is
/// context-bound and cannot be repurposed as a different encrypted value.
///
/// The returned string should be stored in the settings table.
/// On subsequent startups, call [`verify_key_verification_token`] with
/// the stored value to ensure the same master key is in use.
pub fn create_key_verification_token() -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    v2::encrypt_value_v2(key_bytes, KEY_VERIFICATION_SENTINEL, KEY_VERIFICATION_AAD)
}

/// Verify a stored key-verification token against the current master key.
///
/// Accepts both `ENC:v1:` (legacy installations, verified with empty AAD)
/// and `ENC:v2:` (current format, verified with [`KEY_VERIFICATION_AAD`]).
///
/// Decrypts the token and checks that the plaintext matches the expected
/// sentinel. Returns `Err(MasterKeyMismatch)` if decryption succeeds but
/// the plaintext differs, or if decryption itself fails (wrong key).
pub fn verify_key_verification_token(stored: &str) -> Result<()> {
    match decrypt_str(stored, KEY_VERIFICATION_AAD) {
        Ok(plaintext) if plaintext == KEY_VERIFICATION_SENTINEL => Ok(()),
        Ok(_) => bail!(CryptoError::MasterKeyMismatch),
        Err(e) => {
            tracing::debug!(error = %e, "key verification decryption failed");
            bail!(CryptoError::MasterKeyMismatch)
        }
    }
}

/// Create an encrypted verification token using an explicit KEK.
///
/// Used during master key rotation to produce a new verification token
/// bound to the new KEK, without overwriting the global master key.
pub fn create_verification_token_with_key(kek: &Zeroizing<[u8; 32]>) -> Result<String> {
    let unbound = UnboundKey::new(&AES_256_GCM, kek.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = KEY_VERIFICATION_SENTINEL.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(
            nonce,
            Aad::from(KEY_VERIFICATION_AAD.as_bytes()),
            &mut in_out,
        )
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!(
        "{ENC_V2_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&output)
    ))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
