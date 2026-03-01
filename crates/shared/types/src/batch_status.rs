use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Status of an update batch.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    /// The batch has updates still pending or in progress.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "in_progress"))]
    InProgress,
    /// All updates in the batch completed successfully.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "completed"))]
    Completed,
    /// All updates finished but at least one failed.
    #[cfg_attr(
        feature = "sea-orm",
        sea_orm(string_value = "partially_completed")
    )]
    PartiallyCompleted,
}

impl fmt::Display for BatchStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BatchStatus {
    /// Returns the string representation.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::PartiallyCompleted => "partially_completed",
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: BatchStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            BatchStatus::InProgress,
            BatchStatus::Completed,
            BatchStatus::PartiallyCompleted,
        ] {
            let s = variant.as_str();
            let parsed: BatchStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<BatchStatus>().is_err());
        assert!("".parse::<BatchStatus>().is_err());
    }
}
