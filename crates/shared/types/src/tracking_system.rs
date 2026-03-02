use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Identifies which tracking system a piece of software belongs to.
///
/// Uptrakit supports two complementary tracking systems that coexist
/// independently:
///
/// - **Targeted**: specific software items tracked across hosts (Docker images,
///   GitHub releases, promoted packages). Shown in the main Software list.
/// - **HostManaged**: per-host system packages discovered by package managers
///   (APT, Homebrew, npm). Shown as aggregate counts on the Hosts page with
///   drill-down capability.
///
/// The same package can exist in both systems simultaneously — for example,
/// nginx tracked as a targeted item *and* as a host package. Updates triggered
/// through either system work independently.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(
    feature = "sea-orm",
    derive(strum::EnumIter, sea_orm::DeriveActiveEnum)
)]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TrackingSystem {
    /// Specific software items tracked across hosts.
    ///
    /// Examples: Docker images, GitHub releases, PHS apps, promoted packages.
    /// Stored in `software_items` + `host_software_items`.
    #[default]
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "targeted"))]
    Targeted,
    /// Per-host system packages discovered by package managers.
    ///
    /// Examples: APT packages, Homebrew formulae, npm global packages.
    /// Stored in `host_packages`.
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "host_managed"))]
    HostManaged,
}

/// Error returned when parsing an invalid [`TrackingSystem`] string.
#[derive(Debug, Error)]
pub enum ParseTrackingSystemError {
    /// The input string does not match any known tracking system.
    #[error("invalid tracking system value")]
    Invalid,
}

impl fmt::Display for TrackingSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Targeted => "targeted",
            Self::HostManaged => "host_managed",
        })
    }
}

impl FromStr for TrackingSystem {
    type Err = ParseTrackingSystemError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "targeted" => Ok(Self::Targeted),
            "host_managed" => Ok(Self::HostManaged),
            _ => Err(ParseTrackingSystemError::Invalid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        for variant in [TrackingSystem::Targeted, TrackingSystem::HostManaged] {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: TrackingSystem = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn display_values() {
        assert_eq!(TrackingSystem::Targeted.to_string(), "targeted");
        assert_eq!(TrackingSystem::HostManaged.to_string(), "host_managed");
    }

    #[test]
    fn from_str_valid() {
        assert_eq!(
            "targeted".parse::<TrackingSystem>().ok(),
            Some(TrackingSystem::Targeted)
        );
        assert_eq!(
            "host_managed".parse::<TrackingSystem>().ok(),
            Some(TrackingSystem::HostManaged)
        );
    }

    #[test]
    fn from_str_invalid() {
        assert!("".parse::<TrackingSystem>().is_err());
        assert!("Targeted".parse::<TrackingSystem>().is_err());
        assert!("HOST_MANAGED".parse::<TrackingSystem>().is_err());
        assert!("host-managed".parse::<TrackingSystem>().is_err());
    }

    #[test]
    fn default_is_targeted() {
        assert_eq!(TrackingSystem::default(), TrackingSystem::Targeted);
    }

    #[test]
    fn serde_snake_case_format() {
        let json = serde_json::to_string(&TrackingSystem::HostManaged).unwrap();
        assert_eq!(json, r#""host_managed""#);

        let json = serde_json::to_string(&TrackingSystem::Targeted).unwrap();
        assert_eq!(json, r#""targeted""#);
    }
}
