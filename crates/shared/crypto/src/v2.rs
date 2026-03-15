//! v2 encryption format — caller-supplied AAD, KEK-direct.
//!
//! Uses AES-256-GCM with caller-supplied AAD for context binding.
//! The master key (KEK) is used directly to encrypt data. This format
//! is used as a fallback when the data key ring is not yet initialized,
//! and for key verification tokens.
//!
//! Wire format: `ENC:v2:<hex(nonce || ciphertext || tag)>`

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use rand::RngCore;
use rootcause::prelude::*;
use zeroize::Zeroizing;

use crate::{CryptoError, ENC_V2_PREFIX, Result};

/// Encrypt a plaintext value with caller-supplied AAD using v2 format.
///
/// The key bytes are the master key (KEK) used directly for encryption.
pub(crate) fn encrypt_value_v2(
    key_bytes: &Zeroizing<[u8; 32]>,
    plaintext: &str,
    aad: &str,
) -> Result<String> {
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

/// Decrypt a `ENC:v2:` ciphertext using the provided key bytes and AAD.
///
/// The AAD must match the AAD used during encryption.
pub(crate) fn decrypt_value_v2(
    key_bytes: &Zeroizing<[u8; 32]>,
    stored: &str,
    aad: &str,
) -> Result<String> {
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
