use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A newtype wrapper for strings that contain sensitive data (secrets, tokens,
/// passwords).
///
/// # Security properties
///
/// - **`Debug`** and **`Display`** implementations redact the value, preventing
///   accidental exposure in logs, error chains, and tracing output.
/// - **`ZeroizeOnDrop`** overwrites the backing heap allocation with zeros when
///   the value is dropped.
/// - **`Serialize`** is **transparent** (`#[serde(transparent)]`): serializing a
///   `SecretString` emits the plaintext value. Never serialize structs containing
///   `SecretString` to logs or debug output — use `Debug` formatting instead.
/// - **`Clone`** creates a new heap allocation containing the secret. Each clone
///   must be properly dropped (not leaked or stored in a non-zeroizing container)
///   to ensure the zeroize guarantee is upheld.
/// - **`PartialEq`** uses the standard `String` comparison, which short-circuits
///   on the first mismatched byte. Do not use `PartialEq` for authentication
///   comparisons where timing side channels are a concern; use a constant-time
///   comparison function instead (e.g., `subtle::ConstantTimeEq` or
///   `argon2::password_hash::PasswordVerifier`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

// ── SeaORM integration ──────────────────────────────────────────────

#[cfg(feature = "sea-orm")]
mod sea_orm_impl {
    use super::SecretString;
    use sea_orm::entity::prelude::*;
    use sea_orm::sea_query::ValueType;
    use sea_orm::{TryGetError, TryGetable};

    impl From<SecretString> for Value {
        fn from(s: SecretString) -> Self {
            Value::String(Some(s.expose_secret().to_string()))
        }
    }

    impl TryGetable for SecretString {
        fn try_get_by<I: sea_orm::ColIdx>(
            res: &QueryResult,
            index: I,
        ) -> std::result::Result<Self, TryGetError> {
            // Delegate to Option<String> to correctly handle NULL values
            // from all database backends (SQLite returns DbErr::Type for
            // NULL on custom types, which the String impl handles).
            match <Option<String> as TryGetable>::try_get_by(res, index) {
                Ok(Some(val)) => Ok(SecretString::new(val)),
                Ok(None) => Err(TryGetError::Null(std::string::ToString::to_string(
                    &index.as_str().unwrap_or(""),
                ))),
                Err(e) => Err(e),
            }
        }
    }

    impl ValueType for SecretString {
        fn try_from(v: Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
            match v {
                Value::String(Some(s)) => Ok(SecretString::new(s)),
                _ => Err(sea_orm::sea_query::ValueTypeErr),
            }
        }

        fn type_name() -> String {
            "SecretString".to_string()
        }

        fn array_type() -> sea_orm::sea_query::ArrayType {
            sea_orm::sea_query::ArrayType::String
        }

        fn column_type() -> sea_orm::ColumnType {
            sea_orm::ColumnType::String(sea_orm::sea_query::StringLen::None)
        }
    }

    impl sea_orm::sea_query::Nullable for SecretString {
        fn null() -> Value {
            Value::String(None)
        }
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

    #[cfg(feature = "sea-orm")]
    #[test]
    fn sea_orm_value_roundtrip() {
        use sea_orm::entity::prelude::*;
        use sea_orm::sea_query::ValueType;
        let s = SecretString::new("db-secret".into());
        let value: Value = s.into();
        let recovered =
            <SecretString as ValueType>::try_from(value).expect("should recover");
        assert_eq!(recovered.expose_secret(), "db-secret");
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn sea_orm_nullable() {
        use sea_orm::sea_query::Nullable;
        let null_val = SecretString::null();
        assert!(matches!(null_val, sea_orm::Value::String(None)));
    }
}
