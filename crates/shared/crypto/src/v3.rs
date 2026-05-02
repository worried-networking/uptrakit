//! v3 encryption format — envelope encryption with DEK.
//!
//! Uses AES-256-GCM with caller-supplied AAD, but encrypts with a data
//! encryption key (DEK) from the [`DataKeyRing`] rather than the master
//! key directly. The DEK's `key_id` is embedded in the ciphertext prefix
//! to enable lookup during decryption.
//!
//! Wire format: `ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>`

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;

use crate::data_key_ring::DataKeyRing;
use crate::{CryptoError, ENC_V3_PREFIX, Result};

/// Encrypt with the active DEK from the provided data key ring.
///
/// Format: `ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>`
pub(crate) fn encrypt_value_v3(ring: &DataKeyRing, plaintext: &str, aad: &str) -> Result<String> {
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

/// Decrypt a `ENC:v3:<key_id>:<hex>` ciphertext using the provided data key ring.
#[expect(
    clippy::string_slice,
    clippy::indexing_slicing,
    clippy::map_err_ignore,
    reason = "bounds validated: colon_pos from str::find is a valid char boundary; raw.len() < 12+16 bails before slice; TryFromSliceError/ring error carry no additional context"
)]
pub(crate) fn decrypt_value_v3(ring: &DataKeyRing, stored: &str, aad: &str) -> Result<String> {
    let after_prefix = stored
        .strip_prefix(ENC_V3_PREFIX)
        .ok_or_else(|| report!(CryptoError::Decryption("missing ENC:v3: prefix".into())))?;

    // Parse key_id (8 hex chars) and hex payload separated by ':'
    let colon_pos = after_prefix.find(':').ok_or_else(|| {
        report!(CryptoError::Decryption(
            "missing key_id separator in ENC:v3".into()
        ))
    })?;

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
