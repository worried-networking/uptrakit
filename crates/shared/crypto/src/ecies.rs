//! ECIES sealed-box encryption using P-256 ECDH + AES-256-GCM.
//!
//! Provides end-to-end encryption for sensitive extension parameters.
//! The sender encrypts using only the recipient's P-256 public key; the
//! recipient decrypts using its P-256 private key (PKCS#8 DER).
//!
//! ## Sealed-box format (binary)
//!
//! ```text
//! [65 bytes: ephemeral uncompressed P-256 public key]
//! [12 bytes: AES-256-GCM nonce]
//! [N  bytes: ciphertext + 16-byte GCM authentication tag]
//! ```
//!
//! ## Key derivation
//!
//! ```text
//! shared_secret = ECDH(ephemeral_private, recipient_public)
//! aes_key       = SHA-256(shared_secret)
//! AAD           = ephemeral_public_key_bytes   (binds ciphertext to this exchange)
//! ```
//!
//! ## Usage
//!
//! The mTLS P-256 keypair is reused: each service already has an ECDSA P-256
//! key for mutual TLS. The same key can safely serve ECDH key agreement
//! because both algorithms operate on the same secp256r1 curve and the
//! ephemeral-static ECDH construction (fresh sender key each time) provides
//! CCA2 security.

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, EphemeralPrivateKey, UnparsedPublicKey};
use rand::RngCore;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{CryptoError, Result};

/// Size of an uncompressed P-256 public key (0x04 || x || y).
const P256_UNCOMPRESSED_PUBLIC_KEY_LEN: usize = 65;

/// AES-256-GCM nonce length.
const NONCE_LEN: usize = 12;

/// AES-256-GCM authentication tag length.
const TAG_LEN: usize = 16;

/// Minimum sealed-box length: ephemeral pubkey + nonce + tag (no plaintext).
const MIN_SEALED_LEN: usize = P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN + TAG_LEN;

/// Encrypt `plaintext` so that only the holder of `recipient_public_key` can decrypt.
///
/// `recipient_public_key` must be an uncompressed P-256 point (65 bytes,
/// starting with `0x04`). Returns the binary sealed-box.
///
/// # Errors
///
/// Returns [`CryptoError::Encryption`] if key generation or ECDH fails.
pub fn sealed_box_encrypt(plaintext: &[u8], recipient_public_key: &[u8]) -> Result<Vec<u8>> {
    if recipient_public_key.len() != P256_UNCOMPRESSED_PUBLIC_KEY_LEN {
        bail!(CryptoError::Encryption(format!(
            "recipient public key must be {} bytes (uncompressed P-256), got {}",
            P256_UNCOMPRESSED_PUBLIC_KEY_LEN,
            recipient_public_key.len()
        )));
    }

    // 1. Generate ephemeral P-256 keypair.
    let rng = aws_lc_rs::rand::SystemRandom::new();
    let ephemeral_private = EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
        .map_err(|e| report!(CryptoError::Encryption(format!("ephemeral keygen: {e}"))))?;
    let ephemeral_public = ephemeral_private
        .compute_public_key()
        .map_err(|e| report!(CryptoError::Encryption(format!("compute public key: {e}"))))?;
    let ephemeral_public_bytes = ephemeral_public.as_ref();

    // 2. ECDH: derive shared secret.
    let peer_public = UnparsedPublicKey::new(&agreement::ECDH_P256, recipient_public_key);
    let shared_secret: Zeroizing<[u8; 32]> = agreement::agree_ephemeral(
        ephemeral_private,
        peer_public,
        CryptoError::Encryption("ECDH agreement failed".into()),
        |secret| {
            let mut key = Zeroizing::new([0u8; 32]);
            let hash = Sha256::digest(secret);
            key.copy_from_slice(&hash);
            Ok(key)
        },
    )
    .map_err(|e| report!(e))?;

    // 3. AES-256-GCM encrypt with ephemeral public key as AAD.
    let unbound = UnboundKey::new(&AES_256_GCM, shared_secret.as_slice())
        .map_err(|e| report!(CryptoError::Encryption(format!("AES key: {e}"))))?;
    let aes_key = LessSafeKey::new(unbound);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.to_vec();
    let tag = aes_key
        .seal_in_place_separate_tag(nonce, Aad::from(ephemeral_public_bytes), &mut in_out)
        .map_err(|e| report!(CryptoError::Encryption(format!("AES-GCM seal: {e}"))))?;

    // 4. Assemble: ephemeral_public || nonce || ciphertext || tag
    let mut sealed =
        Vec::with_capacity(ephemeral_public_bytes.len() + NONCE_LEN + in_out.len() + TAG_LEN);
    sealed.extend_from_slice(ephemeral_public_bytes);
    sealed.extend_from_slice(&nonce_bytes);
    sealed.extend_from_slice(&in_out);
    sealed.extend_from_slice(tag.as_ref());

    Ok(sealed)
}

/// Decrypt a sealed-box produced by [`sealed_box_encrypt`].
///
/// `private_key_pkcs8_der` is the recipient's P-256 private key in PKCS#8 DER
/// format (as produced by `rcgen::KeyPair::serialize_der()`).
///
/// # Errors
///
/// Returns [`CryptoError::Decryption`] on any failure (wrong key, tampered data,
/// truncated input).
pub fn sealed_box_decrypt(sealed: &[u8], private_key_pkcs8_der: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < MIN_SEALED_LEN {
        bail!(CryptoError::CiphertextTooShort);
    }

    // 1. Parse sealed-box components.
    let ephemeral_public_bytes = &sealed[..P256_UNCOMPRESSED_PUBLIC_KEY_LEN];
    let nonce_bytes: [u8; NONCE_LEN] = sealed
        [P256_UNCOMPRESSED_PUBLIC_KEY_LEN..P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN]
        .try_into()
        .map_err(|_| report!(CryptoError::InvalidNonce))?;
    let ciphertext_and_tag = &sealed[P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN..];

    // 2. ECDH: derive shared secret using the recipient's static private key.
    let private_key =
        agreement::PrivateKey::from_private_key_der(&agreement::ECDH_P256, private_key_pkcs8_der)
            .map_err(|e| report!(CryptoError::Decryption(format!("parse private key: {e}"))))?;

    let peer_public = UnparsedPublicKey::new(&agreement::ECDH_P256, ephemeral_public_bytes);

    let shared_secret: Zeroizing<[u8; 32]> = agreement::agree(
        &private_key,
        peer_public,
        CryptoError::Decryption("ECDH agreement failed".into()),
        |secret| {
            let mut key = Zeroizing::new([0u8; 32]);
            let hash = Sha256::digest(secret);
            key.copy_from_slice(&hash);
            Ok(key)
        },
    )
    .map_err(|e| report!(e))?;

    // 3. AES-256-GCM decrypt.
    let unbound = UnboundKey::new(&AES_256_GCM, shared_secret.as_slice())
        .map_err(|e| report!(CryptoError::Decryption(format!("AES key: {e}"))))?;
    let aes_key = LessSafeKey::new(unbound);

    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut buf = ciphertext_and_tag.to_vec();
    let plaintext = aes_key
        .open_in_place(nonce, Aad::from(ephemeral_public_bytes), &mut buf)
        .map_err(|_| {
            report!(CryptoError::Decryption(
                "wrong key or tampered sealed box".into()
            ))
        })?;

    Ok(plaintext.to_vec())
}

/// Base64-encoded convenience wrapper around [`sealed_box_encrypt`].
///
/// `recipient_public_key_base64` is the standard (non-URL-safe) base64 encoding
/// of the uncompressed P-256 public key (65 bytes).
///
/// Returns the sealed-box as standard base64.
pub fn sealed_box_encrypt_base64(
    plaintext: &str,
    recipient_public_key_base64: &str,
) -> Result<String> {
    use base64::Engine as _;
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(recipient_public_key_base64)
        .map_err(|e| {
            report!(CryptoError::Encryption(format!(
                "base64 decode public key: {e}"
            )))
        })?;
    let sealed = sealed_box_encrypt(plaintext.as_bytes(), &public_key)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&sealed))
}

/// Base64-encoded convenience wrapper around [`sealed_box_decrypt`].
///
/// `sealed_base64` is the standard base64 encoding of the sealed-box.
///
/// Returns the decrypted plaintext as a UTF-8 string.
pub fn sealed_box_decrypt_base64(
    sealed_base64: &str,
    private_key_pkcs8_der: &[u8],
) -> Result<String> {
    use base64::Engine as _;
    let sealed = base64::engine::general_purpose::STANDARD
        .decode(sealed_base64)
        .map_err(|e| {
            report!(CryptoError::Decryption(format!(
                "base64 decode sealed box: {e}"
            )))
        })?;
    let plaintext = sealed_box_decrypt(&sealed, private_key_pkcs8_der)?;
    String::from_utf8(plaintext)
        .map_err(|e| report!(CryptoError::Decryption(format!("invalid UTF-8: {e}"))))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a P-256 keypair for testing using rcgen (same as mTLS keys).
    fn test_keypair() -> (Vec<u8>, Vec<u8>) {
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let public = kp.public_key_raw().to_vec();
        let private_der = kp.serialize_der();
        (public, private_der)
    }

    #[test]
    fn roundtrip_binary() {
        let (public, private_der) = test_keypair();
        let plaintext = b"hello, sealed box!";

        let sealed = sealed_box_encrypt(plaintext, &public).expect("encrypt");
        assert!(sealed.len() >= MIN_SEALED_LEN);

        let decrypted = sealed_box_decrypt(&sealed, &private_der).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn roundtrip_base64() {
        use base64::Engine as _;
        let (public, private_der) = test_keypair();
        let public_b64 = base64::engine::general_purpose::STANDARD.encode(&public);

        let plaintext = r#"{"password":"s3cret","private_key":"-----BEGIN..."}"#;

        let sealed_b64 = sealed_box_encrypt_base64(plaintext, &public_b64).expect("encrypt base64");
        let decrypted =
            sealed_box_decrypt_base64(&sealed_b64, &private_der).expect("decrypt base64");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let (public, _) = test_keypair();
        let (_, wrong_private_der) = test_keypair();

        let sealed = sealed_box_encrypt(b"secret", &public).expect("encrypt");
        let err = sealed_box_decrypt(&sealed, &wrong_private_der);
        assert!(err.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let (public, private_der) = test_keypair();

        let mut sealed = sealed_box_encrypt(b"secret", &public).expect("encrypt");
        // Flip a byte in the ciphertext portion.
        let idx = P256_UNCOMPRESSED_PUBLIC_KEY_LEN + NONCE_LEN + 1;
        sealed[idx] ^= 0xff;

        let err = sealed_box_decrypt(&sealed, &private_der);
        assert!(err.is_err());
    }

    #[test]
    fn truncated_input_fails() {
        let (_, private_der) = test_keypair();
        let err = sealed_box_decrypt(&[0u8; 10], &private_der);
        assert!(err.is_err());
    }

    #[test]
    fn bad_public_key_length_fails() {
        let err = sealed_box_encrypt(b"test", &[0u8; 32]);
        assert!(err.is_err());
    }

    #[test]
    fn empty_plaintext_roundtrip() {
        let (public, private_der) = test_keypair();

        let sealed = sealed_box_encrypt(b"", &public).expect("encrypt");
        let decrypted = sealed_box_decrypt(&sealed, &private_der).expect("decrypt");
        assert!(decrypted.is_empty());
    }

    #[test]
    fn large_plaintext_roundtrip() {
        let (public, private_der) = test_keypair();
        let plaintext = vec![0xAB; 100_000];

        let sealed = sealed_box_encrypt(&plaintext, &public).expect("encrypt");
        let decrypted = sealed_box_decrypt(&sealed, &private_der).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn each_encryption_produces_different_ciphertext() {
        let (public, _) = test_keypair();
        let plaintext = b"determinism check";

        let sealed1 = sealed_box_encrypt(plaintext, &public).expect("encrypt 1");
        let sealed2 = sealed_box_encrypt(plaintext, &public).expect("encrypt 2");
        assert_ne!(
            sealed1, sealed2,
            "each encryption must use fresh ephemeral key"
        );
    }
}
