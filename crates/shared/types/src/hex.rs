//! Minimal hex encoding/decoding helpers.
//!
//! Replaces the external `hex` crate with a zero-dependency implementation.

use std::fmt::Write;

/// Hex-encode a byte slice into a lowercase hex string.
#[expect(
    clippy::unused_result_ok,
    reason = "writing to a String via fmt::Write is infallible; .ok() discards a Result that is always Ok(())"
)]
pub fn encode(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        // Writing to a `String` via `fmt::Write` is infallible.
        write!(s, "{b:02x}").ok();
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
    #[expect(
        clippy::string_slice,
        reason = "boundary is safe: s.is_ascii() is checked above, so every byte is a single-char boundary"
    )]
    #[expect(
        clippy::map_err_ignore,
        reason = "ParseIntError carries no additional context beyond 'invalid hex digit'; the domain error is sufficient"
    )]
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| DecodeError::InvalidChar))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_empty() {
        assert_eq!(encode([]), "");
    }

    #[test]
    fn encode_bytes() {
        assert_eq!(encode([0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn encode_leading_zeros() {
        assert_eq!(encode([0x00, 0x01, 0x0f]), "00010f");
    }

    #[test]
    fn decode_empty() {
        assert_eq!(decode("").as_deref(), Ok(&[][..]));
    }

    #[test]
    fn decode_valid() {
        assert_eq!(decode("deadbeef"), Ok(vec![0xde, 0xad, 0xbe, 0xef]));
    }

    #[test]
    fn decode_uppercase() {
        assert_eq!(decode("DEADBEEF"), Ok(vec![0xde, 0xad, 0xbe, 0xef]));
    }

    #[test]
    fn decode_odd_length() {
        assert_eq!(decode("abc"), Err(DecodeError::OddLength));
    }

    #[test]
    fn decode_invalid_char() {
        assert_eq!(decode("zz"), Err(DecodeError::InvalidChar));
    }

    #[test]
    fn decode_non_ascii_multibyte() {
        // Multi-byte UTF-8 input must not panic (would slice mid-character without guard)
        assert_eq!(decode("caf\u{00e9}"), Err(DecodeError::InvalidChar));
        assert_eq!(decode("\u{1f600}\u{1f600}"), Err(DecodeError::InvalidChar));
        assert_eq!(decode("\u{4e16}\u{754c}"), Err(DecodeError::InvalidChar));
    }

    #[test]
    fn roundtrip() {
        let original = vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        assert_eq!(decode(&encode(&original)), Ok(original));
    }
}
