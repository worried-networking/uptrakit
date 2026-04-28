//! ECIES-sealed sensitive parameter decryption for surface actions.

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, PrivateKey};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const P256_UNCOMPRESSED_PUBLIC_KEY_LEN: usize = 65;
const NONCE_LEN: usize = 12;
const MIN_SEALED_LEN: usize = P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN + 16;

fn sealed_box_decrypt(sealed: &[u8], private_key_pkcs8_der: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < MIN_SEALED_LEN {
        return Err("ciphertext too short".to_string());
    }

    let ephemeral_public_bytes = &sealed[..P256_UNCOMPRESSED_PUBLIC_KEY_LEN];
    let nonce_bytes: [u8; NONCE_LEN] = sealed
        [P256_UNCOMPRESSED_PUBLIC_KEY_LEN..P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN]
        .try_into()
        .map_err(|_| "invalid nonce length".to_string())?;
    let ciphertext_and_tag = &sealed[P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN..];

    let private_key =
        PrivateKey::from_private_key_der(&agreement::ECDH_P256, private_key_pkcs8_der)
            .map_err(|e| format!("parse private key: {e}"))?;
    let peer_public =
        aws_lc_rs::agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ephemeral_public_bytes);
    let shared_secret: Zeroizing<[u8; 32]> = agreement::agree(
        &private_key,
        peer_public,
        "ECDH agreement failed".to_string(),
        |secret| {
            let mut key = Zeroizing::new([0u8; 32]);
            let hash = Sha256::digest(secret);
            key.copy_from_slice(&hash);
            Ok(key)
        },
    )?;

    let unbound = UnboundKey::new(&AES_256_GCM, shared_secret.as_slice())
        .map_err(|e| format!("AES key: {e}"))?;
    let aes_key = LessSafeKey::new(unbound);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut buf = ciphertext_and_tag.to_vec();
    let plaintext = aes_key
        .open_in_place(nonce, Aad::from(ephemeral_public_bytes), &mut buf)
        .map_err(|_| "wrong key or tampered sealed box".to_string())?;

    Ok(plaintext.to_vec())
}

fn sealed_box_decrypt_base64(
    sealed_base64: &str,
    private_key_pkcs8_der: &[u8],
) -> Result<String, String> {
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(sealed_base64)
        .map_err(|e| format!("base64 decode sealed box: {e}"))?;
    let plaintext = sealed_box_decrypt(&sealed, private_key_pkcs8_der)?;
    String::from_utf8(plaintext).map_err(|e| format!("invalid UTF-8: {e}"))
}

/// Decrypt and deserialize ECIES-sealed sensitive parameters.
pub fn decrypt_sensitive_params<T: DeserializeOwned>(
    sealed_base64: Option<&str>,
    private_key_der: Option<&[u8]>,
) -> Result<Option<T>, String> {
    let sealed_b64 = match sealed_base64 {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };
    let private_key = private_key_der
        .ok_or_else(|| "sensitive params received but no private key available".to_string())?;
    let json_str = sealed_box_decrypt_base64(sealed_b64, private_key)
        .map_err(|e| format!("failed to decrypt sensitive params: {e}"))?;
    let params: T = serde_json::from_str(&json_str)
        .map_err(|e| format!("failed to parse sensitive params JSON: {e}"))?;
    Ok(Some(params))
}
