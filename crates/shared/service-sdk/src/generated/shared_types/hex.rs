// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Minimal hex encoding/decoding helpers.
//!
//! Replaces the external `hex` crate with a zero-dependency implementation.
use std::fmt::Write;
/// Hex-encode a byte slice into a lowercase hex string.
pub fn encode(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
/// Error returned when hex decoding fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The input has an odd number of characters.
    OddLength,
    /// The input contains a non-hex character.
    InvalidChar,
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OddLength => f.write_str("odd-length hex string"),
            Self::InvalidChar => f.write_str("invalid hex character"),
        }
    }
}
impl std::error::Error for DecodeError {}
/// Decode a hex string into bytes.
pub fn decode(s: &str) -> Result<Vec<u8>, DecodeError> {
    if !s.is_ascii() {
        return Err(DecodeError::InvalidChar);
    }
    if !s.len().is_multiple_of(2) {
        return Err(DecodeError::OddLength);
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| DecodeError::InvalidChar))
        .collect()
}
