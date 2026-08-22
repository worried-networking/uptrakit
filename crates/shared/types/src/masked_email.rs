use std::fmt;
use std::hash::Hash;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

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
#[serde(try_from = "String")]
pub struct MaskedEmail(String);

/// Maximum stored length of an email address in bytes, measured post-trim.
pub const MAX_EMAIL_LEN: usize = 254;

/// Error returned when parsing an email address fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseMaskedEmailError {
    /// The address contains no `@`.
    #[error("email must contain '@'")]
    MissingAt,
    /// The address contains more than one `@`.
    #[error("email must contain exactly one '@'")]
    MultipleAt,
    /// The part before `@` is empty.
    #[error("email local part must not be empty")]
    EmptyLocal,
    /// The part after `@` is empty.
    #[error("email domain part must not be empty")]
    EmptyDomain,
    /// The trimmed address exceeds [`MAX_EMAIL_LEN`] bytes.
    #[error("email must not exceed {MAX_EMAIL_LEN} bytes")]
    TooLong,
}

impl MaskedEmail {
    /// Borrow the full, unmasked email address.
    pub fn expose_email(&self) -> &str {
        &self.0
    }

    /// Canonical form: trim, then ASCII-lowercase. No validation, no Unicode folding.
    pub fn canonical_form(s: &str) -> String {
        s.trim().to_ascii_lowercase()
    }

    /// Re-canonicalize an already-loaded value without validation.
    ///
    /// Used by the `db-migrate` copy path, which must tolerate stored rows
    /// that the validating [`FromStr`] would reject.
    pub fn canonicalized(&self) -> MaskedEmail {
        MaskedEmail(Self::canonical_form(&self.0))
    }

    /// Wrap a stored value verbatim. DB reads only — no canonicalization.
    /// Private by design: everything else goes through [`FromStr`].
    /// Gated on `sea-orm`: its only callers live in the gated child module,
    /// and an unused private fn is a deny-level warning without that feature.
    #[cfg(feature = "sea-orm")]
    fn from_stored(email: String) -> Self {
        Self(email)
    }
}

impl FromStr for MaskedEmail {
    type Err = ParseMaskedEmailError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let canonical = Self::canonical_form(s);
        if canonical.len() > MAX_EMAIL_LEN {
            return Err(ParseMaskedEmailError::TooLong);
        }
        let at_pos = canonical
            .find('@')
            .ok_or(ParseMaskedEmailError::MissingAt)?;
        let (local, domain) = canonical.split_at(at_pos);
        #[expect(
            clippy::string_slice,
            reason = "safe: skipping '@' which is ASCII (1 byte), so index 1 is always a valid boundary"
        )]
        let domain = &domain[1..]; // skip '@'
        if local.is_empty() {
            return Err(ParseMaskedEmailError::EmptyLocal);
        }
        if domain.is_empty() {
            return Err(ParseMaskedEmailError::EmptyDomain);
        }
        if domain.find('@').is_some() {
            return Err(ParseMaskedEmailError::MultipleAt);
        }
        Ok(Self(canonical))
    }
}

impl TryFrom<String> for MaskedEmail {
    type Error = ParseMaskedEmailError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        value.parse()
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
    #[expect(
        clippy::string_slice,
        reason = "safe: skipping '@' which is ASCII (1 byte), so index 1 is always a valid boundary"
    )]
    let domain = &rest[1..]; // skip '@'

    let mut result = String::with_capacity(email.len() + 8);
    let mut segment_start = 0;

    for (i, ch) in local.char_indices() {
        if ch == '.' || ch == '_' || ch == '+' || ch == '-' {
            // Mask the segment before the delimiter.
            // SAFETY for string_slice: `i` comes from `char_indices()` (always a valid
            // char boundary); `segment_start` is always `i + ch.len_utf8()` from a prior
            // iteration or 0, so both indices are valid UTF-8 char boundaries.
            #[expect(
                clippy::string_slice,
                reason = "both bounds come from char_indices() and ch.len_utf8(), guaranteeing valid char boundaries"
            )]
            result.push_str(&mask_segment(&local[segment_start..i]));
            result.push(ch);
            segment_start = i + ch.len_utf8();
        }
    }
    // Mask the final segment.
    #[expect(
        clippy::string_slice,
        reason = "segment_start is always set to i + ch.len_utf8() from char_indices(), guaranteeing a valid char boundary"
    )]
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

    impl From<&MaskedEmail> for Value {
        fn from(e: &MaskedEmail) -> Self {
            Value::String(Some(e.expose_email().to_string()))
        }
    }

    impl TryGetable for MaskedEmail {
        fn try_get_by<I: sea_orm::ColIdx>(
            res: &QueryResult,
            index: I,
        ) -> std::result::Result<Self, TryGetError> {
            let val: String = res.try_get_by(index)?;
            Ok(MaskedEmail::from_stored(val))
        }
    }

    impl ValueType for MaskedEmail {
        fn try_from(v: Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
            match v {
                Value::String(Some(s)) => Ok(MaskedEmail::from_stored(s)),
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — is_ok/is_err provides readable failure messages"
    )]
    use super::*;

    #[test]
    fn mask_simple_email() {
        let email = "andrey@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "an***@example.com");
    }

    #[test]
    fn mask_dot_separated() {
        let email = "andrey.johnson@example.org"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "an***.joh***@example.org");
    }

    #[test]
    fn mask_single_char() {
        let email = "a@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "a***@example.com");
    }

    #[test]
    fn mask_two_char() {
        let email = "ab@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "a***@example.com");
    }

    #[test]
    fn mask_underscore() {
        let email = "john_doe@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "jo***_d***@example.com");
    }

    #[test]
    fn mask_plus_tag() {
        let email = "user+tag@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "us***+t***@example.com");
    }

    #[test]
    fn mask_hyphen() {
        let email = "first-last@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(format!("{email}"), "fi***-la***@example.com");
    }

    #[test]
    fn debug_shows_masked() {
        let email = "secret@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        let debug = format!("{email:?}");
        assert!(!debug.contains("secret@"));
        assert!(debug.contains("se***@test.com"));
    }

    #[test]
    fn expose_email_returns_full() {
        let email = "user@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(email.expose_email(), "user@test.com");
    }

    #[test]
    fn serde_roundtrip() {
        let email = "test@example.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
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
    fn deserialize_canonicalizes() {
        let e: MaskedEmail = serde_json::from_str(r#"" User@Example.COM ""#).expect("valid");
        assert_eq!(e.expose_email(), "user@example.com");
    }

    #[test]
    fn serialize_stays_plaintext() {
        let e: MaskedEmail = "user@example.com".parse().expect("valid");
        assert_eq!(
            serde_json::to_string(&e).expect("serialize"),
            r#""user@example.com""#
        );
    }

    #[test]
    fn canonical_form_is_idempotent() {
        let once = MaskedEmail::canonical_form(" User@Example.COM ");
        assert_eq!(MaskedEmail::canonical_form(&once), once);
    }

    #[cfg(feature = "sea-orm")]
    #[test]
    fn canonicalized_recanonicalizes_without_validation() {
        // `MaskedEmail::new` no longer exists; the non-canonicalizing read
        // path is exercised via the sea-orm Value/ValueType round trip
        // instead, same as db_read_preserves_stored_bytes. "NoAtSign" is a
        // value the validating FromStr would reject.
        use sea_orm::sea_query::ValueType;
        let value = sea_orm::Value::String(Some("NoAtSign".to_string()));
        let stored = <MaskedEmail as ValueType>::try_from(value).expect("should recover");
        assert_eq!(stored.canonicalized().expose_email(), "noatsign");
    }

    #[test]
    fn non_ascii_case_is_preserved() {
        // ASCII letters fold; the non-ASCII 'Ü' is deliberately preserved
        // (the spec's own example line writes `ÜSER@example.com`, which is
        // wrong — S/E/R are ASCII and fold; the invariant text is authoritative).
        let e: MaskedEmail = "ÜSER@Example.com".parse().expect("valid");
        assert_eq!(e.expose_email(), "Üser@example.com");
    }

    #[test]
    fn from_str_trims() {
        let e: MaskedEmail = "  a@b.c  ".parse().expect("valid");
        assert_eq!(e.expose_email(), "a@b.c");
    }

    #[test]
    fn from_str_rejects_no_at() {
        assert_eq!(
            "nodomain".parse::<MaskedEmail>(),
            Err(ParseMaskedEmailError::MissingAt)
        );
    }

    #[test]
    fn from_str_rejects_multiple_at() {
        assert_eq!(
            "a@b@c.com".parse::<MaskedEmail>(),
            Err(ParseMaskedEmailError::MultipleAt)
        );
    }

    #[test]
    fn from_str_rejects_empty_local() {
        assert_eq!(
            "@domain.com".parse::<MaskedEmail>(),
            Err(ParseMaskedEmailError::EmptyLocal)
        );
    }

    #[test]
    fn from_str_rejects_empty_domain() {
        assert_eq!(
            "user@".parse::<MaskedEmail>(),
            Err(ParseMaskedEmailError::EmptyDomain)
        );
    }

    #[test]
    fn length_cap_is_measured_post_trim() {
        // Derived from MAX_EMAIL_LEN by name (never a bare literal): the local
        // part alone exceeds the cap only before trimming is irrelevant — the
        // canonical (trimmed) form must exceed MAX_EMAIL_LEN to trip TooLong,
        // and a trimmed-to-fit value must pass.
        let too_long = format!("{}@x.com", "a".repeat(MAX_EMAIL_LEN));
        assert_eq!(
            too_long.parse::<MaskedEmail>(),
            Err(ParseMaskedEmailError::TooLong)
        );
        let fits_after_trim = format!("   {}@x.com   ", "a".repeat(MAX_EMAIL_LEN - 6));
        assert!(fits_after_trim.parse::<MaskedEmail>().is_ok());
    }

    #[test]
    fn equality() {
        let a = "same@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        let b = "same@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        let c = "other@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn hash_consistent() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hasher;
        let a = "test@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
        let b = "test@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
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
        let e = "db@test.com"
            .parse::<MaskedEmail>()
            .expect("valid test email");
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

    #[cfg(feature = "sea-orm")]
    #[test]
    fn db_read_preserves_stored_bytes() {
        use sea_orm::sea_query::ValueType;
        let value = sea_orm::Value::String(Some("MiXeD@Case.COM".to_string()));
        let loaded = <MaskedEmail as ValueType>::try_from(value).expect("should recover");
        assert_eq!(loaded.expose_email(), "MiXeD@Case.COM");
    }
}
