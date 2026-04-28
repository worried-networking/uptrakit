use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::Hash;
use std::str::FromStr;
/// A newtype wrapper for email addresses that masks the local part in
/// `Debug` and `Display` output while preserving the full value for
/// serialization and database storage.
///
/// # Not a secret
///
/// `MaskedEmail` is **not** a cryptographic secret. Email addresses are stored
/// in cleartext in the database and are routinely displayed to operators in
/// logs and the UI. The masking is purely a privacy UX feature (reducing the
/// exposure of full email addresses in log output), not a security control.
/// `ZeroizeOnDrop` is therefore not applied — it would impose runtime cost for
/// no security benefit.
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
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    pub fn new(email: impl Into<String>) -> Self {
        Self(email.into())
    }
    /// Borrow the full, unmasked email address.
    pub fn expose_email(&self) -> &str {
        &self.0
    }
}
impl FromStr for MaskedEmail {
    type Err = ParseMaskedEmailError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let at_pos = s.find('@').ok_or(ParseMaskedEmailError)?;
        let (local, domain) = s.split_at(at_pos);
        let domain = &domain[1..];
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
        return mask_segment(email);
    };
    let (local, rest) = email.split_at(at_pos);
    let domain = &rest[1..];
    let mut result = String::with_capacity(email.len() + 8);
    let mut segment_start = 0;
    for (i, ch) in local.char_indices() {
        if ch == '.' || ch == '_' || ch == '+' || ch == '-' {
            result.push_str(&mask_segment(&local[segment_start..i]));
            result.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
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
