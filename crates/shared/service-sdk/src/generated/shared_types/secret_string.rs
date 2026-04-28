// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};
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
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
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
