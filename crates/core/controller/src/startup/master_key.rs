//! Phase 1: Master key initialization.

use rootcause::prelude::*;

use crate::AppError;

/// Initialize the global master encryption key from env var or file.
///
/// Returns the raw hex string wrapped in [`SecretString`] if a master key was
/// loaded, `None` otherwise. The [`SecretString`] zeroes the hex on drop so
/// the key material is not retained in memory beyond its needed lifetime.
pub(crate) fn init_master_key(
    args: &crate::cli::Args,
) -> crate::Result<Option<uptrakit_internal_wire::SecretString>> {
    let env_val = std::env::var("UPTRAKIT_MASTER_KEY").ok();
    // Clear the environment variable immediately to remove it from
    // /proc/pid/environ, container inspection output, and child processes.
    // The value has already been captured in `env_val`.
    //
    // SAFETY: this is called during single-threaded startup before any
    // async runtime or threads are spawned, satisfying the safety
    // requirement that no other thread concurrently reads the environment.
    unsafe { std::env::remove_var("UPTRAKIT_MASTER_KEY") };
    let key_hex = read_master_key_hex(args.master_key_file.as_deref(), env_val.as_deref())?;

    match key_hex {
        Some(key_hex) => {
            if args.allow_plaintext_secrets {
                tracing::warn!(
                    "--allow-plaintext-secrets is enabled. This flag is for development only; \
                    encryption remains enabled because a master key was provided."
                );
            }
            // Warn when the key is supplied via environment variable rather than a file.
            // Environment variables are visible in /proc/pid/environ, container manifests,
            // and orchestration tooling — use --master-key-file with mode 0o600 in production.
            if args.master_key_file.is_none() && env_val.is_some() {
                tracing::warn!(
                    "master encryption key loaded from UPTRAKIT_MASTER_KEY environment variable. \
                     This method is DEPRECATED and will be removed in a future release. \
                     Use --master-key-file with a file readable only by the service user \
                     (mode 0o600). The environment variable has been cleared from the process \
                     environment, but may still be visible in container manifests and \
                     orchestration tooling."
                );
            }
            let key_bytes = parse_master_key_hex(&key_hex)?;
            uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key_bytes)).context_to()?;
            tracing::info!("master encryption key initialized");
            // Transfer the hex string into SecretString which also zeroizes on drop.
            // We clone via Deref<Target=String> so the Zeroizing wrapper scrubs its copy.
            let hex_for_secret = (*key_hex).clone();
            Ok(Some(uptrakit_internal_wire::SecretString::new(
                hex_for_secret,
            )))
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
                    "master encryption key is required: set UPTRAKIT_MASTER_KEY env var \
                     (64-char hex string) or pass --master-key-file <path>. \
                     For development only, pass --allow-plaintext-secrets to run without \
                     encryption at rest."
                        .into()
                ));
            }
            Ok(None)
        }
    }
}

pub(crate) fn read_master_key_hex(
    master_key_file: Option<&std::path::Path>,
    env_val: Option<&str>,
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

    if let Some(env_val) = env_val {
        return Ok(Some(zeroize::Zeroizing::new(env_val.trim().to_string())));
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
    use super::{parse_master_key_hex, read_master_key_hex};
    use std::io::Write;

    #[test]
    fn missing_key_returns_none() {
        let result = read_master_key_hex(None, None);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn env_key_is_trimmed() {
        let result = read_master_key_hex(None, Some("  deadbeef  "));
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "deadbeef"));
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
        let result = read_master_key_hex(Some(file.path()), None);
        assert!(matches!(result, Ok(Some(ref value)) if value.as_str() == "0123"));
    }

    #[test]
    fn read_master_key_hex_returns_zeroizing() {
        let result = read_master_key_hex(None, Some("abcdef")).unwrap().unwrap();
        assert_eq!(result.as_str(), "abcdef");
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
