#![expect(
    clippy::assertions_on_result_states,
    clippy::let_underscore_must_use,
    clippy::string_slice,
    reason = "test helpers and assertions — is_ok/is_err checks, let _ = result, and string slices are idiomatic in tests"
)]
use super::*;
use std::collections::HashMap;
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

/// Helper to get the master key for passing to v1/v2 module functions.
fn test_master_key() -> &'static Zeroizing<[u8; 32]> {
    MASTER_KEY.get().expect("test key must be initialized")
}

#[test]
fn test_v1_round_trip() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();
    let key = test_master_key();

    for plaintext in [
        "hello world",
        "",
        "\u{1f980} Rust",
        "a".repeat(10_000).as_str(),
    ] {
        let encrypted = v1::encrypt_value_v1(key, plaintext).expect("encryption should succeed");
        assert!(encrypted.starts_with(ENC_V1_PREFIX));
        let decrypted =
            v1::decrypt_value_v1_legacy(key, &encrypted).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }
}

#[test]
fn test_v1_nonce_uniqueness() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();
    let key = test_master_key();

    let enc1 = v1::encrypt_value_v1(key, "same input").expect("encryption should succeed");
    let enc2 = v1::encrypt_value_v1(key, "same input").expect("encryption should succeed");
    assert_ne!(
        enc1, enc2,
        "two encryptions of the same value should produce different ciphertext"
    );

    // Both should still decrypt to the same value
    assert_eq!(
        v1::decrypt_value_v1_legacy(key, &enc1).expect("decryption should succeed"),
        "same input"
    );
    assert_eq!(
        v1::decrypt_value_v1_legacy(key, &enc2).expect("decryption should succeed"),
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
    let key = test_master_key();

    let encrypted = v1::encrypt_value_v1(key, "sensitive data").expect("encryption should succeed");
    // Tamper with one byte in the hex payload
    let hex_part = encrypted
        .strip_prefix(ENC_V1_PREFIX)
        .expect("should have prefix");
    let mut raw = uptrakit_shared_types::hex::decode(hex_part).expect("valid hex");
    if let Some(byte) = raw.last_mut() {
        *byte ^= 0xFF;
    }
    let tampered = format!(
        "{ENC_V1_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&raw)
    );
    assert!(v1::decrypt_value_v1_legacy(key, &tampered).is_err());
}

#[cfg(feature = "sea-orm")]
#[test]
fn test_seaorm_value_roundtrip() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();

    // ValueType::try_from uses best-effort empty AAD for v3 ciphertexts,
    // so encrypt with empty AAD to make the round-trip work.
    let original = EncryptedString::new("database secret".to_string(), "").expect("test key set");
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
    let key = test_master_key();

    // Simulate a legacy ENC:v1: token (from an installation before this change).
    // ENC:v1: tokens are decrypted with empty AAD — the sentinel must match.
    let legacy_token = v1::encrypt_value_v1(key, KEY_VERIFICATION_SENTINEL)
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
    let tampered = format!(
        "{ENC_V2_PREFIX}{}",
        uptrakit_shared_types::hex::encode(&raw)
    );
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
    let key = test_master_key();

    // Construct a value with valid prefix but ciphertext shorter than nonce + tag (28 bytes).
    let short_bytes = [0u8; 10];
    let short_hex = uptrakit_shared_types::hex::encode(short_bytes);
    let stored = format!("{ENC_V1_PREFIX}{short_hex}");
    let result = v1::decrypt_value_v1_legacy(key, &stored);
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
    let key = test_master_key();

    // Attempt to decrypt a string without the ENC:v1: prefix.
    let result = v1::decrypt_value_v1_legacy(key, "not-encrypted-data");
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
    let key = test_master_key();

    let result = v1::decrypt_value_v1_legacy(key, &format!("{ENC_V1_PREFIX}not-valid-hex!@#$"));
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

    let es1 = EncryptedString::new("value_a".to_string(), "test-aad").expect("should encrypt");
    let es2 = EncryptedString::new("value_b".to_string(), "test-aad").expect("should encrypt");

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
    for plaintext in [
        "hello",
        "",
        "multi\nline\nvalue",
        "a".repeat(1_000).as_str(),
    ] {
        let encrypted = encrypt_str(plaintext, aad).expect("encryption should succeed");
        assert!(is_encrypted(&encrypted));
        let decrypted = decrypt_str(&encrypted, aad).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }
}

#[test]
fn test_encrypt_decrypt_str_wrong_aad_fails() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();

    let encrypted = encrypt_str("secret", "correct-aad").expect("encryption should succeed");
    let result = decrypt_str(&encrypted, "wrong-aad");
    assert!(result.is_err(), "decrypting with wrong AAD must fail");
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
    assert_ne!(
        enc1, enc2,
        "encryptions of the same value must differ (random nonces)"
    );
}

#[test]
fn test_decrypt_str_accepts_v1_fallback() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();
    let key = test_master_key();

    // An ENC:v1: ciphertext should be accepted by decrypt_str
    // regardless of the provided aad (backward compat).
    let v1_encrypted =
        v1::encrypt_value_v1(key, "legacy_value").expect("v1 encryption should succeed");
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
    let es = EncryptedString::new("aad secret".to_string(), aad).expect("new should succeed");
    // Produces v3 when ring is available, v2 otherwise
    assert!(
        es.db_value.starts_with(ENC_V2_PREFIX) || es.db_value.starts_with(ENC_V3_PREFIX),
        "new must produce ENC:v2: or ENC:v3: ciphertext"
    );
    assert_eq!(es.expose_secret(), "aad secret");

    // Must decrypt with the same AAD (decrypt_str handles both v2 and v3)
    let decrypted = decrypt_str(&es.db_value, aad).expect("decryption with correct AAD");
    assert_eq!(decrypted, "aad secret");
}

#[test]
fn test_encrypted_string_new_wrong_aad_fails() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();

    let es = EncryptedString::new("secret".to_string(), "correct:aad").expect("should encrypt");
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
    let key = test_master_key();

    // Create a v1 ciphertext and wrap it in EncryptedString via plaintext_for_test
    // (which stores the value directly as db_value — works for any format).
    let v1_ciphertext = v1::encrypt_value_v1(key, "v1 value").expect("should encrypt");
    let es = EncryptedString::plaintext_for_test(v1_ciphertext);
    assert!(es.needs_v3_upgrade(), "v1 ciphertext needs v3 upgrade");
}

#[test]
fn test_needs_v3_upgrade_for_v2() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_key();
    let key = test_master_key();

    let v2_ciphertext = v2::encrypt_value_v2(key, "v2 value", "test:aad").expect("should encrypt");
    let es = EncryptedString::plaintext_for_test(v2_ciphertext);
    assert!(es.needs_v3_upgrade(), "v2 ciphertext needs v3 upgrade");
}

#[test]
fn test_needs_v3_upgrade_false_for_v3() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let v3_ciphertext = v3::encrypt_value_v3(ring, "v3 value", "test:aad").expect("should encrypt");
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
    let es = EncryptedString::new("sensitive".to_string(), "uptrakit:table_a:column_a")
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
    let entries = &[ColumnAadEntry {
        table: "test_table",
        column: "test_col",
        aad: "uptrakit:t:test_col",
    }];
    // Ignore error — may already be initialized by another test
    let _ = register_column_aad(entries);

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

// ── DEK / envelope encryption tests ──────────────────────────────

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
    let unwrapped = unwrap_data_key_with(&new_kek, &wrapped, &dek.key_id).expect("should unwrap");

    assert_eq!(unwrapped.key.as_slice(), dek.key.as_slice());

    // Should fail with the original KEK
    let result = unwrap_data_key(&wrapped, &dek.key_id);
    assert!(result.is_err(), "unwrapping with wrong KEK must fail");
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

// ── v3 encryption tests ──────────────────────────────────────────

/// Helper to initialize the data key ring for tests.
/// Since DATA_KEY_RING is a OnceLock, subsequent calls are no-ops.
fn ensure_test_ring() {
    ensure_test_key();
    let dek = generate_data_key().expect("should generate DEK");
    let mut keys = HashMap::new();
    let active_id = dek.key_id.clone();
    keys.insert(dek.key_id, dek.key);
    let ring = DataKeyRing::new(keys, active_id).expect("test ring construction");
    let _ = init_data_key_ring(ring);
}

#[test]
fn test_v3_round_trip() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let aad = "uptrakit:test:v3_context";
    for plaintext in [
        "hello",
        "",
        "multi\nline\nvalue",
        "a".repeat(1_000).as_str(),
    ] {
        let encrypted =
            v3::encrypt_value_v3(ring, plaintext, aad).expect("v3 encryption should succeed");
        assert!(
            encrypted.starts_with(ENC_V3_PREFIX),
            "v3 ciphertext must carry ENC:v3: prefix"
        );
        assert!(is_encrypted(&encrypted));
        let decrypted =
            v3::decrypt_value_v3(ring, &encrypted, aad).expect("v3 decryption should succeed");
        assert_eq!(decrypted, plaintext);
    }
}

#[test]
fn test_v3_key_id_embedded() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let encrypted =
        v3::encrypt_value_v3(ring, "test", "aad").expect("v3 encryption should succeed");
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

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let encrypted =
        v3::encrypt_value_v3(ring, "secret", "correct-aad").expect("v3 encryption should succeed");
    let result = v3::decrypt_value_v3(ring, &encrypted, "wrong-aad");
    assert!(result.is_err(), "decrypting v3 with wrong AAD must fail");
}

#[test]
fn test_v3_nonce_uniqueness() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let aad = "uptrakit:test:v3_nonce";
    let enc1 = v3::encrypt_value_v3(ring, "same", aad).expect("should encrypt");
    let enc2 = v3::encrypt_value_v3(ring, "same", aad).expect("should encrypt");
    assert_ne!(enc1, enc2, "v3 encryptions of the same value must differ");
}

#[test]
fn test_v3_tampered_ciphertext_fails() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    let aad = "uptrakit:test:v3_tamper";
    let encrypted = v3::encrypt_value_v3(ring, "data", aad).expect("v3 encryption should succeed");

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
    assert!(v3::decrypt_value_v3(ring, &tampered, aad).is_err());
}

#[test]
fn test_v3_unknown_key_id_fails() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let ring = DATA_KEY_RING.get().expect("ring must be initialized");
    // Craft a v3 ciphertext with a non-existent key_id
    let encrypted =
        v3::encrypt_value_v3(ring, "test", "aad").expect("v3 encryption should succeed");
    let after_prefix = encrypted.strip_prefix(ENC_V3_PREFIX).unwrap();
    let colon_pos = after_prefix.find(':').unwrap();
    let hex_part = &after_prefix[colon_pos + 1..];

    let fake = format!("{ENC_V3_PREFIX}deadbeef:{hex_part}");
    let result = v3::decrypt_value_v3(ring, &fake, "aad");
    assert!(result.is_err(), "unknown key_id must fail");
}

#[test]
fn test_new_produces_v3_when_ring_available() {
    let _lock = TEST_LOCK.lock().unwrap();
    ensure_test_ring();

    let es = EncryptedString::new("v3 secret".to_string(), "test:aad").expect("should encrypt");
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
