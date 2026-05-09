//! Phase 1: Master key initialization.

use rootcause::prelude::*;

use crate::AppError;

/// Initialize the global master encryption key from `--master-key-file`.
///
/// Returns the raw hex string wrapped in [`SecretString`] if a master key was
/// loaded, `None` otherwise. The [`SecretString`] zeroes the hex on drop so
/// the key material is not retained in memory beyond its needed lifetime.
pub(crate) fn init_master_key(
    args: &crate::cli::Args,
) -> crate::Result<Option<uptrakit_wire::SecretString>> {
    let key_hex = read_master_key_hex(args.master_key_file.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if args.allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).context_to()?;
            tracing::info!("master encryption key initialized");
            let hex_for_secret = (*key_hex).clone();
            Ok(Some(uptrakit_wire::SecretString::new(hex_for_secret)))
        }
        None => {
            if args.allow_plaintext_secrets {
                tracing::warn!(
                    "master encryption key not set; encryption at rest is disabled. \
                    This is for development only and is NOT safe for production."
                );
                uptrakit_crypto::enable_plaintext_mode();
            } else {
                bail!(AppError::Config(
                    "master encryption key is required: pass --master-key-file <path> \
                     (64-char hex). For development only, pass --allow-plaintext-secrets \
                     to run without encryption at rest."
                        .into()
                ));
            }
            Ok(None)
        }
    }
}

pub(crate) fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
) -> crate::Result<Option<zeroize::Zeroizing<String>>> {
    if let Some(key_file) = master_key_file {
        let contents = std::fs::read_to_string(key_file).map_err(|e| {
            report!(AppError::Config(format!(
                "failed to read --master-key-file {}: {e}",
                key_file.display()
            )))
        })?;
        return Ok(Some(zeroize::Zeroizing::new(contents.trim().to_string())));
    }

    Ok(None)
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
        let result = read_master_key_hex(Some(file.path()));
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "0123"));
    }

    #[test]
    fn missing_key_bail_message_does_not_mention_env_var() {
        // Regression guard: the error message must point at --master-key-file
        // and must not resurrect any UPTRAKIT_MASTER_KEY mention.
        use clap::Parser;
        let args = crate::cli::Args::try_parse_from(["uptrakit-controller"])
            .expect("default args should parse");
        assert!(args.master_key_file.is_none() && !args.allow_plaintext_secrets);
        let err = super::init_master_key(&args).expect_err("missing key must error");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("--master-key-file"),
            "error must mention --master-key-file, got: {rendered}"
        );
        assert!(
            !rendered.contains("UPTRAKIT_MASTER_KEY"),
            "error must not mention the legacy env var, got: {rendered}"
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
