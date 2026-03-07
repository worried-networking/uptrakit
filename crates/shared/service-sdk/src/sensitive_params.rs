//! ECIES-sealed sensitive parameter decryption for extension actions.
//!
//! Extension actions that accept sensitive user input (passwords, private keys)
//! receive those values encrypted via an ECIES sealed box.  The frontend encrypts
//! the JSON object with the service's public key; this module decrypts it on the
//! service side.
//!
//! # Usage
//!
//! ```ignore
//! let sensitive: Option<MyParams> = decrypt_sensitive_params(
//!     sealed_base64,
//!     private_key_der,
//! )?;
//! ```

use serde::de::DeserializeOwned;
use uptrakit_crypto::ecies::sealed_box_decrypt_base64;

/// Decrypt and deserialize ECIES-sealed sensitive parameters.
///
/// `sealed_base64` is the base64-encoded ECIES sealed box from
/// [`ExtensionRequestPayload::sensitive_params`].  `private_key_der` is the
/// service's PKCS#8 DER-encoded private key.
///
/// Returns `Ok(None)` when no sensitive params were provided (absent or empty).
/// Returns `Err(message)` on decryption or deserialization failure.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestParams {
        secret: String,
    }

    #[test]
    fn none_when_absent() {
        let result: Result<Option<TestParams>, String> =
            decrypt_sensitive_params(None, Some(b"unused"));
        assert!(result.is_ok());
        assert!(result.as_ref().ok().and_then(|o| o.as_ref()).is_none());
    }

    #[test]
    fn none_when_empty_string() {
        let result: Result<Option<TestParams>, String> =
            decrypt_sensitive_params(Some(""), Some(b"unused"));
        assert!(result.is_ok());
        assert!(result.as_ref().ok().and_then(|o| o.as_ref()).is_none());
    }

    #[test]
    fn error_when_no_private_key() {
        let result: Result<Option<TestParams>, String> =
            decrypt_sensitive_params(Some("dGVzdA=="), None);
        assert!(result.is_err());
        assert!(
            result
                .as_ref()
                .err()
                .is_some_and(|e| e.contains("no private key"))
        );
    }
}
