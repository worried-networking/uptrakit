use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A newtype wrapper for strings that contain sensitive data (secrets, tokens,
/// passwords).
///
/// `Debug` and `Display` implementations redact the value. Serialization is
/// transparent so the JSON wire format is unchanged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// Wrap a plaintext value.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrow the plaintext secret.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }


}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***REDACTED***")
    }
}

impl Hash for SecretString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts() {
        let s = SecretString::new("my-secret".into());
        let debug = format!("{s:?}");
        assert!(!debug.contains("my-secret"));
        assert!(debug.contains("***"));
    }

    #[test]
    fn display_redacts() {
        let s = SecretString::new("my-secret".into());
        let display = format!("{s}");
        assert!(!display.contains("my-secret"));
        assert!(display.contains("REDACTED"));
    }

    #[test]
    fn expose_secret_returns_value() {
        let s = SecretString::new("token-123".into());
        assert_eq!(s.expose_secret(), "token-123");
    }



    #[test]
    fn serde_roundtrip() {
        let s = SecretString::new("secret-value".into());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#""secret-value""#);
        let deserialized: SecretString = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.expose_secret(), "secret-value");
    }

    #[test]
    fn serde_in_option() {
        let some: Option<SecretString> = Some(SecretString::new("val".into()));
        let json = serde_json::to_string(&some).unwrap();
        assert_eq!(json, r#""val""#);

        let none: Option<SecretString> = None;
        let json = serde_json::to_string(&none).unwrap();
        assert_eq!(json, "null");
    }

    #[test]
    fn equality() {
        let a = SecretString::new("same".into());
        let b = SecretString::new("same".into());
        let c = SecretString::new("other".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        let a = SecretString::new("token".into());
        let b = SecretString::new("token".into());
        let hash_a = {
            let mut h = DefaultHasher::new();
            a.hash(&mut h);
            h.finish()
        };
        let hash_b = {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn clone_preserves_value() {
        let s = SecretString::new("value".into());
        let cloned = s.clone();
        assert_eq!(cloned.expose_secret(), "value");
    }
}
