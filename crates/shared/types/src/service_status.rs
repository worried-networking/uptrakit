use std::fmt;

use serde::{Deserialize, Serialize};

/// Status of a service in the enrollment/approval workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(
    feature = "sea-orm",
    sea_orm(rs_type = "String", db_type = "Text")
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ServiceStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "approved"))]
    Approved,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "rejected"))]
    Rejected,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "deactivated"))]
    Deactivated,
}

impl fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ServiceStatus {
    /// Returns the string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Deactivated => "deactivated",
        }
    }

    /// Parses a string into a `ServiceStatus` variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "deactivated" => Some(Self::Deactivated),
            _ => None,
        }
    }
}

/// Error returned when parsing an invalid service status string.
#[derive(Debug)]
pub struct ParseServiceStatusError;

impl fmt::Display for ParseServiceStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid service status value")
    }
}

impl std::error::Error for ParseServiceStatusError {}

impl std::str::FromStr for ServiceStatus {
    type Err = ParseServiceStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or(ParseServiceStatusError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ServiceStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_matches_as_str() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            assert_eq!(format!("{variant}"), variant.as_str());
        }
    }

    #[test]
    fn parse_round_trip() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            let s = variant.as_str();
            let parsed = ServiceStatus::parse(s);
            assert_eq!(parsed, Some(variant));
        }
    }

    #[test]
    fn parse_unknown_returns_none() {
        assert_eq!(ServiceStatus::parse("unknown"), None);
        assert_eq!(ServiceStatus::parse(""), None);
    }

    #[test]
    fn from_str_round_trip() {
        for variant in [
            ServiceStatus::Pending,
            ServiceStatus::Approved,
            ServiceStatus::Rejected,
            ServiceStatus::Deactivated,
        ] {
            let s = variant.as_str();
            let parsed: ServiceStatus = s.parse().unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn from_str_invalid_returns_err() {
        assert!("unknown".parse::<ServiceStatus>().is_err());
    }
}
