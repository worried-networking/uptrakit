//! v1 encryption format — legacy, read-only decrypt path.
//!
//! Uses AES-256-GCM with **empty AAD**. The v1 encrypt function is only
//! available in tests (to create legacy ciphertexts for backward-compat
//! testing). The v1 decrypt function is used by the migration path
//! (`decrypt_str` delegates here for `ENC:v1:` prefixed values).
//!
//! Wire format: `ENC:v1:<hex(nonce || ciphertext || tag)>`

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
#[cfg(test)]
use rand::RngCore;
use rootcause::prelude::*;
use zeroize::Zeroizing;

use crate::{CryptoError, ENC_V1_PREFIX, Result};

/// Encrypt a plaintext value using v1 format (empty AAD).
///
/// Only available in tests — production code must never produce new v1
/// ciphertexts.
#[cfg(test)]
pub(crate) fn encrypt_value_v1(key_bytes: &Zeroizing<[u8; 32]>, plaintext: &str) -> Result<String> {
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

/// Decrypt a `ENC:v1:` ciphertext using the provided key bytes.
///
/// v1 ciphertexts were produced with empty AAD, so no AAD parameter is needed.
pub(crate) fn decrypt_value_v1_legacy(
    key_bytes: &Zeroizing<[u8; 32]>,
    stored: &str,
) -> Result<String> {
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
