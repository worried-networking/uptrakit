//! A URL safety wrapper that masks embedded credentials in all outputs.
//!
//! `Display`, `Debug`, and `Serialize` always redact the password component
//! so the value is safe to log and return in API responses. The raw URL is
//! accessible via [`MaskedUrl::as_raw_str`] for internal use (e.g. connecting
//! to NATS, encrypting before storing).

use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A URL that may contain embedded credentials.
///
/// `Display`, `Debug`, and `Serialize` always emit the password portion
/// replaced with `***` so that the value is safe to log and return in API
/// responses. The raw URL is accessible via [`as_raw_str`](Self::as_raw_str)
/// for internal use (connecting, encrypting, storing).
///
/// The inner `String` is zeroed (overwritten with zeros) when the value is
/// dropped, preventing credentials from lingering in freed heap memory.
#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MaskedUrl(String);

impl MaskedUrl {
    /// Wrap a raw URL string (may contain embedded credentials).
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// Returns the raw URL, including any embedded password.
    ///
    /// Use this only for internal operations (NATS connection, encryption).
    /// Never log or return this value directly over the API.
    pub fn as_raw_str(&self) -> &str {
        &self.0
    }

    /// Returns the URL with the password component replaced with `***`.
    ///
    /// If no password is present the URL is returned unchanged.
    pub fn masked(&self) -> String {
        mask_url_password(&self.0)
    }
}

impl fmt::Display for MaskedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.masked())
    }
}

impl fmt::Debug for MaskedUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MaskedUrl(\"{}\")", self.masked())
    }
}

impl Serialize for MaskedUrl {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.masked())
    }
}

impl<'de> Deserialize<'de> for MaskedUrl {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self(s))
    }
}

/// Replace the password component of a URL with `***`.
///
/// Handles the `scheme://user:password@host` pattern. If the URL has no
/// embedded password, the string is returned unchanged.
fn mask_url_password(url: &str) -> String {
    // Find the scheme separator "://"
    let Some(after_scheme) = url.find("://").map(|i| i + 3) else {
        return url.to_string();
    };

    let authority = &url[after_scheme..];

    // Look for '@' which separates user-info from host
    let Some(at_pos) = authority.find('@') else {
        return url.to_string();
    };

    let user_info = &authority[..at_pos];

    // Look for ':' separating user from password
    let Some(colon_pos) = user_info.find(':') else {
        // Has user but no password — nothing to mask
        return url.to_string();
    };

    let scheme_and_user = &url[..after_scheme + colon_pos + 1]; // up to and including the ':'
    let rest = &url[after_scheme + at_pos..]; // '@' and everything after

    format!("{scheme_and_user}***{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_url_with_password() {
        let url = MaskedUrl::new("nats://user:secret@host:4222");
        assert_eq!(url.masked(), "nats://user:***@host:4222");
        assert_eq!(url.to_string(), "nats://user:***@host:4222");
        assert_eq!(url.as_raw_str(), "nats://user:secret@host:4222");
    }

    #[test]
    fn mask_url_no_password() {
        let url = MaskedUrl::new("nats://host:4222");
        assert_eq!(url.masked(), "nats://host:4222");
    }

    #[test]
    fn mask_url_user_no_password() {
        let url = MaskedUrl::new("nats://user@host:4222");
        assert_eq!(url.masked(), "nats://user@host:4222");
    }

    #[test]
    fn mask_url_no_scheme() {
        let url = MaskedUrl::new("host:4222");
        assert_eq!(url.masked(), "host:4222");
    }

    #[test]
    fn debug_redacts_password() {
        let url = MaskedUrl::new("nats://admin:hunter2@nats.internal:4222");
        let debug = format!("{url:?}");
        assert!(!debug.contains("hunter2"), "debug must not expose password");
        assert!(debug.contains("***"));
    }

    #[test]
    fn serialize_redacts_password() {
        let url = MaskedUrl::new("nats://admin:hunter2@nats.internal:4222");
        let json = serde_json::to_string(&url).expect("serialize");
        assert!(!json.contains("hunter2"));
        assert!(json.contains("***"));
    }

    #[test]
    fn deserialize_roundtrip_preserves_raw() {
        let raw = "nats://admin:hunter2@nats.internal:4222";
        let url = MaskedUrl::new(raw);
        // Deserialize from the raw string (as sent in API requests)
        let from_json: MaskedUrl =
            serde_json::from_str(&format!("\"{raw}\"")).expect("deserialize");
        assert_eq!(from_json.as_raw_str(), raw);
        // The original is unaffected
        assert_eq!(url.as_raw_str(), raw);
    }

    #[test]
    fn equality() {
        let a = MaskedUrl::new("nats://user:pw@host:4222");
        let b = MaskedUrl::new("nats://user:pw@host:4222");
        let c = MaskedUrl::new("nats://host:4222");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
