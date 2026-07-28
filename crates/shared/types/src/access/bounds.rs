//! Bounded-size constants for the access model
//! (`09-resolved-questions.md` §Grant model, owner-resolved 2026-07-21).

use super::pattern::ActionPattern;

/// Maximum number of patterns per grant row.
pub const MAX_PATTERNS_PER_GRANT: usize = 16;
/// Maximum action/pattern string length in bytes (shared bound; test A9).
pub const MAX_PATTERN_LEN: usize = 64;
/// Maximum grant description length.
pub const MAX_GRANT_DESCRIPTION_LEN: usize = 500;
/// Maximum tag IDs per `Selector::Tags`.
pub const MAX_SELECTOR_TAG_IDS: usize = 32;
/// Maximum host IDs per `Selector::Hosts` (aligned with the batch-action maximum).
pub const MAX_SELECTOR_HOST_IDS: usize = 100;
/// Maximum software-item IDs per `Selector::Software`.
pub const MAX_SELECTOR_SOFTWARE_IDS: usize = 100;
/// Maximum host-software-item IDs per `Selector::Items`.
pub const MAX_SELECTOR_ITEM_IDS: usize = 100;
/// Maximum grant rows per subject.
pub const MAX_GRANTS_PER_SUBJECT: usize = 200;

/// Error returned when a grant's pattern set violates its bounds or
/// contains an unmatchable pattern (validation rule 1).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternSetError {
    /// More patterns than [`MAX_PATTERNS_PER_GRANT`].
    #[error("too many patterns: {actual} exceeds the maximum of {max}")]
    TooMany { max: usize, actual: usize },
    /// A pattern is provably unmatchable (validation rule 1).
    #[error("pattern `{pattern}` can match no valid action")]
    Unmatchable { index: usize, pattern: String },
}

/// Write-time validation of a grant's pattern list: count bound plus
/// per-pattern matchability (rule 1 of `06-grant-model.md`).
pub fn validate_patterns(patterns: &[ActionPattern]) -> Result<(), PatternSetError> {
    if patterns.len() > MAX_PATTERNS_PER_GRANT {
        return Err(PatternSetError::TooMany {
            max: MAX_PATTERNS_PER_GRANT,
            actual: patterns.len(),
        });
    }
    for (index, pattern) in patterns.iter().enumerate() {
        if !pattern.can_match_any() {
            return Err(PatternSetError::Unmatchable {
                index,
                pattern: pattern.to_string(),
            });
        }
    }
    Ok(())
}
