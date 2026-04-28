// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! A URL safety wrapper that masks embedded credentials in all outputs.
//!
//! `Display`, `Debug`, and `Serialize` always redact the password component
//! so the value is safe to log and return in API responses. The raw URL is
//! accessible via [`MaskedUrl::as_raw_str`] for internal use (e.g. connecting
//! to NATS, encrypting before storing).
use serde::{Deserialize, Serialize};
use std::fmt;
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
    let Some(after_scheme) = url.find("://").map(|i| i + 3) else {
        return url.to_string();
    };
    let authority = &url[after_scheme..];
    let Some(at_pos) = authority.find('@') else {
        return url.to_string();
    };
    let user_info = &authority[..at_pos];
    let Some(colon_pos) = user_info.find(':') else {
        return url.to_string();
    };
    let scheme_and_user = &url[..after_scheme + colon_pos + 1];
    let rest = &url[after_scheme + at_pos..];
    format!("{scheme_and_user}***{rest}")
}
