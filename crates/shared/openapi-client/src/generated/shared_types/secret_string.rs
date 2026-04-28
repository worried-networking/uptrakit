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
