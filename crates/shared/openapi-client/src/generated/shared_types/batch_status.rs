// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
/// Status of an update batch.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for status strings received from a
/// newer peer that this binary does not yet know about. Serde deserialization
/// is infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older clients to survive rolling upgrades without dropping
/// entire messages.
///
/// `FromStr` remains strict for DB lookups and URL parameters where callers
/// need to distinguish known variants from unknown ones.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchStatus {
    /// The batch has updates still pending or in progress.
    InProgress,
    /// All updates in the batch completed successfully.
    Completed,
    /// All updates finished but at least one failed.
    PartiallyCompleted,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}
impl BatchStatus {
    /// Returns the string representation.
    ///
    /// For [`BatchStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::PartiallyCompleted => "partially_completed",
            Self::Other(s) => s.as_str(),
        }
    }
}
impl fmt::Display for BatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
/// Error returned when parsing an invalid batch status string.
#[derive(Debug)]
pub struct ParseBatchStatusError;
impl fmt::Display for ParseBatchStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid batch status value")
    }
}
impl std::error::Error for ParseBatchStatusError {}
impl FromStr for BatchStatus {
    type Err = ParseBatchStatusError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "partially_completed" => Ok(Self::PartiallyCompleted),
            _ => Err(ParseBatchStatusError),
        }
    }
}
impl From<String> for BatchStatus {
    /// Converts a snake_case string to a batch status.
    ///
    /// Unknown strings map to [`BatchStatus::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "partially_completed" => Self::PartiallyCompleted,
            _ => Self::Other(s),
        }
    }
}
impl Serialize for BatchStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for BatchStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(BatchStatus::from)
    }
}
