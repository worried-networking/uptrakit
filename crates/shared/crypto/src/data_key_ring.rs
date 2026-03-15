//! Data key ring for envelope encryption.
//!
//! Manages a set of data encryption keys (DEKs) used for v3 envelope
//! encryption. The master key (KEK) wraps/unwraps DEKs — it is never
//! used to encrypt data directly in the v3 path.

use std::collections::HashMap;

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{CryptoError, Result};

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
    /// * `keys` -- Map of key_id to raw DEK bytes.
    /// * `active_key_id` -- The key_id of the DEK to use for new encryptions.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::MissingActiveKey`] if `active_key_id` is not present in `keys`.
    pub fn new(keys: HashMap<String, Zeroizing<[u8; 32]>>, active_key_id: String) -> Result<Self> {
        if !keys.contains_key(&active_key_id) {
            bail!(CryptoError::MissingActiveKey);
        }
        Ok(Self {
            keys,
            active_key_id,
        })
    }

    /// Returns the key_id of the active DEK.
    pub fn active_key_id(&self) -> &str {
        &self.active_key_id
    }

    /// Returns the raw key bytes for the active DEK.
    ///
    /// # Panics (impossible)
    ///
    /// The invariant that `active_key_id` is present in `keys` is established
    /// at construction time by [`DataKeyRing::new`]. This `.expect()` can never
    /// be reached in practice.
    pub(crate) fn active_key(&self) -> &Zeroizing<[u8; 32]> {
        self.keys
            .get(&self.active_key_id)
            .expect("active key must exist in ring -- invariant established at construction")
    }

    /// Look up a DEK by its key_id.
    pub(crate) fn get(&self, key_id: &str) -> Option<&Zeroizing<[u8; 32]>> {
        self.keys.get(key_id)
    }

    /// Returns the number of keys in the ring.
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }
}

/// Compute the key_id for a raw DEK: first 8 hex chars of SHA-256(dek).
pub fn compute_key_id(dek_bytes: &[u8; 32]) -> String {
    let hash = Sha256::digest(dek_bytes);
    uptrakit_shared_types::hex::encode(&hash[..4])
}

/// Generate a new random 32-byte DEK, returning its key_id and key material.
pub fn generate_data_key() -> Result<DataKey> {
    let mut key_bytes = Zeroizing::new([0u8; 32]);
    rand::rng().fill_bytes(key_bytes.as_mut_slice());
    let key_id = compute_key_id(&key_bytes);
    tracing::debug!(key_id, "generated new data encryption key");
    Ok(DataKey {
        key_id,
        key: key_bytes,
    })
}

/// Wrap (encrypt) a DEK with the provided KEK.
///
/// Uses AES-256-GCM with AAD `"uptrakit:dek:<key_id>"` to bind the
/// ciphertext to the specific DEK identity.  Returns the hex-encoded
/// `nonce || ciphertext || tag`.
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

/// Unwrap (decrypt) a DEK using the provided KEK.
///
/// The `wrapped_hex` is the hex-encoded `nonce || ciphertext || tag` produced
/// by [`wrap_data_key_with`].  The `key_id` is used to reconstruct the AAD.
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
        .open_in_place(
            nonce,
            Aad::from(aad_str.as_bytes()),
            &mut ciphertext_and_tag,
        )
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
