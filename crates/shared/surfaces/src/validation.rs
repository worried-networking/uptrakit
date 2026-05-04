//! Shared icon-name validation used by the wire layer and by
//! `InteractionDescriptor::validate_for_provider`.

use thiserror::Error;

/// Maximum length (in bytes) of a Lucide-canonical icon name.
pub const MAX_ICON_NAME_LEN: usize = 64;

/// Errors reported by [`validate_icon_name`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IconNameError {
    #[error("icon name must not be empty")]
    Empty,
    #[error("icon name exceeds {MAX_ICON_NAME_LEN} characters")]
    TooLong,
    #[error("icon name must match lucide kebab-case (lowercase letters, digits, hyphens)")]
    InvalidFormat,
}

/// Validates a candidate Lucide icon name.
///
/// Accepts ASCII kebab-case identifiers matching the regex
/// `^[a-z][a-z0-9-]*[a-z0-9]$` (≥ 2 chars, no leading/trailing dash).
///
/// # Errors
///
/// Returns [`IconNameError::Empty`] for empty strings,
/// [`IconNameError::TooLong`] for inputs longer than [`MAX_ICON_NAME_LEN`],
/// and [`IconNameError::InvalidFormat`] for any other shape mismatch.
pub fn validate_icon_name(name: &str) -> Result<(), IconNameError> {
    if name.is_empty() {
        return Err(IconNameError::Empty);
    }
    if name.len() > MAX_ICON_NAME_LEN {
        return Err(IconNameError::TooLong);
    }
    // Slice destructuring satisfies clippy::unwrap_used = deny and
    // clippy::indexing_slicing = deny without an #[expect(...)] escape:
    // the empty- and single-element cases fall through to the `else`.
    let bytes = name.as_bytes();
    let [first, middle @ .., last] = bytes else {
        return Err(IconNameError::InvalidFormat);
    };
    if !first.is_ascii_lowercase() {
        return Err(IconNameError::InvalidFormat);
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(IconNameError::InvalidFormat);
    }
    for &byte in middle {
        let allowed = byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
        if !allowed {
            return Err(IconNameError::InvalidFormat);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_icon_name_accepts_kebab_case() {
        validate_icon_name("trash-2").unwrap();
        validate_icon_name("refresh-cw").unwrap();
        validate_icon_name("server-cog").unwrap();
        validate_icon_name("plug-zap").unwrap();
        validate_icon_name("box").unwrap();
        validate_icon_name("ab").unwrap();
    }

    #[test]
    fn validate_icon_name_rejects_empty() {
        assert_eq!(validate_icon_name(""), Err(IconNameError::Empty));
    }

    #[test]
    fn validate_icon_name_rejects_oversized() {
        let long = "a".repeat(MAX_ICON_NAME_LEN + 1);
        assert_eq!(validate_icon_name(&long), Err(IconNameError::TooLong));
    }

    #[test]
    fn validate_icon_name_rejects_pascal_case() {
        assert_eq!(
            validate_icon_name("Trash2"),
            Err(IconNameError::InvalidFormat)
        );
        assert_eq!(
            validate_icon_name("Package"),
            Err(IconNameError::InvalidFormat)
        );
    }

    #[test]
    fn validate_icon_name_rejects_underscore() {
        assert_eq!(
            validate_icon_name("trash_2"),
            Err(IconNameError::InvalidFormat)
        );
    }

    #[test]
    fn validate_icon_name_rejects_leading_or_trailing_dash() {
        assert_eq!(
            validate_icon_name("-trash"),
            Err(IconNameError::InvalidFormat)
        );
        assert_eq!(
            validate_icon_name("trash-"),
            Err(IconNameError::InvalidFormat)
        );
    }

    #[test]
    fn validate_icon_name_rejects_single_char() {
        assert_eq!(validate_icon_name("x"), Err(IconNameError::InvalidFormat));
    }

    #[test]
    fn validate_icon_name_rejects_uppercase_or_punctuation_in_middle() {
        assert_eq!(
            validate_icon_name("traSh-2"),
            Err(IconNameError::InvalidFormat)
        );
        assert_eq!(
            validate_icon_name("trash 2"),
            Err(IconNameError::InvalidFormat)
        );
    }
}
