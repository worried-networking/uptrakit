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

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
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

/// When `true`, encryption is disabled and values are stored as plaintext.
///
/// Only set by [`enable_plaintext_mode`] at startup when
/// `--allow-plaintext-secrets` is passed without a master key.
/// Must never be set in production.
static PLAINTEXT_MODE: AtomicBool = AtomicBool::new(false);

// ── Data Key Ring (envelope encryption) ──────────────────────────────

/// Global ring of data encryption keys (DEKs) used for envelope encryption.
///
/// When initialized, [`encrypt_str_with_aad`] produces `ENC:v3:` ciphertexts
/// using the active DEK from this ring.  The KEK (master key) is only used to
/// wrap/unwrap DEKs — never to encrypt data directly.
static DATA_KEY_RING: OnceLock<DataKeyRing> = OnceLock::new();

/// A data encryption key (DEK) with its key ID.
///
/// The `key_id` is the first 8 hex characters of the SHA-256 hash of the raw
/// DEK bytes, providing a stable, non-secret identifier that can be embedded
/// in ciphertext prefixes to select the correct DEK for decryption.
pub struct DataKey {
    /// First 8 hex chars of SHA-256(key).
    pub key_id: String,
    /// Raw 32-byte AES-256 key material.
    pub key: Zeroizing<[u8; 32]>,
}

/// A ring of data encryption keys for envelope encryption.
///
/// Holds all known DEKs (indexed by `key_id`) and tracks which one is
/// currently active for new encryptions.  Retired DEKs remain in the ring
/// for decryption of existing ciphertext.
pub struct DataKeyRing {
    keys: HashMap<String, Zeroizing<[u8; 32]>>,
    active_key_id: String,
}

impl DataKeyRing {
    /// Construct a new key ring.
    ///
    /// # Arguments
    ///
    /// * `keys` — Map of key_id → raw DEK bytes.
    /// * `active_key_id` — The key_id of the DEK to use for new encryptions.
    ///
    /// # Panics
    ///
    /// Panics if `active_key_id` is not present in `keys`.
    pub fn new(keys: HashMap<String, Zeroizing<[u8; 32]>>, active_key_id: String) -> Self {
        assert!(
            keys.contains_key(&active_key_id),
            "active_key_id must be present in keys"
        );
        Self {
            keys,
            active_key_id,
        }
    }

    /// Returns the key_id of the active DEK.
    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    /// Returns the raw key bytes for the active DEK.
    fn active_key(&self) -> &Zeroizing<[u8; 32]> {
        self.keys
            .get(&self.active_key_id)
            .expect("active key must exist in ring")
    }

    /// Look up a DEK by its key_id.
    fn get(&self, key_id: &str) -> Option<&Zeroizing<[u8; 32]>> {
        self.keys.get(key_id)
    }
}

/// Initialize the global data key ring.
///
/// Must be called once at startup after DEKs have been loaded and unwrapped
/// from the database.  Returns `Err` if the ring has already been initialized.
pub fn init_data_key_ring(ring: DataKeyRing) -> Result<()> {
    DATA_KEY_RING
        .set(ring)
        .map_err(|_| report!(CryptoError::AlreadyInitialized))
}

/// Returns `true` if the data key ring has been initialized.
pub fn data_key_ring_available() -> bool {
    DATA_KEY_RING.get().is_some()
}

/// Compute the key_id for a raw DEK: first 8 hex chars of SHA-256(dek).
pub fn compute_key_id(dek_bytes: &[u8; 32]) -> String {
    let hash = Sha256::digest(dek_bytes);
    uptrakit_shared_types::hex::encode(&hash[..4])
}

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

/// Generate a new random 32-byte DEK, returning its key_id and key material.
pub fn generate_data_key() -> Result<DataKey> {
    let mut key_bytes = Zeroizing::new([0u8; 32]);
    rand::rng().fill_bytes(key_bytes.as_mut_slice());
    let key_id = compute_key_id(&key_bytes);
    Ok(DataKey {
        key_id,
        key: key_bytes,
    })
}

/// Wrap (encrypt) a DEK with the current master key (KEK).
///
/// Uses AES-256-GCM with AAD `"uptrakit:dek:<key_id>"` to bind the
/// ciphertext to the specific DEK identity.  Returns the hex-encoded
/// `nonce || ciphertext || tag`.
pub fn wrap_data_key(dek: &DataKey) -> Result<String> {
    let kek = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    wrap_data_key_with(kek, dek)
}

/// Wrap (encrypt) a DEK with an explicit KEK.
///
/// This variant is used during key rotation to wrap DEKs with the new KEK.
pub fn wrap_data_key_with(kek: &Zeroizing<[u8; 32]>, dek: &DataKey) -> Result<String> {
    let aad_str = format!("uptrakit:dek:{}", dek.key_id);

    let unbound = UnboundKey::new(&AES_256_GCM, kek.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = dek.key.as_slice().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::from(aad_str.as_bytes()), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(uptrakit_shared_types::hex::encode(&output))
}

/// Unwrap (decrypt) a DEK using the current master key (KEK).
///
/// The `wrapped_hex` is the hex-encoded `nonce || ciphertext || tag` produced
/// by [`wrap_data_key`].  The `key_id` is used to reconstruct the AAD.
pub fn unwrap_data_key(wrapped_hex: &str, key_id: &str) -> Result<DataKey> {
    let kek = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;
    unwrap_data_key_with(kek, wrapped_hex, key_id)
}

/// Unwrap (decrypt) a DEK using an explicit KEK.
///
/// This variant is used during key rotation to verify that DEKs can be
/// unwrapped with both old and new KEKs.
pub fn unwrap_data_key_with(
    kek: &Zeroizing<[u8; 32]>,
    wrapped_hex: &str,
    key_id: &str,
) -> Result<DataKey> {
    let aad_str = format!("uptrakit:dek:{key_id}");

    let raw = uptrakit_shared_types::hex::decode(wrapped_hex).context_to()?;

    if raw.len() < 12 + 16 {
        bail!(CryptoError::CiphertextTooShort);
    }

    let nonce_bytes: [u8; 12] = raw[..12]
        .try_into()
        .map_err(|_| report!(CryptoError::InvalidNonce))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let unbound = UnboundKey::new(&AES_256_GCM, kek.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut ciphertext_and_tag = raw[12..].to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::from(aad_str.as_bytes()), &mut ciphertext_and_tag)
        .map_err(|_| {
            report!(CryptoError::Decryption(
                "DEK unwrap failed: wrong KEK or tampered data".into()
            ))
        })?;

    if plaintext.len() != 32 {
        bail!(CryptoError::Decryption(format!(
            "unwrapped DEK has invalid length: {} (expected 32)",
            plaintext.len()
        )));
    }

    let mut dek_bytes = Zeroizing::new([0u8; 32]);
    dek_bytes.copy_from_slice(plaintext);

    // Verify that the computed key_id matches the expected one.
    let computed_id = compute_key_id(&dek_bytes);
    if computed_id != key_id {
        bail!(CryptoError::Decryption(format!(
            "DEK key_id mismatch: expected {key_id}, computed {computed_id}"
        )));
    }

    Ok(DataKey {
        key_id: key_id.to_string(),
        key: dek_bytes,
    })
}

// ── Column AAD registry ──────────────────────────────────────────────

/// Global registry mapping column names to their AAD strings.
///
/// Used by [`EncryptedString`]'s `TryGetable` implementation to look up the
/// correct AAD for `ENC:v2:` decryption based on the column name in the
/// query result.
///
/// Initialized at controller startup via [`register_column_aad`].
static COLUMN_AAD_REGISTRY: OnceLock<std::collections::HashMap<String, String>> = OnceLock::new();

/// Register the column-name-to-AAD mappings used for `ENC:v2:` decryption.
///
/// Must be called once at startup, before any database queries that read
/// `EncryptedString` columns. Subsequent calls return an error.
///
/// The map keys are column names (e.g. `"password"`, `"client_secret"`) and
/// values are the corresponding AAD strings (e.g. `"uptrakit:mqtt_clients:password"`).
///
/// # Errors
///
/// Returns `Err(CryptoError::AlreadyInitialized)` if the registry has already
/// been initialized.
pub fn register_column_aad(
    mappings: std::collections::HashMap<String, String>,
) -> Result<()> {
    COLUMN_AAD_REGISTRY
        .set(mappings)
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

/// Check whether a stored string is already encrypted (`ENC:v1:`, `ENC:v2:`,
/// or `ENC:v3:` prefix).
pub fn is_encrypted(s: &str) -> bool {
    s.starts_with(ENC_V1_PREFIX) || s.starts_with(ENC_V2_PREFIX) || s.starts_with(ENC_V3_PREFIX)
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
    encrypt_value_v2(KEY_VERIFICATION_SENTINEL, KEY_VERIFICATION_AAD)
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

// ── Public encryption API ────────────────────────────────────────────

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
    if data_key_ring_available() {
        encrypt_value_v3(plaintext, aad)
    } else {
        encrypt_value_v2(plaintext, aad)
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
        decrypt_value_v3(stored, aad)
    } else if stored.starts_with(ENC_V2_PREFIX) {
        decrypt_value_v2(stored, aad)
    } else if stored.starts_with(ENC_V1_PREFIX) {
        // Backward compat: ENC:v1: ciphertexts were produced with empty AAD.
        // Needed during v1→v3 migration.
        decrypt_value_v1_legacy(stored)
    } else if is_plaintext_mode() || !is_encrypted(stored) {
        Ok(stored.to_string())
    } else {
        bail!(CryptoError::Decryption("unrecognised prefix".into()))
    }
}

// ── Internal v1 legacy implementation (empty AAD, read-only) ─────────
//
// The v1 encrypt function is only used in tests. The v1 decrypt function is
// needed by the migration path (decrypt_str handles v1 → delegate here) and
// by the `verify_key_verification_token` backward-compat branch.

#[cfg(test)]
fn encrypt_value_v1(plaintext: &str) -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!(
        "{ENC_V1_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&output)
    ))
}

fn decrypt_value_v1_legacy(stored: &str) -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let hex_part = stored
        .strip_prefix(ENC_V1_PREFIX)
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

// ── Internal v2 implementation (caller-supplied AAD) ─────────────────

fn encrypt_value_v2(plaintext: &str, aad: &str) -> Result<String> {
    if PLAINTEXT_MODE.load(Ordering::Acquire) {
        return Ok(plaintext.to_string());
    }

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

    // Encrypt in-place with caller-supplied AAD
    let mut in_out = plaintext.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::from(aad.as_bytes()), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    // Build output: nonce || ciphertext || tag
    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!(
        "{ENC_V2_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&output)
    ))
}

fn decrypt_value_v2(stored: &str, aad: &str) -> Result<String> {
    let key_bytes = MASTER_KEY
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let hex_part = stored
        .strip_prefix(ENC_V2_PREFIX)
        .ok_or_else(|| report!(CryptoError::Decryption("missing ENC:v2: prefix".into())))?;

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
        .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut ciphertext_and_tag)
        .map_err(|_| {
            report!(CryptoError::Decryption(
                "wrong key, wrong AAD, or tampered data".into()
            ))
        })?;

    String::from_utf8(plaintext.to_vec()).context_to()
}

// ── Internal v3 implementation (envelope encryption with DEK) ────────

/// Encrypt with the active DEK from the data key ring.
///
/// Format: `ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>`
fn encrypt_value_v3(plaintext: &str, aad: &str) -> Result<String> {
    let ring = DATA_KEY_RING
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let dek = ring.active_key();
    let key_id = ring.active_key_id();

    let unbound = UnboundKey::new(&AES_256_GCM, dek.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.as_bytes().to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, Aad::from(aad.as_bytes()), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(e.to_string())))?;

    let mut output = Vec::with_capacity(12 + in_out.len() + tag.as_ref().len());
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&in_out);
    output.extend_from_slice(tag.as_ref());

    Ok(format!(
        "{ENC_V3_PREFIX}{key_id}:{}",
        uptrakit_shared_types::hex::encode(&output)
    ))
}

/// Decrypt a `ENC:v3:<key_id>:<hex>` ciphertext using the data key ring.
fn decrypt_value_v3(stored: &str, aad: &str) -> Result<String> {
    let ring = DATA_KEY_RING
        .get()
        .ok_or_else(|| report!(CryptoError::NotInitialized))?;

    let after_prefix = stored
        .strip_prefix(ENC_V3_PREFIX)
        .ok_or_else(|| report!(CryptoError::Decryption("missing ENC:v3: prefix".into())))?;

    // Parse key_id (8 hex chars) and hex payload separated by ':'
    let colon_pos = after_prefix
        .find(':')
        .ok_or_else(|| report!(CryptoError::Decryption("missing key_id separator in ENC:v3".into())))?;

    let key_id = &after_prefix[..colon_pos];
    let hex_part = &after_prefix[colon_pos + 1..];

    let dek = ring.get(key_id).ok_or_else(|| {
        report!(CryptoError::Decryption(format!(
            "unknown DEK key_id: {key_id}"
        )))
    })?;

    let raw = uptrakit_shared_types::hex::decode(hex_part).context_to()?;

    if raw.len() < 12 + 16 {
        bail!(CryptoError::CiphertextTooShort);
    }

    let nonce_bytes: [u8; 12] = raw[..12]
        .try_into()
        .map_err(|_| report!(CryptoError::InvalidNonce))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let unbound = UnboundKey::new(&AES_256_GCM, dek.as_slice())
        .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
    let key = LessSafeKey::new(unbound);

    let mut ciphertext_and_tag = raw[12..].to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::from(aad.as_bytes()), &mut ciphertext_and_tag)
        .map_err(|_| {
            report!(CryptoError::Decryption(
                "wrong DEK, wrong AAD, or tampered data".into()
            ))
        })?;

    String::from_utf8(plaintext.to_vec()).context_to()
}

/// A string that is transparently encrypted when written to the database
/// and decrypted when read back.
///
/// Encryption is performed eagerly at construction time via [`EncryptedString::new`].
/// The pre-computed database representation is stored alongside the plaintext
/// so that `From<EncryptedString> for sea_orm::Value` is infallible.
///
/// Construction **requires** the master key (and ideally the data key ring)
/// to be initialized. If neither is available, [`EncryptedString::new`]
/// returns `Err(CryptoError::NotInitialized)`. There is no plaintext
/// fallback — a missing key is always treated as a hard error to prevent
/// silent secret exposure in misconfigured deployments.
///
/// ## Ciphertext format
///
/// - [`EncryptedString::new`] produces `ENC:v3:` (envelope encryption with
///   DEK + caller-supplied AAD) when the data key ring is initialized, or
///   `ENC:v2:` (KEK-direct with AAD) as fallback.
/// - All three formats (`ENC:v1:`, `ENC:v2:`, `ENC:v3:`) are transparently
///   handled on the read path by the `TryGetable` implementation.
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
    /// Create a new `EncryptedString` from a plaintext value with
    /// context-bound AAD.
    ///
    /// Produces `ENC:v3:` ciphertext (envelope encryption with DEK) when the
    /// data key ring is initialized, or `ENC:v2:` as fallback (KEK-direct).
    ///
    /// The `aad` string is mixed into the GCM authentication tag, binding
    /// the ciphertext to a specific column/purpose.  Use the
    /// `"uptrakit:<table>:<column>"` convention.
    ///
    /// # Errors
    ///
    /// Returns `Err(CryptoError::NotInitialized)` if the master key has not
    /// been initialized, or `Err` on any other encryption failure.
    pub fn new(plaintext: String, aad: &str) -> Result<Self> {
        let db_value = encrypt_str(&plaintext, aad)?;
        Ok(Self {
            plaintext: SecretString::new(plaintext),
            db_value,
        })
    }

    /// Construct from a decrypted DB value on the read path.
    ///
    /// Used by `ValueType` / `TryGetable` impls to construct from a decrypted
    /// value while preserving the original DB representation.
    #[cfg(feature = "sea-orm")]
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

    /// Returns `true` if the stored DB value is not v3 format.
    ///
    /// Used by the re-encryption routine to identify v1/v2/plaintext values
    /// that should be upgraded to `ENC:v3:` format.
    pub fn needs_v3_upgrade(&self) -> bool {
        !self.db_value.starts_with(ENC_V3_PREFIX)
    }

    /// Construct an `EncryptedString` whose database representation is the
    /// raw `value` string **without** the `ENC:v1:` prefix.
    ///
    /// This is used exclusively in tests to simulate legacy rows that were
    /// written to the database before encryption was added, allowing the
    /// re-encryption routine to be tested without raw SQL `UPDATE` statements.
    ///
    /// **Never call this in production code.**
    #[cfg(any(test, feature = "testing"))]
    pub fn plaintext_for_test(value: String) -> Self {
        Self {
            plaintext: SecretString::new(value.clone()),
            db_value: value,
        }
    }

    /// Returns `true` if the stored DB representation is already encrypted.
    ///
    /// Used by the re-encryption routine to identify legacy plaintext values
    /// that need to be re-encrypted with the current master key.
    pub fn is_db_value_encrypted(&self) -> bool {
        is_encrypted(&self.db_value)
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

#[cfg(feature = "sea-orm")]
impl sea_orm::sea_query::ValueType for EncryptedString {
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::Value::String(Some(s)) => {
                if s.starts_with(ENC_V3_PREFIX) {
                    // ValueType has no column name — best-effort with empty AAD.
                    // Normal SeaORM entity queries go through TryGetable which has
                    // the column name and can look up the correct AAD.
                    let plaintext =
                        decrypt_value_v3(&s, "").map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(EncryptedString::from_db(plaintext, s))
                } else if s.starts_with(ENC_V1_PREFIX) {
                    let plaintext =
                        decrypt_value_v1_legacy(&s).map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
                    Ok(EncryptedString::from_db(plaintext, s))
                } else if s.starts_with(ENC_V2_PREFIX) {
                    // ValueType has no column name — use empty AAD as fallback.
                    let plaintext =
                        decrypt_value_v2(&s, "").map_err(|_| sea_orm::sea_query::ValueTypeErr)?;
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

#[cfg(feature = "sea-orm")]
impl sea_orm::sea_query::Nullable for EncryptedString {
    fn null() -> sea_orm::Value {
        sea_orm::Value::String(None)
    }
}

#[cfg(feature = "sea-orm")]
impl From<EncryptedString> for sea_orm::Value {
    fn from(val: EncryptedString) -> Self {
        sea_orm::Value::String(Some(val.db_value))
    }
}

#[cfg(feature = "sea-orm")]
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

        if s.starts_with(ENC_V3_PREFIX) {
            let col_name = index.as_str().unwrap_or("unknown");
            let aad = column_aad(col_name).unwrap_or("");
            let plaintext = decrypt_value_v3(&s, aad).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "ENC:v3 decryption failed for column '{col_name}': {e}"
                )))
            })?;
            Ok(EncryptedString::from_db(plaintext, s))
        } else if s.starts_with(ENC_V2_PREFIX) {
            let col_name = index.as_str().unwrap_or("unknown");
            let aad = column_aad(col_name).unwrap_or("");
            let plaintext = decrypt_value_v2(&s, aad).map_err(|e| {
                sea_orm::TryGetError::DbErr(sea_orm::DbErr::Type(format!(
                    "ENC:v2 decryption failed for column '{col_name}': {e}"
                )))
            })?;
            Ok(EncryptedString::from_db(plaintext, s))
        } else if s.starts_with(ENC_V1_PREFIX) {
            let plaintext = decrypt_value_v1_legacy(&s).map_err(|e| {
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
    fn test_v1_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        for plaintext in [
            "hello world",
            "",
            "\u{1f980} Rust",
            "a".repeat(10_000).as_str(),
        ] {
            let encrypted = encrypt_value_v1(plaintext).expect("encryption should succeed");
            assert!(encrypted.starts_with(ENC_V1_PREFIX));
            let decrypted =
                decrypt_value_v1_legacy(&encrypted).expect("decryption should succeed");
            assert_eq!(decrypted, plaintext);
        }
    }

    #[test]
    fn test_v1_nonce_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let enc1 = encrypt_value_v1("same input").expect("encryption should succeed");
        let enc2 = encrypt_value_v1("same input").expect("encryption should succeed");
        assert_ne!(
            enc1, enc2,
            "two encryptions of the same value should produce different ciphertext"
        );

        // Both should still decrypt to the same value
        assert_eq!(
            decrypt_value_v1_legacy(&enc1).expect("decryption should succeed"),
            "same input"
        );
        assert_eq!(
            decrypt_value_v1_legacy(&enc2).expect("decryption should succeed"),
            "same input"
        );
    }

    #[test]
    fn test_is_encrypted_detection() {
        assert!(is_encrypted("ENC:v1:aabbcc"));
        assert!(is_encrypted("ENC:v2:aabbcc"));
        assert!(is_encrypted("ENC:v3:aabbcc"));
        assert!(!is_encrypted("plaintext"));
        assert!(!is_encrypted("ENC:v4:aabbcc"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn test_debug_display_redact() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let s = EncryptedString::new("my secret".to_string(), "test-aad").expect("test key set");
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
    fn test_v1_tampered_ciphertext_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let encrypted = encrypt_value_v1("sensitive data").expect("encryption should succeed");
        // Tamper with one byte in the hex payload
        let hex_part = encrypted
            .strip_prefix(ENC_V1_PREFIX)
            .expect("should have prefix");
        let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!("{ENC_V1_PREFIX}{}", uptrakit_shared_types::hex::encode(&raw));
        assert!(decrypt_value_v1_legacy(&tampered).is_err());
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn test_seaorm_value_roundtrip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // ValueType::try_from uses best-effort empty AAD for v3 ciphertexts,
        // so encrypt with empty AAD to make the round-trip work.
        let original =
            EncryptedString::new("database secret".to_string(), "").expect("test key set");
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

        let es = EncryptedString::new("secret".to_string(), "test-aad").expect("test key set");
        assert!(
            is_encrypted(&es.db_value),
            "db_value should be encrypted when master key is available"
        );
        assert_eq!(es.expose_secret(), "secret");
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn test_from_impl_uses_precomputed_value() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es = EncryptedString::new("precomputed".to_string(), "test-aad").expect("test key set");
        let precomputed = es.db_value.clone();
        let value: sea_orm::Value = es.into();

        // The Value must contain exactly the pre-computed db_value
        if let sea_orm::Value::String(Some(s)) = value {
            assert_eq!(s, precomputed);
        } else {
            panic!("Expected String value");
        }
    }

    #[cfg(feature = "sea-orm")]
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
        // Verification token must now be ENC:v2:
        assert!(
            token.starts_with(ENC_V2_PREFIX),
            "key verification token should use ENC:v2: format"
        );
        assert!(is_encrypted(&token));
        assert!(verify_key_verification_token(&token).is_ok());
    }

    #[test]
    fn test_key_verification_accepts_legacy_v1() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Simulate a legacy ENC:v1: token (from an installation before this change).
        // ENC:v1: tokens are decrypted with empty AAD — the sentinel must match.
        let legacy_token = encrypt_value_v1(KEY_VERIFICATION_SENTINEL)
            .expect("should encrypt sentinel with v1");
        assert!(legacy_token.starts_with(ENC_V1_PREFIX));
        assert!(
            verify_key_verification_token(&legacy_token).is_ok(),
            "verify must accept legacy ENC:v1: tokens for backward compatibility"
        );
    }

    #[test]
    fn test_key_verification_rejects_tampered_token() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let token = create_key_verification_token().expect("should create token");
        // Tamper with the ciphertext
        let hex_part = token.strip_prefix(ENC_V2_PREFIX).expect("has v2 prefix");
        let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!("{ENC_V2_PREFIX}{}", uptrakit_shared_types::hex::encode(&raw));
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

    #[test]
    fn test_init_master_key_already_initialized() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Calling init_master_key again should return AlreadyInitialized.
        let key = Zeroizing::new([0xABu8; 32]);
        let result = init_master_key(key);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CryptoError::AlreadyInitialized
        ));
    }

    #[test]
    fn test_decrypt_v1_ciphertext_too_short() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Construct a value with valid prefix but ciphertext shorter than nonce + tag (28 bytes).
        let short_bytes = [0u8; 10];
        let short_hex = uptrakit_shared_types::hex::encode(short_bytes);
        let stored = format!("{ENC_V1_PREFIX}{short_hex}");
        let result = decrypt_value_v1_legacy(&stored);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CryptoError::CiphertextTooShort
        ));
    }

    #[test]
    fn test_decrypt_v1_missing_prefix() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Attempt to decrypt a string without the ENC:v1: prefix.
        let result = decrypt_value_v1_legacy("not-encrypted-data");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CryptoError::Decryption(_)
        ));
    }

    #[test]
    fn test_decrypt_v1_invalid_hex() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let result = decrypt_value_v1_legacy(&format!("{ENC_V1_PREFIX}not-valid-hex!@#$"));
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().current_context(),
            CryptoError::HexDecode(_)
        ));
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn test_seaorm_value_type_non_string_fails() {
        // Passing a non-String Value should return ValueTypeErr.
        let value = sea_orm::Value::Int(Some(42));
        let result = <EncryptedString as sea_orm::sea_query::ValueType>::try_from(value);
        assert!(result.is_err());
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn test_seaorm_nullable_returns_none() {
        let null = <EncryptedString as sea_orm::sea_query::Nullable>::null();
        assert!(matches!(null, sea_orm::Value::String(None)));
    }

    #[test]
    fn test_encrypted_string_clone_and_eq() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es1 = EncryptedString::new("test value".to_string(), "test-aad").expect("should encrypt");
        let es2 = es1.clone();

        // Clone should produce equal values (PartialEq compares plaintext only).
        assert_eq!(es1, es2);
        assert_eq!(es1.expose_secret(), es2.expose_secret());
    }

    #[test]
    fn test_encrypted_string_inequality() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es1 =
            EncryptedString::new("value_a".to_string(), "test-aad").expect("should encrypt");
        let es2 =
            EncryptedString::new("value_b".to_string(), "test-aad").expect("should encrypt");

        assert_ne!(es1, es2);
    }

    #[test]
    fn test_plaintext_mode_encrypt_returns_plaintext() {
        let _lock = TEST_LOCK.lock().unwrap();

        // Save and set plaintext mode; restore on exit to avoid affecting other tests.
        let was_plaintext = PLAINTEXT_MODE.load(Ordering::Acquire);
        PLAINTEXT_MODE.store(true, Ordering::Release);

        let result = encrypt_str("dev secret", "test-aad");
        PLAINTEXT_MODE.store(was_plaintext, Ordering::Release);

        let encrypted = result.expect("plaintext mode encrypt should succeed");
        assert_eq!(
            encrypted, "dev secret",
            "in plaintext mode, encrypt_str should return the value unchanged"
        );
        assert!(
            !is_encrypted(&encrypted),
            "plaintext mode output must not carry an ENC: prefix"
        );
    }

    #[test]
    fn test_plaintext_mode_encrypted_string_new() {
        let _lock = TEST_LOCK.lock().unwrap();

        let was_plaintext = PLAINTEXT_MODE.load(Ordering::Acquire);
        PLAINTEXT_MODE.store(true, Ordering::Release);

        let es = EncryptedString::new("plain dev value".to_string(), "test-aad");
        PLAINTEXT_MODE.store(was_plaintext, Ordering::Release);

        let es = es.expect("EncryptedString::new should succeed in plaintext mode");
        assert_eq!(es.expose_secret(), "plain dev value");
        assert!(
            !es.is_db_value_encrypted(),
            "db_value should not be encrypted in plaintext mode"
        );
    }

    // ── encrypt_str / decrypt_str (AAD-based) tests ──────────────────

    #[test]
    fn test_encrypt_decrypt_str_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let aad = "uptrakit:test:context";
        for plaintext in ["hello", "", "multi\nline\nvalue", "a".repeat(1_000).as_str()] {
            let encrypted =
                encrypt_str(plaintext, aad).expect("encryption should succeed");
            assert!(is_encrypted(&encrypted));
            let decrypted =
                decrypt_str(&encrypted, aad).expect("decryption should succeed");
            assert_eq!(decrypted, plaintext);
        }
    }

    #[test]
    fn test_encrypt_decrypt_str_wrong_aad_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let encrypted =
            encrypt_str("secret", "correct-aad").expect("encryption should succeed");
        let result = decrypt_str(&encrypted, "wrong-aad");
        assert!(
            result.is_err(),
            "decrypting with wrong AAD must fail"
        );
        assert!(matches!(
            result.unwrap_err().current_context(),
            CryptoError::Decryption(_)
        ));
    }

    #[test]
    fn test_encrypt_str_nonce_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let aad = "uptrakit:test:nonce";
        let enc1 = encrypt_str("same", aad).expect("should encrypt");
        let enc2 = encrypt_str("same", aad).expect("should encrypt");
        assert_ne!(enc1, enc2, "encryptions of the same value must differ (random nonces)");
    }

    #[test]
    fn test_decrypt_str_accepts_v1_fallback() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // An ENC:v1: ciphertext should be accepted by decrypt_str
        // regardless of the provided aad (backward compat).
        let v1_encrypted = encrypt_value_v1("legacy_value").expect("v1 encryption should succeed");
        assert!(v1_encrypted.starts_with(ENC_V1_PREFIX));
        let result = decrypt_str(&v1_encrypted, "any-aad-is-ignored-for-v1");
        assert!(
            result.is_ok(),
            "decrypt_str must accept ENC:v1: tokens for backward compatibility"
        );
        assert_eq!(result.unwrap(), "legacy_value");
    }

    #[test]
    fn test_encrypted_string_new_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let aad = "uptrakit:test_table:test_column";
        let es = EncryptedString::new("aad secret".to_string(), aad)
            .expect("new should succeed");
        // Produces v3 when ring is available, v2 otherwise
        assert!(
            es.db_value.starts_with(ENC_V2_PREFIX) || es.db_value.starts_with(ENC_V3_PREFIX),
            "new must produce ENC:v2: or ENC:v3: ciphertext"
        );
        assert_eq!(es.expose_secret(), "aad secret");

        // Must decrypt with the same AAD (decrypt_str handles both v2 and v3)
        let decrypted =
            decrypt_str(&es.db_value, aad).expect("decryption with correct AAD");
        assert_eq!(decrypted, "aad secret");
    }

    #[test]
    fn test_encrypted_string_new_wrong_aad_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let es = EncryptedString::new("secret".to_string(), "correct:aad")
            .expect("should encrypt");
        // Try decrypting with wrong AAD — should fail regardless of v2/v3 format
        let result = decrypt_str(&es.db_value, "wrong:aad");
        assert!(
            result.is_err(),
            "decrypting ciphertext with wrong AAD must fail"
        );
    }

    #[test]
    fn test_needs_v3_upgrade_for_v1() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Create a v1 ciphertext and wrap it in EncryptedString via plaintext_for_test
        // (which stores the value directly as db_value — works for any format).
        let v1_ciphertext = encrypt_value_v1("v1 value").expect("should encrypt");
        let es = EncryptedString::plaintext_for_test(v1_ciphertext);
        assert!(
            es.needs_v3_upgrade(),
            "v1 ciphertext needs v3 upgrade"
        );
    }

    #[test]
    fn test_needs_v3_upgrade_for_v2() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let v2_ciphertext =
            encrypt_value_v2("v2 value", "test:aad").expect("should encrypt");
        let es = EncryptedString::plaintext_for_test(v2_ciphertext);
        assert!(
            es.needs_v3_upgrade(),
            "v2 ciphertext needs v3 upgrade"
        );
    }

    #[test]
    fn test_needs_v3_upgrade_false_for_v3() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let v3_ciphertext =
            encrypt_value_v3("v3 value", "test:aad").expect("should encrypt");
        let es = EncryptedString::plaintext_for_test(v3_ciphertext);
        assert!(
            !es.needs_v3_upgrade(),
            "v3 ciphertext does not need upgrade"
        );
    }

    #[test]
    fn test_needs_v3_upgrade_true_for_plaintext() {
        let es = EncryptedString::plaintext_for_test("plain".to_string());
        assert!(
            es.needs_v3_upgrade(),
            "plaintext db_value needs v3 upgrade (encryption)"
        );
    }

    #[test]
    fn test_cross_column_relocation_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        // Encrypt with one AAD context (simulating column A)
        let es = EncryptedString::new(
            "sensitive".to_string(),
            "uptrakit:table_a:column_a",
        )
        .expect("should encrypt");

        // Attempt to decrypt with a different AAD (simulating column B) — must fail
        let result = decrypt_str(&es.db_value, "uptrakit:table_b:column_b");
        assert!(
            result.is_err(),
            "ciphertext decrypted with wrong AAD (different column) must fail"
        );
    }

    #[test]
    fn test_register_column_aad_and_lookup() {
        // Note: this test relies on the fact that COLUMN_AAD_REGISTRY is a
        // process-wide OnceLock. Since other tests may call register_column_aad
        // too, we just verify the lookup function works with whatever is
        // registered (or not).
        let mut mappings = std::collections::HashMap::new();
        mappings.insert("test_col".to_string(), "uptrakit:t:test_col".to_string());
        // Ignore error — may already be initialized by another test
        let _ = register_column_aad(mappings);

        // If registry was initialized by us, lookup should work.
        // If already initialized by another test, we just verify no panic.
        let _ = column_aad("test_col");
        assert!(column_aad("nonexistent_col_xyz").is_none());
    }

    #[test]
    fn test_encrypt_str_plaintext_mode_returns_plaintext() {
        let _lock = TEST_LOCK.lock().unwrap();

        let was_plaintext = PLAINTEXT_MODE.load(Ordering::Acquire);
        PLAINTEXT_MODE.store(true, Ordering::Release);

        let result = encrypt_str("dev secret", "some-aad");
        PLAINTEXT_MODE.store(was_plaintext, Ordering::Release);

        let value = result.expect("plaintext mode should succeed");
        assert_eq!(value, "dev secret");
        assert!(!is_encrypted(&value));
    }

    // ── DEK / envelope encryption tests ─────────────────────────────

    #[test]
    fn test_compute_key_id_deterministic() {
        let dek = [0x42u8; 32];
        let id1 = compute_key_id(&dek);
        let id2 = compute_key_id(&dek);
        assert_eq!(id1, id2, "compute_key_id must be deterministic");
        assert_eq!(id1.len(), 8, "key_id must be 8 hex chars");
    }

    #[test]
    fn test_compute_key_id_different_keys() {
        let dek1 = [0x42u8; 32];
        let dek2 = [0x43u8; 32];
        assert_ne!(
            compute_key_id(&dek1),
            compute_key_id(&dek2),
            "different DEKs must produce different key_ids"
        );
    }

    #[test]
    fn test_dek_wrap_unwrap_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let dek = generate_data_key().expect("should generate DEK");
        assert_eq!(dek.key_id.len(), 8);

        let wrapped = wrap_data_key(&dek).expect("should wrap DEK");
        let unwrapped = unwrap_data_key(&wrapped, &dek.key_id).expect("should unwrap DEK");

        assert_eq!(unwrapped.key_id, dek.key_id);
        assert_eq!(unwrapped.key.as_slice(), dek.key.as_slice());
    }

    #[test]
    fn test_dek_wrap_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let dek = generate_data_key().expect("should generate DEK");
        let w1 = wrap_data_key(&dek).expect("should wrap");
        let w2 = wrap_data_key(&dek).expect("should wrap");
        assert_ne!(
            w1, w2,
            "two wrappings of the same DEK must differ (random nonces)"
        );
    }

    #[test]
    fn test_dek_unwrap_wrong_key_id_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let dek = generate_data_key().expect("should generate DEK");
        let wrapped = wrap_data_key(&dek).expect("should wrap DEK");

        let result = unwrap_data_key(&wrapped, "deadbeef");
        assert!(
            result.is_err(),
            "unwrapping with wrong key_id must fail (AAD mismatch)"
        );
    }

    #[test]
    fn test_dek_unwrap_tampered_data_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let dek = generate_data_key().expect("should generate DEK");
        let wrapped = wrap_data_key(&dek).expect("should wrap DEK");

        let mut raw = uptrakit_shared_types::hex::decode(&wrapped).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = uptrakit_shared_types::hex::encode(&raw);

        let result = unwrap_data_key(&tampered, &dek.key_id);
        assert!(result.is_err(), "tampered wrapped DEK must fail");
    }

    #[test]
    fn test_wrap_data_key_with_explicit_kek() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let new_kek = Zeroizing::new([0xABu8; 32]);
        let dek = generate_data_key().expect("should generate DEK");

        let wrapped = wrap_data_key_with(&new_kek, &dek).expect("should wrap with explicit KEK");
        let unwrapped =
            unwrap_data_key_with(&new_kek, &wrapped, &dek.key_id).expect("should unwrap");

        assert_eq!(unwrapped.key.as_slice(), dek.key.as_slice());

        // Should fail with the original KEK
        let result = unwrap_data_key(&wrapped, &dek.key_id);
        assert!(
            result.is_err(),
            "unwrapping with wrong KEK must fail"
        );
    }

    #[test]
    fn test_master_key_fingerprint() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let fp = master_key_fingerprint().expect("should compute fingerprint");
        assert_eq!(fp.len(), 16, "fingerprint must be 16 hex chars");

        // Must be deterministic
        let fp2 = master_key_fingerprint().expect("should compute fingerprint");
        assert_eq!(fp, fp2);
    }

    // ── v3 encryption tests ─────────────────────────────────────────

    /// Helper to initialize the data key ring for tests.
    /// Since DATA_KEY_RING is a OnceLock, subsequent calls are no-ops.
    fn ensure_test_ring() {
        ensure_test_key();
        let dek = generate_data_key().expect("should generate DEK");
        let mut keys = HashMap::new();
        let active_id = dek.key_id.clone();
        keys.insert(dek.key_id, dek.key);
        let ring = DataKeyRing::new(keys, active_id);
        let _ = init_data_key_ring(ring);
    }

    #[test]
    fn test_v3_round_trip() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let aad = "uptrakit:test:v3_context";
        for plaintext in ["hello", "", "multi\nline\nvalue", "a".repeat(1_000).as_str()] {
            let encrypted =
                encrypt_value_v3(plaintext, aad).expect("v3 encryption should succeed");
            assert!(
                encrypted.starts_with(ENC_V3_PREFIX),
                "v3 ciphertext must carry ENC:v3: prefix"
            );
            assert!(is_encrypted(&encrypted));
            let decrypted =
                decrypt_value_v3(&encrypted, aad).expect("v3 decryption should succeed");
            assert_eq!(decrypted, plaintext);
        }
    }

    #[test]
    fn test_v3_key_id_embedded() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let encrypted =
            encrypt_value_v3("test", "aad").expect("v3 encryption should succeed");
        // Format: ENC:v3:<key_id>:<hex>
        let after_prefix = encrypted.strip_prefix(ENC_V3_PREFIX).unwrap();
        let colon_pos = after_prefix.find(':').unwrap();
        let key_id = &after_prefix[..colon_pos];
        assert_eq!(key_id.len(), 8, "embedded key_id must be 8 hex chars");
    }

    #[test]
    fn test_v3_wrong_aad_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let encrypted =
            encrypt_value_v3("secret", "correct-aad").expect("v3 encryption should succeed");
        let result = decrypt_value_v3(&encrypted, "wrong-aad");
        assert!(result.is_err(), "decrypting v3 with wrong AAD must fail");
    }

    #[test]
    fn test_v3_nonce_uniqueness() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let aad = "uptrakit:test:v3_nonce";
        let enc1 = encrypt_value_v3("same", aad).expect("should encrypt");
        let enc2 = encrypt_value_v3("same", aad).expect("should encrypt");
        assert_ne!(enc1, enc2, "v3 encryptions of the same value must differ");
    }

    #[test]
    fn test_v3_tampered_ciphertext_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let aad = "uptrakit:test:v3_tamper";
        let encrypted = encrypt_value_v3("data", aad).expect("v3 encryption should succeed");

        // Parse the format to tamper with the hex payload
        let after_prefix = encrypted.strip_prefix(ENC_V3_PREFIX).unwrap();
        let colon_pos = after_prefix.find(':').unwrap();
        let key_id = &after_prefix[..colon_pos];
        let hex_part = &after_prefix[colon_pos + 1..];

        let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
        if let Some(byte) = raw.last_mut() {
            *byte ^= 0xFF;
        }
        let tampered = format!(
            "{ENC_V3_PREFIX}{key_id}:{}",
            uptrakit_shared_types::hex::encode(&raw)
        );
        assert!(decrypt_value_v3(&tampered, aad).is_err());
    }

    #[test]
    fn test_v3_unknown_key_id_fails() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        // Craft a v3 ciphertext with a non-existent key_id
        let encrypted =
            encrypt_value_v3("test", "aad").expect("v3 encryption should succeed");
        let after_prefix = encrypted.strip_prefix(ENC_V3_PREFIX).unwrap();
        let colon_pos = after_prefix.find(':').unwrap();
        let hex_part = &after_prefix[colon_pos + 1..];

        let fake = format!("{ENC_V3_PREFIX}deadbeef:{hex_part}");
        let result = decrypt_value_v3(&fake, "aad");
        assert!(result.is_err(), "unknown key_id must fail");
    }

    #[test]
    fn test_new_produces_v3_when_ring_available() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_ring();

        let es = EncryptedString::new("v3 secret".to_string(), "test:aad")
            .expect("should encrypt");
        assert!(
            es.db_value.starts_with(ENC_V3_PREFIX),
            "new must produce ENC:v3: when ring is available"
        );
        assert_eq!(es.expose_secret(), "v3 secret");
    }

    #[test]
    fn test_create_verification_token_with_explicit_key() {
        let _lock = TEST_LOCK.lock().unwrap();
        ensure_test_key();

        let explicit_kek = Zeroizing::new([0xCDu8; 32]);
        let token = create_verification_token_with_key(&explicit_kek)
            .expect("should create token with explicit key");
        assert!(token.starts_with(ENC_V2_PREFIX));

        // Verify the token cannot be verified with the global key
        // (it was encrypted with a different key)
        let result = verify_key_verification_token(&token);
        assert!(result.is_err());
    }
}
