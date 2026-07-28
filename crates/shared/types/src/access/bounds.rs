//! Bounded-size constants for the access model
//! (`09-resolved-questions.md` §Grant model, owner-resolved 2026-07-21).

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
