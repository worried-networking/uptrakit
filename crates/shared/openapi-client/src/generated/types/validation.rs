// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use std::fmt;
/// Error returned when request field validation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: &'static str,
    pub message: String,
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}
impl std::error::Error for ValidationError {}
/// Trait for validating request types before processing.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}
