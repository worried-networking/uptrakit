use std::fmt;
use std::hash::Hash;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A newtype wrapper for email addresses that masks the local part in
/// `Debug` and `Display` output while preserving the full value for
/// serialization and database storage.
///
/// The inner `String` is zeroed when the value is dropped.
///
/// # Masking algorithm
///
/// The local part (before `@`) is split into segments by delimiters
/// (`.`, `_`, `+`, `-`). Each segment shows `ceil(len/3)` leading
/// characters (minimum 1) followed by `***`. The domain is always
/// shown in full. Original delimiters are preserved.
///
/// # Examples
///
/// - `andrey@example.com` → `an***@example.com`
/// - `andrey.johnson@example.org` → `an***.joh***@example.org`
/// - `john_doe@example.com` → `jo***_d***@example.com`
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct MaskedEmail(String);

/// Error returned when parsing an email address fails.
#[derive(Debug, Clone)]
pub struct ParseMaskedEmailError;

impl fmt::Display for ParseMaskedEmailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid email address")
    }
}

impl std::error::Error for ParseMaskedEmailError {}

impl MaskedEmail {
    /// Wrap a raw email address.
    pub fn new(email: String) -> Self {
        Self(email)
    }

    /// Borrow the full, unmasked email address.
    pub fn expose_email(&self) -> &str {
        &self.0
    }
}

impl FromStr for MaskedEmail {
    type Err = ParseMaskedEmailError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // Minimal validation: must contain exactly one `@` with non-empty parts.
        let at_pos = s.find('@').ok_or(ParseMaskedEmailError)?;
        let (local, domain) = s.split_at(at_pos);
        let domain = &domain[1..]; // skip '@'
        if local.is_empty() || domain.is_empty() || domain.find('@').is_some() {
            return Err(ParseMaskedEmailError);
        }
        Ok(Self(s.to_string()))
    }
}

impl fmt::Debug for MaskedEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MaskedEmail({})", mask_email(&self.0))
    }
}

impl fmt::Display for MaskedEmail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&mask_email(&self.0))
    }
}

/// Mask the local part of an email address, preserving delimiters and
/// showing `ceil(len/3)` leading characters per segment.
fn mask_email(email: &str) -> String {
    let Some(at_pos) = email.find('@') else {
        // No '@' — mask the whole string.
        return mask_segment(email);
    };
    let (local, rest) = email.split_at(at_pos);
    let domain = &rest[1..]; // skip '@'

    let mut result = String::with_capacity(email.len() + 8);
    let mut segment_start = 0;

    for (i, ch) in local.char_indices() {
        if ch == '.' || ch == '_' || ch == '+' || ch == '-' {
            // Mask the segment before the delimiter.
            result.push_str(&mask_segment(&local[segment_start..i]));
            result.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
    // Mask the final segment.
    result.push_str(&mask_segment(&local[segment_start..]));

    result.push('@');
    result.push_str(domain);
    result
}

/// Mask a single segment: show `ceil(len/3)` leading chars (min 1), then `***`.
fn mask_segment(segment: &str) -> String {
    if segment.is_empty() {
        return "***".to_string();
    }
    let len = segment.chars().count();
    let visible = len.div_ceil(3).max(1);
    let prefix: String = segment.chars().take(visible).collect();
    format!("{prefix}***")
}

#[cfg(feature = "sea-orm")]
mod sea_orm_impl {
    use super::MaskedEmail;
    use sea_orm::entity::prelude::*;
    use sea_orm::sea_query::ValueType;
    use sea_orm::{TryGetError, TryGetable};

    impl From<MaskedEmail> for Value {
        fn from(e: MaskedEmail) -> Self {
            Value::String(Some(e.expose_email().to_string()))
        }
    }

    impl TryGetable for MaskedEmail {
        fn try_get_by<I: sea_orm::ColIdx>(
            res: &QueryResult,
            index: I,
        ) -> std::result::Result<Self, TryGetError> {
            let val: String = res.try_get_by(index)?;
            Ok(MaskedEmail::new(val))
        }
    }

    impl ValueType for MaskedEmail {
        fn try_from(v: Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
            match v {
                Value::String(Some(s)) => Ok(MaskedEmail::new(s)),
                _ => Err(sea_orm::sea_query::ValueTypeErr),
            }
        }

        fn type_name() -> String {
            "MaskedEmail".to_string()
        }

        fn array_type() -> sea_orm::sea_query::ArrayType {
            sea_orm::sea_query::ArrayType::String
        }

        fn column_type() -> sea_orm::ColumnType {
            sea_orm::ColumnType::String(sea_orm::sea_query::StringLen::None)
        }
    }

    impl sea_orm::sea_query::Nullable for MaskedEmail {
        fn null() -> Value {
            Value::String(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_simple_email() {
        let email = MaskedEmail::new("andrey@example.com".into());
        assert_eq!(format!("{email}"), "an***@example.com");
    }

    #[test]
    fn mask_dot_separated() {
        let email = MaskedEmail::new("andrey.johnson@example.org".into());
        assert_eq!(format!("{email}"), "an***.joh***@example.org");
    }

    #[test]
    fn mask_single_char() {
        let email = MaskedEmail::new("a@example.com".into());
        assert_eq!(format!("{email}"), "a***@example.com");
    }

    #[test]
    fn mask_two_char() {
        let email = MaskedEmail::new("ab@example.com".into());
        assert_eq!(format!("{email}"), "a***@example.com");
    }

    #[test]
    fn mask_underscore() {
        let email = MaskedEmail::new("john_doe@example.com".into());
        assert_eq!(format!("{email}"), "jo***_d***@example.com");
    }

    #[test]
    fn mask_plus_tag() {
        let email = MaskedEmail::new("user+tag@example.com".into());
        assert_eq!(format!("{email}"), "us***+t***@example.com");
    }

    #[test]
    fn mask_hyphen() {
        let email = MaskedEmail::new("first-last@example.com".into());
        assert_eq!(format!("{email}"), "fi***-la***@example.com");
    }

    #[test]
    fn debug_shows_masked() {
        let email = MaskedEmail::new("secret@test.com".into());
        let debug = format!("{email:?}");
        assert!(!debug.contains("secret@"));
        assert!(debug.contains("se***@test.com"));
    }

    #[test]
    fn expose_email_returns_full() {
        let email = MaskedEmail::new("user@test.com".into());
        assert_eq!(email.expose_email(), "user@test.com");
    }

    #[test]
    fn serde_roundtrip() {
        let email = MaskedEmail::new("test@example.com".into());
        let json = serde_json::to_string(&email).expect("serialize");
        assert_eq!(json, r#""test@example.com""#);
        let recovered: MaskedEmail = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(recovered.expose_email(), "test@example.com");
    }

    #[test]
    fn from_str_valid() {
        let email: MaskedEmail = "user@domain.com".parse().expect("parse");
        assert_eq!(email.expose_email(), "user@domain.com");
    }

    #[test]
    fn from_str_rejects_no_at() {
        assert!("nodomain".parse::<MaskedEmail>().is_err());
    }

    #[test]
    fn from_str_rejects_empty_local() {
        assert!("@domain.com".parse::<MaskedEmail>().is_err());
    }

    #[test]
    fn from_str_rejects_empty_domain() {
        assert!("user@".parse::<MaskedEmail>().is_err());
    }

    #[test]
    fn from_str_rejects_multiple_at() {
        assert!("a@b@c.com".parse::<MaskedEmail>().is_err());
    }

    #[test]
    fn equality() {
        let a = MaskedEmail::new("same@test.com".into());
        let b = MaskedEmail::new("same@test.com".into());
        let c = MaskedEmail::new("other@test.com".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let a = MaskedEmail::new("test@test.com".into());
        let b = MaskedEmail::new("test@test.com".into());
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

    #[cfg(feature = "sea-orm")]
    #[test]
    fn sea_orm_value_roundtrip() {
        use sea_orm::entity::prelude::*;
        use sea_orm::sea_query::ValueType;
        let e = MaskedEmail::new("db@test.com".into());
        let value: Value = e.into();
        let recovered = <MaskedEmail as ValueType>::try_from(value).expect("should recover");
        assert_eq!(recovered.expose_email(), "db@test.com");
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn sea_orm_nullable() {
        use sea_orm::sea_query::Nullable;
        let null_val = MaskedEmail::null();
        assert!(matches!(null_val, sea_orm::Value::String(None)));
    }
}
