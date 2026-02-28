use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Discovery lifecycle state for software items created by autodiscovery.
///
/// Items created manually have `discovery_state = NULL`.
/// Items created by discovery are initially `pending` and must be explicitly
/// approved or deleted by the user.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SoftwareDiscoveryState {
    /// Discovered but not yet reviewed by the user.
    /// The item is disabled and excluded from version checks.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "pending"))]
    Pending,
    /// Approved by the user; version tracking and updates are active.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "approved"))]
    Approved,
}

/// Error returned when parsing an invalid [`SoftwareDiscoveryState`] string.
#[derive(Debug, Error)]
pub enum ParseSoftwareDiscoveryStateError {
    /// The input string does not match any known state.
    #[error("invalid software discovery state value")]
    Invalid,
}

impl fmt::Display for SoftwareDiscoveryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
        })
    }
}

impl FromStr for SoftwareDiscoveryState {
    type Err = ParseSoftwareDiscoveryStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            _ => Err(ParseSoftwareDiscoveryStateError::Invalid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [
            SoftwareDiscoveryState::Pending,
            SoftwareDiscoveryState::Approved,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: SoftwareDiscoveryState = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_values() {
        assert_eq!(SoftwareDiscoveryState::Pending.to_string(), "pending");
        assert_eq!(SoftwareDiscoveryState::Approved.to_string(), "approved");
    }

    #[test]
    fn from_str_valid() {
        assert_eq!(
            "pending".parse::<SoftwareDiscoveryState>().ok(),
            Some(SoftwareDiscoveryState::Pending)
        );
        assert_eq!(
            "approved".parse::<SoftwareDiscoveryState>().ok(),
            Some(SoftwareDiscoveryState::Approved)
        );
    }

    #[test]
    fn from_str_invalid() {
        assert!("unknown".parse::<SoftwareDiscoveryState>().is_err());
        assert!("".parse::<SoftwareDiscoveryState>().is_err());
        assert!("Pending".parse::<SoftwareDiscoveryState>().is_err());
    }
}
