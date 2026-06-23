//! Phase 1: Master key initialization.

use rootcause::prelude::*;

use crate::AppError;

/// Initialize the global master encryption key from the `master_key_from` source spec.
///
/// The `master_key_from` argument supports three source URI forms:
/// - `file:/path/to/key` — read the key from a file
/// - `env:VAR_NAME` — read the key from an environment variable
/// - any other value — treated as an inline hex string
///
/// Returns the raw hex string wrapped in [`SecretString`] if a master key was
/// loaded, `None` otherwise. The [`SecretString`] zeroes the hex on drop so
/// the key material is not retained in memory beyond its needed lifetime.
pub(crate) fn init_master_key(
    master_key_from: Option<&str>,
) -> crate::Result<Option<uptrakit_wire::SecretString>> {
    let key_hex = read_master_key_hex(master_key_from)?;

    match key_hex {
        Some(key_hex) => {
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).context_to()?;
            tracing::info!("master encryption key initialized");
            let hex_for_secret = (*key_hex).clone();
            Ok(Some(uptrakit_wire::SecretString::new(hex_for_secret)))
        }
        None => {
            bail!(AppError::Config(
                "master encryption key is required: pass --master-key-from file:/path/to/key, \
                 env:VAR_NAME, or an inline hex string via UPTRAKIT_MASTER_KEY_FROM."
                    .into()
            ));
        }
    }
}

pub(crate) fn read_master_key_hex(
    master_key_from: Option<&str>,
) -> crate::Result<Option<zeroize::Zeroizing<String>>> {
    let Some(source) = master_key_from else {
        return Ok(None);
    };

    if let Some(path) = source.strip_prefix("file:") {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to read master key file {path}: {e}"
            )))
        })?;
        return Ok(Some(zeroize::Zeroizing::new(contents.trim().to_string())));
    }

    if let Some(var_name) = source.strip_prefix("env:") {
        let value = std::env::var(var_name).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to read master key from environment variable {var_name}: {e}"
            )))
        })?;
        return Ok(Some(zeroize::Zeroizing::new(value.trim().to_string())));
    }

    // Inline hex string
    Ok(Some(zeroize::Zeroizing::new(source.trim().to_string())))
}

pub(crate) fn parse_master_key_hex(key_hex: &str) -> crate::Result<[u8; 32]> {
    let bytes = uptrakit_shared_types::hex::decode(key_hex).map_err(|e| {
        report!(AppError::Config(format!(
            "master key must be a 64-character hex string: {e}"
        )))
    })?;
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
        report!(AppError::Config(format!(
            "master key must be exactly 32 bytes (64 hex chars), got {} bytes",
            v.len()
        )))
    })?;
    Ok(key_bytes)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test code: `assert!(r.is_err())` is idiomatic in tests where the error variant is not inspected"
    )]

    use super::{parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn file_key_is_trimmed() {
        let file = tempfile::NamedTempFile::new();
        assert!(file.is_ok());
        let mut file = match file {
            Ok(file) => file,
            Err(_) => return,
        };
        assert!(file.write_all(b"  0123  ").is_ok());
        let path = format!("file:{}", file.path().display());
        let result = read_master_key_hex(Some(&path));
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "0123"));
    }

    #[test]
    fn inline_hex_is_passed_through() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = read_master_key_hex(Some(hex));
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == hex));
    }

    #[test]
    fn missing_key_errors_without_source() {
        let err = super::init_master_key(None).expect_err("missing key must error");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("--master-key-from"),
            "error must mention --master-key-from, got: {rendered}"
        );
    }

    #[test]
    fn parse_master_key_rejects_invalid_hex() {
        let result = parse_master_key_hex("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_rejects_invalid_length() {
        let result = parse_master_key_hex("aa");
        assert!(result.is_err());
    }

    #[test]
    fn parse_master_key_accepts_valid_length() {
        let key_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let result = parse_master_key_hex(key_hex);
        assert!(matches!(result, Ok(bytes) if bytes.len() == 32));
    }
}
